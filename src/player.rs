use crate::models::PlaybackInfo;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

type IpcReader = Box<dyn Read + Send>;
type IpcWriter = Box<dyn Write + Send>;

pub struct AudioPlayer {
    process: Child,
    cmd_tx: mpsc::Sender<String>,
    #[allow(dead_code)]
    sock_path: PathBuf,
    pub info: Arc<Mutex<PlaybackInfo>>,
    pub track_finished: Arc<AtomicBool>,
    /// true, если mpv завершился или IPC-канал закрылся
    pub mpv_died: Arc<AtomicBool>,
}

impl AudioPlayer {
    #[cfg(unix)]
    fn secure_socket_path() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let base_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|_| dirs::cache_dir().ok_or(()))
            .unwrap_or_else(|_| std::env::temp_dir());
        let app_dir = base_dir.join("cibitube");
        let _ = std::fs::create_dir_all(&app_dir);
        let _ = std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o700));
        app_dir.join(format!("mpv-{}.sock", std::process::id()))
    }

    #[cfg(windows)]
    fn secure_socket_path() -> PathBuf {
        // Именованный канал Windows: \\.\pipe\cibitube-mpv-<pid>
        PathBuf::from(format!(r"\\.\pipe\cibitube-mpv-{}", std::process::id()))
    }

    fn resolve_ytdl_binary() -> Option<PathBuf> {
        let exe = if cfg!(target_os = "windows") {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        };
        let candidate = dirs::home_dir()?.join(".local/bin").join(exe);
        candidate.exists().then_some(candidate)
    }

    /// Подключение к IPC mpv: до 5 секунд, с проверкой, что mpv не упал при старте.
    fn connect_ipc(
        sock_path: &Path,
        process: &mut Child,
    ) -> Result<(IpcReader, IpcWriter), String> {
        for _ in 0..100 {
            if let Ok(Some(status)) = process.try_wait() {
                return Err(format!("mpv завершился при запуске ({status})"));
            }

            #[cfg(unix)]
            {
                if let Ok(stream) = UnixStream::connect(sock_path) {
                    let reader = stream
                        .try_clone()
                        .map_err(|e| format!("Не удалось клонировать сокет mpv: {e}"))?;
                    return Ok((Box::new(reader), Box::new(stream)));
                }
            }

            #[cfg(windows)]
            {
                // ОДИН хэндл на чтение и запись (mpv шлёт события тому же клиенту, что подписался)
                if let Ok(file) = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(sock_path)
                {
                    let reader = file
                        .try_clone()
                        .map_err(|e| format!("Не удалось клонировать канал mpv: {e}"))?;
                    return Ok((Box::new(reader), Box::new(file)));
                }
            }

            thread::sleep(Duration::from_millis(50));
        }

        let _ = process.kill();
        let _ = process.wait();
        Err("Таймаут подключения к IPC mpv (5 сек).".to_string())
    }

    /// Совместимый конструктор (как раньше, без аргументов).
    pub fn new() -> Result<Self, String> {
        Self::with_volume(80.0)
    }

    /// Конструктор с начальной громкостью из настроек.
    pub fn with_volume(initial_volume: f64) -> Result<Self, String> {
        let volume = initial_volume.clamp(0.0, 100.0);
        let sock_path = Self::secure_socket_path();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&sock_path);

        let mut mpv_cmd = Command::new("mpv");
        mpv_cmd
            .arg("--idle=yes")
            .arg("--no-video")
            .arg(format!("--input-ipc-server={}", sock_path.display()))
            .arg("--ytdl-format=bestaudio/best")
            .arg("--keep-open=yes")
            .arg("--cache=yes")
            .arg("--demuxer-max-bytes=67108864")
            .arg("--demuxer-readahead-secs=120")
            .arg("--demuxer-max-back-bytes=33554432")
            .arg(format!("--volume={:.1}", volume))
            .arg("--ytdl-raw-options=js-runtimes=node");

        if let Some(ytdl) = Self::resolve_ytdl_binary() {
            mpv_cmd.arg(format!(
                "--script-opts=ytdl_hook-ytdl_path={}",
                ytdl.display()
            ));
        }

        // Кроссплатформенный PATH: ~/.local/bin первым
        if let Some(home) = dirs::home_dir() {
            let current_path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths: Vec<PathBuf> = std::env::split_paths(&current_path).collect();
            paths.insert(0, home.join(".local/bin"));
            if let Ok(new_path) = std::env::join_paths(paths) {
                mpv_cmd.env("PATH", new_path);
            }
        }

        let mut process = mpv_cmd.spawn().map_err(|e| {
            format!("Не удалось запустить mpv: {e}. Убедитесь, что пакет mpv установлен.")
        })?;

        let (read_stream, mut write_stream) = Self::connect_ipc(&sock_path, &mut process)?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
        let info = Arc::new(Mutex::new(PlaybackInfo {
            is_playing: false,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume,
        }));
        let track_finished = Arc::new(AtomicBool::new(false));
        let mpv_died = Arc::new(AtomicBool::new(false));

        // Поток записи команд
        thread::spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                let _ = write_stream.write_all(cmd.as_bytes());
                let _ = write_stream.write_all(b"\n");
                let _ = write_stream.flush();
            }
        });

        // Поток чтения событий
        let info_clone = Arc::clone(&info);
        let finished_clone = Arc::clone(&track_finished);
        let died_clone = Arc::clone(&mpv_died);
        thread::spawn(move || {
            let reader = BufReader::new(read_stream);
            for line in reader.lines() {
                let Ok(line_str) = line else { break };
                let Ok(val) = serde_json::from_str::<Value>(&line_str) else {
                    continue;
                };

                if val.get("event").and_then(|e| e.as_str()) == Some("end-file")
                    && val.get("reason").and_then(|r| r.as_str()) == Some("eof")
                {
                    finished_clone.store(true, Ordering::SeqCst);
                }

                if let (Some(name), Some(data)) =
                    (val.get("name").and_then(|n| n.as_str()), val.get("data"))
                {
                    if let Ok(mut lock) = info_clone.lock() {
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
                            "volume" => {
                                if let Some(v) = data.as_f64() {
                                    lock.volume = v;
                                }
                            }
                            "eof-reached" if data.as_bool() == Some(true) => {
                                finished_clone.store(true, Ordering::SeqCst);
                            }
                            _ => {}
                        }
                    }
                }
            }
            // Ошибка чтения ИЛИ обычный EOF = mpv закрыл канал
            died_clone.store(true, Ordering::SeqCst);
        });

        for (id, prop) in [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "eof-reached"),
            (5, "volume"),
        ] {
            let _ = cmd_tx
                .send(serde_json::json!({ "command": ["observe_property", id, prop] }).to_string());
        }
        let _ = cmd_tx
            .send(serde_json::json!({ "command": ["set_property", "volume", volume] }).to_string());

        Ok(Self {
            process,
            cmd_tx,
            sock_path,
            info,
            track_finished,
            mpv_died,
        })
    }

    /// Жив ли процесс mpv (для показа ошибки / перезапуска в UI).
    pub fn is_alive(&self) -> bool {
        !self.mpv_died.load(Ordering::SeqCst)
    }

    pub fn play_url(&self, url: &str) {
        self.track_finished.store(false, Ordering::SeqCst);
        let cmd = serde_json::json!({ "command": ["loadfile", url, "replace"] }).to_string();
        let _ = self.cmd_tx.send(cmd);
        let _ = self
            .cmd_tx
            .send(r#"{"command":["set_property","pause",false]}"#.to_string());
        if let Ok(mut l) = self.info.lock() {
            l.is_playing = true;
            l.position_secs = 0.0;
        }
    }

    pub fn set_speed(&self, speed: f64, preserve_pitch: bool) {
        let clamped = speed.clamp(0.5, 2.0);
        let pitch_cmd = serde_json::json!({
            "command": ["set_property", "audio-pitch-correction", preserve_pitch]
        })
        .to_string();
        let speed_cmd = serde_json::json!({
            "command": ["set_property", "speed", clamped]
        })
        .to_string();
        let _ = self.cmd_tx.send(pitch_cmd);
        let _ = self.cmd_tx.send(speed_cmd);
    }

    pub fn toggle_pause(&self) {
        let is_playing = self.info.lock().map(|l| l.is_playing).unwrap_or(false);
        let cmd = serde_json::json!({
            "command": ["set_property", "pause", is_playing]
        })
        .to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.is_playing = !is_playing;
        }
    }

    pub fn seek(&self, seconds: f64) {
        let cmd = serde_json::json!({
            "command": ["seek", seconds, "absolute"]
        })
        .to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.position_secs = seconds;
        }
    }

    pub fn set_volume(&self, volume: f64) {
        let cmd = serde_json::json!({
            "command": ["set_property", "volume", volume]
        })
        .to_string();
        let _ = self.cmd_tx.send(cmd);
        if let Ok(mut l) = self.info.lock() {
            l.volume = volume;
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.sock_path);
    }
}
