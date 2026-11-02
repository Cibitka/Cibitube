use crate::models::Track;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    pub liked_tracks: Vec<Track>,
    pub history: Vec<Track>,
    pub downloaded_tracks: Vec<Track>,
}

impl Library {
    fn path() -> PathBuf {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka/.local/share"))
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

    pub fn add_downloaded(&mut self, track: Track) {
        if !self.downloaded_tracks.iter().any(|t| t.id == track.id) {
            self.downloaded_tracks.insert(0, track);
            self.save();
        }
    }
}
