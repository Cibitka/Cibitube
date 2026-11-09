use crate::lyrics::LyricsClient;
use crate::models::Track;
use crate::settings::AppSettings;
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
}

impl DownloadOptions {
    pub fn from_settings(s: &AppSettings) -> Self {
        Self {
            download_dir: s.download_dir.clone(),
            audio_format: s.download_audio_format.clone(),
            embed_cover: s.download_embed_cover,
            save_cover_file: s.download_save_cover_file,
            save_lyrics_lrc: s.download_save_lyrics_lrc,
        }
    }
}

pub struct DownloadTask {
    pub track: Track,
    pub options: DownloadOptions,
}

pub struct Downloader {
    tx: mpsc::Sender<DownloadTask>,
}

impl Downloader {
    pub fn new() -> (Self, mpsc::Receiver<(Track, Result<String, String>)>) {
        let (task_tx, task_rx) = mpsc::channel::<DownloadTask>();
        let (result_tx, result_rx) = mpsc::channel::<(Track, Result<String, String>)>();

        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok();
            let lyrics_client = LyricsClient::new();

            // Последовательная очередь: один трек за раз
            while let Ok(task) = task_rx.recv() {
                let res = Self::run_download(&task, rt.as_ref(), &lyrics_client);
                let _ = result_tx.send((task.track, res));
            }
        });

        (Self { tx: task_tx }, result_rx)
    }

    fn run_download(
        task: &DownloadTask,
        rt: Option<&tokio::runtime::Runtime>,
        lyrics_client: &LyricsClient,
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

        let safe_title =
            Self::sanitize_filename(&format!("{} - {}", task.track.artist, task.track.title));
        // '%' в шаблоне yt-dlp это спецсимвол, экранируем
        let template_name = safe_title.replace('%', "%%");
        let output_template = PathBuf::from(&opts.download_dir)
            .join(format!("{template_name}.%(ext)s"))
            .to_string_lossy()
            .to_string();
        let url = format!("https://www.youtube.com/watch?v={}", task.track.id);

        let mut cmd = Command::new(Self::resolve_ytdl_path());
        cmd.arg("-x")
            .arg("--no-playlist")
            .arg("--audio-format")
            .arg(&opts.audio_format)
            .arg("--audio-quality")
            .arg("0")
            .arg("--embed-metadata")
            .arg("--no-overwrites")
            .arg("--js-runtimes")
            .arg("node");

        if opts.embed_cover {
            cmd.arg("--embed-thumbnail");
        }
        if opts.save_cover_file {
            cmd.arg("--write-thumbnail")
                .arg("--convert-thumbnails")
                .arg("jpg");
        }
        cmd.arg("-o").arg(&output_template).arg(&url);

        let status = cmd
            .status()
            .map_err(|e| format!("Ошибка запуска yt-dlp: {e}"))?;
        if !status.success() {
            return Err(format!("yt-dlp завершился с кодом: {status}"));
        }

        // Текст песни (.lrc), только после успешной загрузки аудио
        let mut lyrics_saved = false;
        if opts.save_lyrics_lrc {
            if let Some(runtime) = rt {
                if let Some(lyrics) =
                    runtime.block_on(lyrics_client.fetch(&task.track.title, &task.track.artist))
                {
                    let lrc_path =
                        PathBuf::from(&opts.download_dir).join(format!("{safe_title}.lrc"));
                    let mut lrc = format!(
                        "[ti:{}]\n[ar:{}]\n[by:Cibitube]\n",
                        task.track.title, task.track.artist
                    );
                    if lyrics.is_synced && !lyrics.lines.is_empty() {
                        for line in &lyrics.lines {
                            let mins = (line.seconds / 60.0).floor() as u32;
                            let secs = line.seconds % 60.0;
                            lrc.push_str(&format!("[{:02}:{:05.2}]{}\n", mins, secs, line.text));
                        }
                    } else {
                        lrc.push_str(&lyrics.plain);
                    }
                    lyrics_saved = std::fs::write(lrc_path, lrc).is_ok();
                }
            }
        }

        let mut details = vec!["Аудио"];
        if opts.embed_cover || opts.save_cover_file {
            details.push("Обложка");
        }
        if lyrics_saved {
            details.push("Текст .LRC");
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

#[cfg(test)]
mod tests {
    use super::*;

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
