use crate::lyrics::{LrcParser, LyricsClient};
use crate::models::Track;
use crate::settings::AppSettings;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct DownloadOptions {
    pub download_dir: String,
    pub audio_format: String,
    pub embed_cover: bool,
    pub save_cover_file: bool,
    pub save_lyrics_lrc: bool,
    pub google_cookie: String,
}

impl DownloadOptions {
    pub fn from_settings(s: &AppSettings) -> Self {
        Self {
            download_dir: s.download_dir.clone(),
            audio_format: s.download_audio_format.clone(),
            embed_cover: s.download_embed_cover,
            save_cover_file: s.download_save_cover_file,
            save_lyrics_lrc: s.download_save_lyrics_lrc,
            google_cookie: s.google_cookie.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct DownloadControl(Arc<AtomicBool>);

impl DownloadControl {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn same_job(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct DownloadTask {
    pub track: Track,
    pub options: DownloadOptions,
    control: DownloadControl,
}

#[derive(Clone)]
pub(crate) enum DownloadState {
    Queued,
    Active {
        attempt: u8,
        percent: f32,
        speed: Option<f64>,
        processing: bool,
    },
}

pub(crate) enum DownloadEvent {
    Progress(String, DownloadControl, DownloadState),
    Finished(Track, DownloadControl, Result<(String, PathBuf), String>),
}

pub struct Downloader {
    tx: Option<mpsc::Sender<DownloadTask>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Downloader {
    pub fn new(ctx: eframe::egui::Context) -> (Self, mpsc::Receiver<DownloadEvent>) {
        let (task_tx, task_rx) = mpsc::channel::<DownloadTask>();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok();
            let lyrics_client = LyricsClient::new();
            let send = |event| {
                let _ = result_tx.send(event);
                ctx.request_repaint();
            };
            while let Ok(task) = task_rx.recv() {
                let mut result = Err("Загрузка не началась".to_owned());
                for attempt in 1..=3 {
                    if task.control.cancelled() {
                        break;
                    }
                    let progress = |percent, speed, processing| {
                        send(DownloadEvent::Progress(
                            task.track.id.clone(),
                            task.control.clone(),
                            DownloadState::Active {
                                attempt,
                                percent,
                                speed,
                                processing,
                            },
                        ));
                    };
                    progress(0.0, None, false);
                    result =
                        Self::run_download(&task, attempt, rt.as_ref(), &lyrics_client, &progress);
                    if result.is_ok() || task.control.cancelled() {
                        break;
                    }
                    if attempt < 3 {
                        for _ in 0..(u16::from(attempt) * 10) {
                            if task.control.cancelled() {
                                break;
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
                send(DownloadEvent::Finished(task.track, task.control, result));
            }
        });

        (
            Self {
                tx: Some(task_tx),
                worker: Some(worker),
            },
            result_rx,
        )
    }

    fn run_download(
        task: &DownloadTask,
        attempt: u8,
        rt: Option<&tokio::runtime::Runtime>,
        lyrics_client: &LyricsClient,
        progress: &impl Fn(f32, Option<f64>, bool),
    ) -> Result<(String, PathBuf), String> {
        let opts = &task.options;

        std::fs::create_dir_all(&opts.download_dir)
            .map_err(|e| format!("Не удалось создать папку загрузок: {e}"))?;

        let mut ffmpeg = Command::new("ffmpeg");
        ffmpeg.arg("-version");
        if !run_bounded(
            &mut ffmpeg,
            &task.control,
            Duration::from_secs(8),
            &|_, _, _| {},
        )
        .is_ok_and(|(status, _)| status.success())
        {
            return Err("Для скачивания и конвертации нужен работающий FFmpeg.".into());
        }

        let safe_title = Self::track_basename(&task.track);
        // '%' в шаблоне yt-dlp это спецсимвол, экранируем
        let template_name = safe_title.replace('%', "%%");
        let output_template = PathBuf::from(&opts.download_dir)
            .join(format!("{template_name}.%(ext)s"))
            .to_string_lossy()
            .to_string();
        let url = format!("https://www.youtube.com/watch?v={}", task.track.id);

        let format = match attempt {
            1 => "ba[protocol=https]/ba/b",
            2 => "ba[protocol^=m3u8]/ba/b",
            _ => "ba/b",
        };
        let mut cmd = Command::new(Self::resolve_ytdl_path());
        cmd.arg("--newline")
            .arg("--progress")
            .arg("--progress-delta")
            .arg("0.4")
            .arg("--progress-template")
            .arg("download:CIBITUBE %(progress._percent_str)s|%(progress.speed)s")
            .arg("-x")
            .arg("--no-playlist")
            .arg("-f")
            .arg(format)
            .arg("--audio-format")
            .arg(&opts.audio_format)
            .arg("--audio-quality")
            .arg("0")
            .arg("--embed-metadata")
            .arg("--no-overwrites")
            .arg("--socket-timeout")
            .arg("15")
            .arg("--retries")
            .arg("0")
            .arg("--extractor-retries")
            .arg("0")
            .arg("--fragment-retries")
            .arg("2")
            .arg("--abort-on-unavailable-fragments")
            .arg("--js-runtimes")
            .arg("node");

        let cookie_file = if attempt == 3 && !opts.google_cookie.is_empty() {
            Some(youtube_cookie_file(&opts.google_cookie).map_err(|e| {
                format!(
                    "Не удалось подготовить авторизацию для yt-dlp: {}",
                    e.kind()
                )
            })?)
        } else {
            None
        };
        if let Some(file) = &cookie_file {
            cmd.arg("--cookies").arg(file.path());
        }

        // The final attempt saves the audio even when artwork post-processing fails.
        let embed_cover = opts.embed_cover && attempt < 3;
        let save_cover = opts.save_cover_file && attempt < 3;
        if embed_cover {
            cmd.arg("--embed-thumbnail");
        }
        if save_cover {
            cmd.arg("--write-thumbnail")
                .arg("--convert-thumbnails")
                .arg("jpg");
        }
        cmd.arg("-o").arg(&output_template).arg("--").arg(&url);

        let (status, errors) =
            run_bounded(&mut cmd, &task.control, Duration::from_secs(240), progress)?;
        if !status.success() {
            let stderr = errors;
            let meaningful_err = stderr
                .lines()
                .filter(|l| {
                    let trim = l.trim();
                    trim.contains("ERROR:")
                        || trim.contains("timed out")
                        || trim.contains("HTTP Error")
                })
                .collect::<Vec<_>>()
                .join("; ");

            let err_summary = if !meaningful_err.is_empty() {
                meaningful_err
            } else {
                stderr
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .rev()
                    .take(2)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" ")
            };

            let message = if err_summary.is_empty() {
                status.to_string()
            } else {
                err_summary
                    .split_whitespace()
                    .map(|word| {
                        if word.contains("https://") || word.contains("http://") {
                            "[URL]"
                        } else {
                            word
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            return Err(format!("yt-dlp: {message}"));
        }

        let audio =
            PathBuf::from(&opts.download_dir).join(format!("{safe_title}.{}", opts.audio_format));
        if !audio
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        {
            return Err("yt-dlp завершился, но итоговый аудиофайл отсутствует.".to_owned());
        }
        progress(100.0, None, true);
        // Текст песни (.lrc), только после успешной загрузки аудио
        let mut lyrics_saved = false;
        if opts.save_lyrics_lrc {
            if let Some(runtime) = rt {
                let duration = LrcParser::parse_timestamp(&task.track.duration_text)
                    .filter(|&millis| millis > 0)
                    .map(|millis| millis / 1000);
                if let Some(lyrics) = runtime.block_on(async {
                    let cancellation = async {
                        while !task.control.cancelled() {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    };
                    tokio::select! {
                        _ = cancellation => None,
                        lyrics = tokio::time::timeout(
                            Duration::from_secs(18),
                            lyrics_client.fetch_with_meta(&task.track.title, &task.track.artist, None, duration),
                        ) => lyrics.ok().flatten(),
                    }
                }) {
                    let lrc_path =
                        PathBuf::from(&opts.download_dir).join(format!("{safe_title}.lrc"));
                    let mut lrc = format!(
                        "[ti:{}]\n[ar:{}]\n[by:Cibitube]\n",
                        task.track.title, task.track.artist
                    );
                    if lyrics.is_synced && !lyrics.lines.is_empty() {
                        if lyrics.offset_ms != 0 {
                            lrc.push_str(&format!("[offset:{}]\n", lyrics.offset_ms));
                        }
                        for line in &lyrics.lines {
                            let minutes = line.start_ms / 60_000;
                            let seconds = line.start_ms / 1_000 % 60;
                            let millis = line.start_ms % 1_000;
                            lrc.push_str(&format!(
                                "[{minutes:02}:{seconds:02}.{millis:03}]{}\n",
                                line.text
                            ));
                        }
                    } else {
                        lrc.push_str(&lyrics.plain);
                    }
                    lyrics_saved = std::fs::write(lrc_path, lrc).is_ok();
                }
            }
        }

        let mut details = vec!["Аудио"];
        if embed_cover || save_cover {
            details.push("Обложка");
        }
        if lyrics_saved {
            details.push("Текст");
        }
        Ok((
            format!("Скачано ({})", details.join(" + ")),
            audio.canonicalize().unwrap_or(audio),
        ))
    }

    /// Безопасное имя файла: без запрещённых символов, без точек/пробелов по краям,
    /// не длиннее 150 символов, без зарезервированных имён Windows.
    pub fn sanitize_filename(raw: &str) -> String {
        const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
        const RESERVED: &[&str] = &[
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];

        let cleaned: String = raw
            .chars()
            .map(|c| {
                if FORBIDDEN.contains(&c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();

        let limited: String = cleaned.trim().chars().take(150).collect();
        let trimmed = limited
            .trim_matches(|c: char| c == ' ' || c == '.')
            .to_string();

        if trimmed.is_empty() {
            return "track".to_string();
        }

        let stem_upper = trimmed.split('.').next().unwrap_or("").to_uppercase();
        if RESERVED.contains(&stem_upper.as_str()) {
            format!("_{trimmed}")
        } else {
            trimmed
        }
    }

    pub(crate) fn download_basename(raw: &str) -> String {
        let mut name = Self::sanitize_filename(raw);
        let mut limit = name.len().min(225);
        while !name.is_char_boundary(limit) {
            limit -= 1;
        }
        name.truncate(limit);
        name.trim_end_matches([' ', '.']).to_owned()
    }

    pub(crate) fn track_basename(track: &Track) -> String {
        let stem = Self::download_basename(&format!("{} - {}", track.artist, track.title));
        let mut limit = stem.len().min(190);
        while !stem.is_char_boundary(limit) {
            limit -= 1;
        }
        let stem = stem[..limit].trim_end_matches([' ', '.']);
        let id = hex::encode(&Sha256::digest(track.id.as_bytes())[..8]);
        format!("{stem} [{id}]")
    }

    fn resolve_ytdl_path() -> PathBuf {
        let exe = if cfg!(target_os = "windows") {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        };
        if let Some(home) = dirs::home_dir() {
            let local_bin = home.join(".local/bin").join(exe);
            if local_bin.exists() {
                return local_bin;
            }
        }
        PathBuf::from(exe)
    }

    pub(crate) fn download_with(
        &self,
        track: Track,
        options: DownloadOptions,
    ) -> Option<DownloadControl> {
        let control = DownloadControl::default();
        self.tx
            .as_ref()?
            .send(DownloadTask {
                track,
                options,
                control: control.clone(),
            })
            .ok()?;
        Some(control)
    }
    pub(crate) fn shutdown(&mut self) {
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_bounded(
    command: &mut Command,
    control: &DownloadControl,
    timeout: Duration,
    progress: &impl Fn(f32, Option<f64>, bool),
) -> Result<(ExitStatus, String), String> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    let capture = || -> std::io::Result<_> {
        let file = tempfile::NamedTempFile::new()?;
        let reader = std::fs::File::open(file.path())?;
        let writer = file.reopen()?;
        Ok((file, reader, writer))
    };
    let (_stdout, mut output, stdout_writer) = capture().map_err(|e| e.to_string())?;
    let (_stderr, mut errors, stderr_writer) = capture().map_err(|e| e.to_string())?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout_writer)
        .stderr(stderr_writer)
        .spawn()
        .map_err(|error| format!("Не удалось запустить загрузчик: {error}"))?;
    let deadline = Instant::now() + timeout;
    let mut pending = Vec::new();
    let status = loop {
        if control.cancelled() || Instant::now() >= deadline {
            stop_process(&mut child);
            return Err(if control.cancelled() {
                "Загрузка отменена".into()
            } else {
                format!("Загрузчик превысил лимит {} сек.", timeout.as_secs())
            });
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                stop_process(&mut child);
                return Err(format!("Ошибка ожидания загрузчика: {error}"));
            }
        };

        if Read::by_ref(&mut output)
            .take(16_384)
            .read_to_end(&mut pending)
            .is_ok()
        {
            if let Some(last_line) = pending.iter().rposition(|byte| *byte == b'\n') {
                for line in String::from_utf8_lossy(&pending[..=last_line]).lines() {
                    if let Some((percent, speed)) = parse_progress(line) {
                        progress(percent, speed, false);
                    }
                }
                pending.drain(..=last_line);
            }
            if pending.len() > 32_768 {
                pending.clear();
            }
        }
        if let Some(status) = status {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let mut tail = Vec::new();
    if let Ok(size) = errors.metadata().map(|meta| meta.len()) {
        let _ = errors.seek(SeekFrom::Start(size.saturating_sub(16_384)));
        let _ = errors.take(16_384).read_to_end(&mut tail);
    }
    Ok((status, String::from_utf8_lossy(&tail).into_owned()))
}

fn stop_process(child: &mut Child) {
    #[cfg(unix)]
    {
        // The process group includes post-processors such as FFmpeg.
        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID"])
            .arg(child.id().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn youtube_cookie_file(raw: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "# Netscape HTTP Cookie File")?;
    for (name, value) in raw
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
    {
        if !name.is_empty()
            && !name.contains(char::is_whitespace)
            && !value.contains(char::is_control)
        {
            writeln!(file, ".youtube.com\tTRUE\t/\tTRUE\t0\t{name}\t{value}")?;
        }
    }
    file.flush()?;
    Ok(file)
}

fn parse_progress(line: &str) -> Option<(f32, Option<f64>)> {
    let (percent, speed) = line.strip_prefix("CIBITUBE ")?.split_once('|')?;
    let percent = percent.trim().strip_suffix('%')?.parse::<f32>().ok()?;
    let speed = speed
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0);
    percent
        .is_finite()
        .then_some((percent.clamp(0.0, 100.0), speed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_yt_dlp_progress_and_bounds_long_unicode_names() {
        assert_eq!(
            parse_progress("CIBITUBE  13.2%|1024.5"),
            Some((13.2, Some(1024.5)))
        );
        assert_eq!(parse_progress("CIBITUBE 100.0%|NA"), Some((100.0, None)));
        assert_eq!(parse_progress("CIBITUBE NaN%|NA"), None);
        assert_eq!(Downloader::download_basename(&"Я".repeat(150)).len(), 224);
    }

    #[cfg(unix)]
    #[test]
    fn hung_child_is_stopped_and_next_command_can_run() {
        let mut hanging = Command::new("sh");
        hanging.args(["-c", "sleep 15 & wait"]);
        let noop = |_, _, _| {};
        let start = Instant::now();
        let timed_out = run_bounded(
            &mut hanging,
            &DownloadControl::default(),
            Duration::from_millis(170),
            &noop,
        );
        assert!(timed_out.unwrap_err().contains("лимит"));
        assert!(start.elapsed() < Duration::from_secs(3));

        let control = DownloadControl::default();
        let signal = control.clone();
        let cancel = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            signal.cancel();
        });
        let aborted = run_bounded(&mut hanging, &control, Duration::from_secs(5), &noop);
        cancel.join().unwrap();
        assert_eq!(aborted.unwrap_err(), "Загрузка отменена");

        let mut next = Command::new("sh");
        next.args(["-c", "printf 'CIBITUBE  25.0%%|4096\n'"]);
        let progress = std::cell::Cell::new(None);
        let (status, _) = run_bounded(
            &mut next,
            &DownloadControl::default(),
            Duration::from_secs(2),
            &|percent, speed, _| progress.set(Some((percent, speed))),
        )
        .unwrap();
        assert!(status.success());
        assert_eq!(progress.get(), Some((25.0, Some(4096.0))));
    }

    #[test]
    fn test_sanitize_filename() {
        // Forbidden characters
        let sanitized = Downloader::sanitize_filename("Song/Name\\With:Forbidden*Chars?\"<>|End");
        assert_eq!(sanitized, "Song_Name_With_Forbidden_Chars_____End");

        // Reserved Windows name CON
        assert_eq!(Downloader::sanitize_filename("CON"), "_CON");
        assert_eq!(Downloader::sanitize_filename("con.mp3"), "_con.mp3");

        // Empty string and whitespace
        assert_eq!(Downloader::sanitize_filename(""), "track");
        assert_eq!(Downloader::sanitize_filename("   ...   "), "track");

        // String longer than 150 characters
        let long_title = "a".repeat(200);
        let res_long = Downloader::sanitize_filename(&long_title);
        assert_eq!(res_long.len(), 150);

        // Leading and trailing dots / spaces
        assert_eq!(
            Downloader::sanitize_filename(" ... My Track Name ... "),
            "My Track Name"
        );
    }
}
