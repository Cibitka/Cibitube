use crate::lyrics::{LrcParser, LyricsClient};
use crate::models::Track;
use crate::settings::AppSettings;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

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

pub struct DownloadTask {
    pub track: Track,
    pub options: DownloadOptions,
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
    Progress(String, DownloadState),
    Finished(Track, Result<String, String>),
}

pub struct Downloader {
    tx: mpsc::Sender<DownloadTask>,
}

impl Downloader {
    pub fn new(ctx: eframe::egui::Context) -> (Self, mpsc::Receiver<DownloadEvent>) {
        let (task_tx, task_rx) = mpsc::channel::<DownloadTask>();
        let (result_tx, result_rx) = mpsc::channel();
        thread::spawn(move || {
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
                    let progress = |percent, speed, processing| {
                        send(DownloadEvent::Progress(
                            task.track.id.clone(),
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
                    if result.is_ok() {
                        break;
                    }
                    if attempt < 3 {
                        thread::sleep(std::time::Duration::from_secs(u64::from(attempt)));
                    }
                }
                send(DownloadEvent::Finished(task.track, result));
            }
        });

        (Self { tx: task_tx }, result_rx)
    }

    fn run_download(
        task: &DownloadTask,
        attempt: u8,
        rt: Option<&tokio::runtime::Runtime>,
        lyrics_client: &LyricsClient,
        progress: &impl Fn(f32, Option<f64>, bool),
    ) -> Result<String, String> {
        let opts = &task.options;

        std::fs::create_dir_all(&opts.download_dir)
            .map_err(|e| format!("Не удалось создать папку загрузок: {e}"))?;

        if !Self::ffmpeg_available() {
            return Err(
                "Для конвертации аудио и обложек нужен ffmpeg. Установите его \
                 (Arch: sudo pacman -S ffmpeg, Debian/Ubuntu: sudo apt install ffmpeg)."
                    .to_string(),
            );
        }

        let raw_title = format!("{} - {}", task.track.artist, task.track.title);
        let safe_title = Self::download_basename(&raw_title);
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

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Ошибка запуска yt-dlp: {e}"))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (status, errors) = thread::scope(|scope| {
            let errors = scope.spawn(move || {
                let mut recent = std::collections::VecDeque::new();
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if recent.len() == 16 {
                        recent.pop_front();
                    }
                    recent.push_back(line);
                }
                recent.into_iter().collect::<Vec<_>>().join("\n")
            });
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((percent, speed)) = parse_progress(&line) {
                    progress(percent, speed, false);
                }
            }
            (child.wait(), errors.join().unwrap_or_default())
        });
        let status = status.map_err(|e| format!("Ошибка ожидания yt-dlp: {e}"))?;
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
                    tokio::time::timeout(
                        std::time::Duration::from_secs(18),
                        lyrics_client.fetch_with_meta(
                            &task.track.title,
                            &task.track.artist,
                            None,
                            duration,
                        ),
                    )
                    .await
                    .ok()
                    .flatten()
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
        Ok(format!(
            "Скачано ({}) в {}",
            details.join(" + "),
            opts.download_dir
        ))
    }

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
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

    /// Совместимый вызов (как раньше): параметры берутся из текущих настроек.
    #[allow(dead_code)]
    pub fn download(&self, track: Track) {
        let options = DownloadOptions::from_settings(&AppSettings::load());
        self.download_with(track, options);
    }

    /// Явные параметры загрузки.
    pub fn download_with(&self, track: Track, options: DownloadOptions) {
        let _ = self.tx.send(DownloadTask { track, options });
    }
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
