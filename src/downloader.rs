use crate::models::Track;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

pub struct Downloader {
    tx: mpsc::Sender<DownloadTask>,
}

struct DownloadTask {
    track: Track,
    download_dir: String,
    result_tx: mpsc::Sender<(Track, Result<String, String>)>,
}

impl Downloader {
    pub fn new() -> (Self, mpsc::Receiver<(Track, Result<String, String>)>) {
        let (task_tx, task_rx) = mpsc::channel::<DownloadTask>();
        let (_result_tx, result_rx) = mpsc::channel::<(Track, Result<String, String>)>();

        thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                let ytdl_path = dirs::home_dir()
                    .map(|h| h.join(".local/bin/yt-dlp"))
                    .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin/yt-dlp"));

                let output_template = format!("{}/%(title)s.%(ext)s", task.download_dir);
                let url = format!("https://www.youtube.com/watch?v={}", task.track.id);

                let status = Command::new(&ytdl_path)
                    .arg("-x")
                    .arg("--audio-format")
                    .arg("mp3")
                    .arg("--audio-quality")
                    .arg("0")
                    .arg("--embed-thumbnail")
                    .arg("--add-metadata")
                    .arg("--js-runtimes")
                    .arg("node")
                    .arg("-o")
                    .arg(&output_template)
                    .arg(&url)
                    .env("PATH", format!("{}:{}", "/home/cibitka/.local/bin", std::env::var("PATH").unwrap_or_default()))
                    .status();

                let res = match status {
                    Ok(s) if s.success() => Ok(format!("Скачано в {}", task.download_dir)),
                    Ok(s) => Err(format!("yt-dlp завершился с ошибкой: {}", s)),
                    Err(e) => Err(format!("Ошибка запуска загрузчика: {}", e)),
                };

                let _ = task.result_tx.send((task.track, res));
            }
        });

        (Self { tx: task_tx }, result_rx)
    }

    pub fn download(&self, track: Track, download_dir: &str, result_tx: mpsc::Sender<(Track, Result<String, String>)>) {
        let _ = self.tx.send(DownloadTask {
            track,
            download_dir: download_dir.to_string(),
            result_tx,
        });
    }
}
