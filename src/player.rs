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
use std::time::{Duration, Instant};

type IpcReader = Box<dyn Read + Send>;
type IpcWriter = Box<dyn Write + Send>;

struct PreparedStream {
    source: String,
    requested_at: Instant,
    direct_url: Option<String>,
}

pub struct AudioPlayer {
    gain: Mutex<(f64, f64)>,
    audio_pulse: Arc<Mutex<(f32, Instant)>>,
    prepared_stream: Arc<Mutex<Option<PreparedStream>>>,
    preparing_stream: Arc<AtomicBool>,
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
        let audio_pulse = Arc::new(Mutex::new((0.0_f32, Instant::now())));
        let pulse = Arc::clone(&audio_pulse);
        thread::spawn(move || {
            let mut average = None::<f32>;
            let reader = BufReader::new(read_stream);
            for line in reader.lines() {
                let Ok(line_str) = line else { break };
                let Ok(val) = serde_json::from_str::<Value>(&line_str) else {
                    continue;
                };

                if val.get("request_id").and_then(Value::as_i64) == Some(900) {
                    let level = val
                        .pointer("/data/lavfi.astats.Overall.RMS_level")
                        .and_then(Value::as_str)
                        .and_then(|db| db.parse::<f32>().ok())
                        .map(|db| 10.0_f32.powf(db / 20.0))
                        .filter(|level| level.is_finite())
                        .unwrap_or(0.0);
                    let baseline = average.unwrap_or(level);
                    average = Some(baseline * 0.97 + level * 0.03);
                    let hit = (5.5 * (level - baseline) / (baseline + 0.008)).clamp(0.0, 1.0);
                    if let Ok(mut pulse) = pulse.lock() {
                        let decay = (-pulse.1.elapsed().as_secs_f32() * 5.0).exp();
                        pulse.0 = hit.max(pulse.0 * decay);
                        pulse.1 = Instant::now();
                    }
                    continue;
                }
                if val.get("event").and_then(Value::as_str) == Some("start-file") {
                    average = None;
                }

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
        ] {
            let _ = cmd_tx
                .send(serde_json::json!({ "command": ["observe_property", id, prop] }).to_string());
        }
        let _ = cmd_tx
            .send(serde_json::json!({ "command": ["set_property", "volume", volume] }).to_string());

        Ok(Self {
            gain: Mutex::new((volume, 1.0)),
            audio_pulse,
            prepared_stream: Arc::new(Mutex::new(None)),
            preparing_stream: Arc::new(AtomicBool::new(false)),
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

    pub fn prepare_stream(&self, url: String) {
        let Ok(mut slot) = self.prepared_stream.lock() else {
            return;
        };
        if slot.as_ref().is_some_and(|entry| {
            entry.source == url && entry.requested_at.elapsed() < Duration::from_secs(120)
        }) || self.preparing_stream.swap(true, Ordering::SeqCst)
        {
            return;
        }
        *slot = Some(PreparedStream {
            source: url.clone(),
            requested_at: Instant::now(),
            direct_url: None,
        });
        let cache = Arc::clone(&self.prepared_stream);
        let busy = Arc::clone(&self.preparing_stream);
        tokio::task::spawn_blocking(move || {
            let executable = Self::resolve_ytdl_binary().unwrap_or_else(|| PathBuf::from("yt-dlp"));
            let direct_url = Command::new(executable)
                .args([
                    "--no-playlist",
                    "--no-warnings",
                    "--socket-timeout",
                    "8",
                    "--extractor-retries",
                    "0",
                    "--js-runtimes",
                    "node",
                    "-f",
                    "bestaudio[protocol=https]",
                    "--get-url",
                    "--",
                ])
                .arg(&url)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|output| output.trim().to_owned())
                .filter(|output| {
                    output.starts_with("https://") && !output.contains(char::is_whitespace)
                });
            if let Ok(mut slot) = cache.lock() {
                if let Some(entry) = slot.as_mut().filter(|entry| entry.source == url) {
                    entry.direct_url = direct_url;
                }
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn set_audio_processing(&self, normalize: bool, skip_silence: bool, ambient: bool) {
        let mut filters = Vec::new();
        if skip_silence {
            filters.push("lavfi=[silenceremove=stop_periods=-1:stop_duration=0.8:stop_threshold=-55dB:stop_silence=0.2:timestamp=copy]");
        }
        if normalize {
            filters.push("lavfi=[dynaudnorm=f=150:g=9:m=3]");
        }
        if ambient {
            filters.push("@ambient:lavfi=[astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=RMS_level]");
        }
        let cmd = serde_json::json!({"command": ["set_property", "af", filters.join(",")]});
        let _ = self.cmd_tx.send(cmd.to_string());
    }

    pub fn play_url(&self, url: &str, start_gain: f64) {
        self.track_finished.store(false, Ordering::SeqCst);
        self.set_fade_gain(start_gain);
        let prepared = self.prepared_stream.lock().ok().and_then(|mut slot| {
            if slot.as_ref().is_some_and(|entry| entry.source == url) {
                slot.take()
                    .filter(|entry| entry.requested_at.elapsed() < Duration::from_secs(120))
                    .and_then(|entry| entry.direct_url)
            } else {
                None
            }
        });
        let cmd = serde_json::json!({
            "command": ["loadfile", prepared.as_deref().unwrap_or(url), "replace"]
        })
        .to_string();
        let _ = self.cmd_tx.send(cmd);
        let _ = self
            .cmd_tx
            .send(r#"{"command":["set_property","pause",false]}"#.to_string());
        if let Ok(mut l) = self.info.lock() {
            l.is_playing = true;
            l.position_secs = 0.0;
            l.duration_secs = 0.0;
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

    pub fn poll_audio_pulse(&self) -> f32 {
        let _ = self.cmd_tx.send(
            r#"{"command":["get_property","af-metadata/ambient"],"request_id":900}"#.to_string(),
        );
        self.audio_pulse
            .lock()
            .map(|pulse| pulse.0 * (-pulse.1.elapsed().as_secs_f32() * 5.0).exp())
            .unwrap_or_default()
    }

    fn send_volume(&self, base: f64, gain: f64) {
        let cmd = serde_json::json!({"command": ["set_property", "volume", base * gain]});
        let _ = self.cmd_tx.send(cmd.to_string());
    }

    pub fn set_fade_gain(&self, fade: f64) {
        if let Ok(mut gain) = self.gain.lock() {
            let fade = fade.clamp(0.0, 1.0);
            if (gain.1 - fade).abs() >= 0.002 || (fade == 1.0 && gain.1 != 1.0) {
                gain.1 = fade;
                self.send_volume(gain.0, gain.1);
            }
        }
    }

    pub fn set_volume(&self, volume: f64) {
        if let Ok(mut gain) = self.gain.lock() {
            gain.0 = volume.clamp(0.0, 100.0);
            self.send_volume(gain.0, gain.1);
        }
        if let Ok(mut info) = self.info.lock() {
            info.volume = volume;
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
