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
    pub(crate) pending_downloads: Vec<Track>,
    #[serde(default)]
    legacy_merged: bool,
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
    fn data_root() -> PathBuf {
        dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    fn path() -> PathBuf {
        Self::data_root().join("cibitube").join("library.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::data_root())
    }

    fn load_from(root: &Path) -> Self {
        let path = root.join("cibitube").join("library.json");
        let read = |path: &Path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
        };
        let primary = read(&path);
        if primary
            .as_ref()
            .is_some_and(|library| library.legacy_merged)
        {
            return primary.unwrap();
        }

        let legacy = read(&root.join("metrolist").join("library.json"));
        // An existing current library takes precedence over legacy data for the same track.
        let mut library = primary.unwrap_or_default();
        if let Some(legacy) = legacy {
            let merge_tracks = |current: &mut Vec<Track>, old: Vec<Track>| {
                let mut ids: std::collections::HashSet<_> =
                    current.iter().map(|track| track.id.clone()).collect();
                current.extend(old.into_iter().filter(|track| ids.insert(track.id.clone())));
            };
            merge_tracks(&mut library.liked_tracks, legacy.liked_tracks);
            merge_tracks(&mut library.history, legacy.history);
            merge_tracks(&mut library.downloaded_tracks, legacy.downloaded_tracks);
            merge_tracks(&mut library.pending_downloads, legacy.pending_downloads);
            library.history.truncate(100);
            library.download_failures.extend(legacy.download_failures);
            library
                .download_failures
                .sort_by_key(|failure| std::cmp::Reverse(failure.occurred_at));
            library.download_failures.truncate(100);
            for (id, path) in legacy.local_audio {
                library.local_audio.entry(id).or_insert(path);
            }
            for (id, path) in legacy.local_lyrics {
                library.local_lyrics.entry(id).or_insert(path);
            }
            for (id, offset) in legacy.lyric_track_offsets {
                library.lyric_track_offsets.entry(id).or_insert(offset);
            }
            library.legacy_merged = true;
            // Keep the source file available for recovery if the new file cannot be written.
            let _ = crate::local_media::write_json(&path, &library);
        }
        library
    }

    pub(crate) fn persist(&self) -> Result<(), String> {
        crate::local_media::write_json(&Self::path(), self)
    }

    pub fn save(&self) {
        let _ = self.persist();
    }

    pub(crate) fn import_audio(&mut self, path: PathBuf) -> Result<Track, String> {
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
        self.link_audio(&track, path)?;
        Ok(track)
    }

    pub(crate) fn link_audio(&mut self, track: &Track, path: PathBuf) -> Result<(), String> {
        let previous = self.local_audio.insert(track.id.clone(), path);
        let inserted = !self
            .downloaded_tracks
            .iter()
            .any(|existing| existing.id == track.id);
        if inserted {
            self.downloaded_tracks.push(track.clone());
        }
        if let Err(error) = self.persist() {
            if let Some(path) = previous {
                self.local_audio.insert(track.id.clone(), path);
            } else {
                self.local_audio.remove(&track.id);
            }
            if inserted {
                self.downloaded_tracks.pop();
            }
            return Err(format!("Не удалось сохранить медиатеку: {error}"));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_old_and_new_libraries_once_and_keeps_pending_downloads() {
        let root = tempfile::tempdir().unwrap();
        let make_track = |id: &str| Track {
            id: id.into(),
            title: id.into(),
            artist: "Fixture".into(),
            duration_text: String::new(),
            thumbnail_url: None,
        };
        let old_path = root.path().join("metrolist/library.json");
        let new_path = root.path().join("cibitube/library.json");
        let old = Library {
            liked_tracks: vec![make_track("old"), make_track("shared")],
            lyric_track_offsets: HashMap::from([("shared".into(), 350)]),
            ..Library::default()
        };
        let current = Library {
            liked_tracks: vec![make_track("shared"), make_track("new")],
            pending_downloads: vec![make_track("pending")],
            lyric_track_offsets: HashMap::from([("shared".into(), 700)]),
            ..Library::default()
        };
        crate::local_media::write_json(&old_path, &old).unwrap();
        crate::local_media::write_json(&new_path, &current).unwrap();
        let mut loaded = Library::load_from(root.path());
        assert_eq!(
            loaded
                .liked_tracks
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            ["shared", "new", "old"]
        );
        assert_eq!(loaded.lyric_track_offsets["shared"], 700);
        assert_eq!(loaded.pending_downloads[0].id, "pending");
        assert!(old_path.is_file());

        loaded.liked_tracks.retain(|track| track.id != "old");
        crate::local_media::write_json(&new_path, &loaded).unwrap();
        let loaded = Library::load_from(root.path());
        assert_eq!(loaded.liked_tracks.len(), 2);
        assert_eq!(loaded.pending_downloads[0].id, "pending");
    }
}
