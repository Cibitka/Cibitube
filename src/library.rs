use crate::models::Track;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DownloadFailure {
    pub track: Track,
    pub error: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    pub liked_tracks: Vec<Track>,
    pub history: Vec<Track>,
    pub downloaded_tracks: Vec<Track>,
    #[serde(default)]
    pub(crate) download_failures: Vec<DownloadFailure>,
    #[serde(default)]
    pub local_audio: HashMap<String, PathBuf>,
    #[serde(default)]
    pub local_lyrics: HashMap<String, PathBuf>,
    #[serde(default)]
    pub lyric_track_offsets: HashMap<String, i64>,
}

impl Library {
    fn path() -> PathBuf {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("metrolist");
        let _ = fs::create_dir_all(&dir);
        dir.join("library.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(lib) = serde_json::from_str::<Library>(&data) {
                return lib;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub(crate) fn import_audio(&mut self, path: &Path) -> Result<Track, String> {
        if !crate::local_media::is_audio(path) || !path.is_file() {
            return Err("Выберите существующий аудиофайл.".into());
        }
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let id = format!(
            "local:{}",
            hex::encode(Sha256::digest(path.as_os_str().as_encoded_bytes()))
        );
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let (artist, title) = name.split_once(" - ").unwrap_or(("", &name));
        let track = Track {
            id: id.clone(),
            title: title.to_owned(),
            artist: artist.to_owned(),
            duration_text: String::new(),
            thumbnail_url: None,
        };
        self.local_audio.insert(id, path);
        if !self
            .downloaded_tracks
            .iter()
            .any(|existing| existing.id == track.id)
        {
            self.downloaded_tracks.push(track.clone());
        }
        self.save();
        Ok(track)
    }

    pub fn is_liked(&self, track_id: &str) -> bool {
        self.liked_tracks.iter().any(|t| t.id == track_id)
    }

    pub fn toggle_like(&mut self, track: Track) {
        if let Some(pos) = self.liked_tracks.iter().position(|t| t.id == track.id) {
            self.liked_tracks.remove(pos);
        } else {
            self.liked_tracks.insert(0, track);
        }
        self.save();
    }

    pub fn add_history(&mut self, track: Track) {
        self.history.retain(|t| t.id != track.id);
        self.history.insert(0, track);
        if self.history.len() > 100 {
            self.history.truncate(100);
        }
        self.save();
    }

    pub(crate) fn add_download_failure(&mut self, track: Track, error: String) {
        self.download_failures.insert(
            0,
            DownloadFailure {
                track,
                error,
                occurred_at: chrono::Utc::now().timestamp(),
            },
        );
        self.download_failures.truncate(100);
        self.save();
    }

    pub fn add_downloaded(&mut self, track: Track) {
        if !self.downloaded_tracks.iter().any(|t| t.id == track.id) {
            self.downloaded_tracks.insert(0, track);
            self.save();
        }
    }
}
