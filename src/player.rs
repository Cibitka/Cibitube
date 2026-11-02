use crate::models::PlaybackInfo;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct AudioPlayer {
    _process: Child,
    cmd_tx: mpsc::Sender<String>,
    pub info: Arc<Mutex<PlaybackInfo>>,
    pub track_finished: Arc<AtomicBool>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let sock_path = format!("/tmp/metrolist-mpv-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&sock_path);

        let process = Command::new("mpv")
            .arg("--idle=yes")
            .arg("--no-video")
            .arg(format!("--input-ipc-server={}", sock_path))
            .arg("--script-opts=ytdl_hook-ytdl_path=/home/cibitka/.local/bin/yt-dlp")
            .arg("--ytdl-format=bestaudio/best")
            .arg("--keep-open=yes")
            .env("PATH", format!("{}:{}", "/home/cibitka/.local/bin", std::env::var("PATH").unwrap_or_default()))
            .spawn()
            .map_err(|e| format!("Failed to start mpv: {}. Make sure mpv is installed.", e))?;

        // Wait for socket to appear (fast poll 15ms)
        let mut stream = None;
        for _ in 0..40 {
            if let Ok(s) = UnixStream::connect(&sock_path) {
                stream = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }

        let stream = stream.ok_or_else(|| "Failed to connect to MPV IPC socket".to_string())?;
        let read_stream = stream.try_clone().map_err(|e| e.to_string())?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
        let info = Arc::new(Mutex::new(PlaybackInfo {
            is_playing: false,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume: 80.0,
        }));
        let track_finished = Arc::new(AtomicBool::new(false));

        // Command writer thread
        let mut write_stream = stream;
        thread::spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                let _ = write_stream.write_all(cmd.as_bytes());
                let _ = write_stream.write_all(b"\n");
                let _ = write_stream.flush();
            }
        });

        // Event reader thread
        let info_clone = Arc::clone(&info);
        let finished_clone = Arc::clone(&track_finished);
        thread::spawn(move || {
            let reader = BufReader::new(read_stream);
            for line in reader.lines().flatten() {
                if let Ok(val) = serde_json::from_str::<Value>(&line) {
                    if let Some(event) = val.get("event").and_then(|e| e.as_str()) {
                        if event == "end-file" {
                            let reason = val.get("reason").and_then(|r| r.as_str()).unwrap_or("");
                            if reason == "eof" {
                                finished_clone.store(true, Ordering::SeqCst);
                            }
                        }
                    }

                    if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                        if let Some(data) = val.get("data") {
                            let mut lock = info_clone.lock().unwrap();
                            match name {
                                "time-pos" => {
                                    if let Some(secs) = data.as_f64() {
                                        lock.position_secs = secs;
                                    }
                                }
                                "duration" => {
                                    if let Some(d) = data.as_f64() {
                                        lock.duration_secs = d;
                                    }
                                }
                                "pause" => {
                                    if let Some(p) = data.as_bool() {
                                        lock.is_playing = !p;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        // Subscribe to properties
        let _ = cmd_tx.send(r#"{"command":["observe_property",1,"time-pos"]}"#.to_string());
        let _ = cmd_tx.send(r#"{"command":["observe_property",2,"duration"]}"#.to_string());
        let _ = cmd_tx.send(r#"{"command":["observe_property",3,"pause"]}"#.to_string());
        let _ = cmd_tx.send(r#"{"command":["set_property","volume",80]}"#.to_string());

        Ok(Self {
            _process: process,
            cmd_tx,
            info,
            track_finished,
        })
    }

    pub fn play_url(&self, url: &str) {
        self.track_finished.store(false, Ordering::SeqCst);
        let cmd = serde_json::json!({
            "command": ["loadfile", url, "replace"]
        }).to_string();
        let _ = self.cmd_tx.send(cmd);
        let _ = self.cmd_tx.send(r#"{"command":["set_property","pause",false]}"#.to_string());
        if let Ok(mut l) = self.info.lock() {
            l.is_playing = true;
            l.position_secs = 0.0;
        }
    }

    pub fn toggle_pause(&self) {
        let is_playing = self.info.lock().map(|l| l.is_playing).unwrap_or(false);
        let new_state = is_playing;
        let cmd = serde_json::json!({
            "command": ["set_property", "pause", new_state]
        }).to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.is_playing = !new_state;
        }
    }

    pub fn seek(&self, seconds: f64) {
        let cmd = serde_json::json!({
            "command": ["seek", seconds, "absolute"]
        }).to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.position_secs = seconds;
        }
    }

    pub fn set_volume(&self, volume: f64) {
        let cmd = serde_json::json!({
            "command": ["set_property", "volume", volume]
        }).to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.volume = volume;
        }
    }
}
