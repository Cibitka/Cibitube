use crate::models::Track;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTrackEntry {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub duration_secs: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossDeviceSyncState {
    #[serde(alias = "track_id")]
    pub track_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(alias = "position_secs")]
    pub position_secs: f64,
    #[serde(alias = "duration_secs")]
    pub duration_secs: f64,
    #[serde(alias = "is_playing")]
    pub is_playing: bool,
    #[serde(alias = "playback_speed", default = "default_speed")]
    pub playback_speed: f64,
    #[serde(default)]
    pub sleep_timer_mins: Option<u32>,
    #[serde(default)]
    pub queue: Vec<QueueTrackEntry>,
    #[serde(alias = "queue_index", default)]
    pub queue_index: usize,
    #[serde(alias = "repeat_mode", default = "default_repeat_mode")]
    pub repeat_mode: String,
    #[serde(alias = "shuffle_enabled", default)]
    pub shuffle_enabled: bool,
    #[serde(alias = "device_name", default = "default_device_name")]
    pub source_device: String,
    #[serde(alias = "updated_at")]
    pub updated_at: String,
    #[serde(alias = "updated_at_unix", default)]
    pub updated_at_unix: u64,
}

fn default_speed() -> f64 {
    1.0
}
fn default_repeat_mode() -> String {
    "all".to_string()
}
fn default_device_name() -> String {
    "Cibitube Client".to_string()
}

pub fn parse_duration_to_secs(text: &str) -> f64 {
    let parts: Vec<&str> = text.split(':').collect();
    match parts.len() {
        2 => {
            let m: f64 = parts[0].trim().parse().unwrap_or(0.0);
            let s: f64 = parts[1].trim().parse().unwrap_or(0.0);
            m * 60.0 + s
        }
        3 => {
            let h: f64 = parts[0].trim().parse().unwrap_or(0.0);
            let m: f64 = parts[1].trim().parse().unwrap_or(0.0);
            let s: f64 = parts[2].trim().parse().unwrap_or(0.0);
            h * 3600.0 + m * 60.0 + s
        }
        _ => 0.0,
    }
}

impl QueueTrackEntry {
    pub fn to_track(&self) -> Track {
        let mins = (self.duration_secs / 60.0).floor() as u32;
        let secs = (self.duration_secs % 60.0).round() as u32;
        Track {
            id: self.id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            duration_text: format!("{}:{:02}", mins, secs),
            thumbnail_url: self.thumbnail.clone(),
        }
    }
}

impl CrossDeviceSyncState {
    pub fn to_track(&self) -> Track {
        let mins = (self.duration_secs / 60.0).floor() as u32;
        let secs = (self.duration_secs % 60.0).round() as u32;
        Track {
            id: self.track_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            duration_text: format!("{}:{:02}", mins, secs),
            thumbnail_url: self.thumbnail.clone(),
        }
    }
    pub fn get_timestamp_unix(&self) -> u64 {
        if self.updated_at_unix > 0 {
            return self.updated_at_unix;
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(&self.updated_at) {
            return dt.timestamp().max(0) as u64;
        }
        0
    }

    /// Вычисляет точную секунду с учетом времени, прошедшего с момента записи
    pub fn effective_position_secs(&self) -> f64 {
        if !self.is_playing {
            return self.position_secs.clamp(0.0, self.duration_secs.max(1.0));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ts = self.get_timestamp_unix();
        let elapsed = now.saturating_sub(ts) as f64;
        if elapsed <= 600.0 {
            (self.position_secs + elapsed * self.playback_speed)
                .clamp(0.0, self.duration_secs.max(1.0))
        } else {
            self.position_secs.clamp(0.0, self.duration_secs.max(1.0))
        }
    }
}

pub struct SyncManager;

#[allow(dead_code)]
impl SyncManager {
    pub const SYNC_FILE_NAME: &'static str = "cibitube_sync_state.json";
    pub const SYNC_FOLDER_NAME: &'static str = "Cibitube-Sync";

    pub fn local_sync_path(custom_folder: &str) -> PathBuf {
        let dir = if !custom_folder.trim().is_empty() {
            PathBuf::from(custom_folder)
        } else {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("cibitube")
        };
        let _ = fs::create_dir_all(&dir);
        dir.join(Self::SYNC_FILE_NAME)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_state_with_queue(
        custom_folder: &str,
        track: &Track,
        position_secs: f64,
        duration_secs: f64,
        is_playing: bool,
        playback_speed: f64,
        queue_tracks: &[Track],
        queue_index: usize,
        repeat_mode: &str,
        shuffle_enabled: bool,
    ) -> Result<CrossDeviceSyncState, String> {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now_iso = Utc::now().to_rfc3339();

        // Критично (1.6): Берем реальную индивидуальную длительность каждого трека в очереди, а не текущего!
        let queue = queue_tracks
            .iter()
            .map(|t| QueueTrackEntry {
                id: t.id.clone(),
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: None,
                thumbnail: t.thumbnail_url.clone(),
                duration_secs: parse_duration_to_secs(&t.duration_text),
            })
            .collect();

        let state = CrossDeviceSyncState {
            track_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: None,
            thumbnail: track.thumbnail_url.clone(),
            position_secs,
            duration_secs,
            is_playing,
            playback_speed,
            sleep_timer_mins: None,
            queue,
            queue_index,
            repeat_mode: repeat_mode.to_string(),
            shuffle_enabled,
            source_device: format!("Cibitube Desktop ({})", std::env::consts::OS),
            updated_at: now_iso,
            updated_at_unix: now_unix,
        };

        let path = Self::local_sync_path(custom_folder);
        let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())?;
        Ok(state)
    }

    pub fn load_state(custom_folder: &str) -> Option<CrossDeviceSyncState> {
        let path = Self::local_sync_path(custom_folder);
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str::<CrossDeviceSyncState>(&data).ok()
    }

    /// Поиск существующего файла cibitube_sync_state.json на Google Диске (1.6)
    async fn find_drive_file_id(client: &reqwest::Client, token: &str) -> Option<String> {
        let q = format!("name = '{}' and trashed = false", Self::SYNC_FILE_NAME);
        let res = client
            .get("https://www.googleapis.com/drive/v3/files")
            .bearer_auth(token)
            .query(&[("q", &q), ("fields", &"files(id,name)".to_string())])
            .send()
            .await
            .ok()?;
        let val: serde_json::Value = res.json().await.ok()?;
        val.get("files")?
            .as_array()?
            .first()?
            .get("id")?
            .as_str()
            .map(|s| s.to_string())
    }

    /// Надежный push в Google Drive: если файл существует — делаем PATCH, иначе создаем один файл (1.6)
    pub async fn push_to_google_drive(
        oauth_bearer_token: &str,
        state: &CrossDeviceSyncState,
    ) -> Result<(), String> {
        if oauth_bearer_token.trim().is_empty() {
            return Ok(());
        }
        let client = reqwest::Client::new();
        let body = serde_json::to_string(state).map_err(|e| e.to_string())?;

        let existing_id = Self::find_drive_file_id(&client, oauth_bearer_token).await;

        let res = if let Some(file_id) = existing_id {
            // PATCH существующего файла
            client
                .patch(format!("https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media", file_id))
                .bearer_auth(oauth_bearer_token)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
        } else {
            // Создание с метаданными имени (multipart)
            let boundary = "cibitube_sync_boundary";
            let metadata = serde_json::json!({
                "name": Self::SYNC_FILE_NAME,
                "mimeType": "application/json"
            });
            let multipart_body = format!(
                "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{b}\r\nContent-Type: application/json\r\n\r\n{body}\r\n--{b}--\r\n",
                b = boundary,
                meta = metadata,
                body = body
            );
            client
                .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart")
                .bearer_auth(oauth_bearer_token)
                .header("Content-Type", format!("multipart/related; boundary={}", boundary))
                .body(multipart_body)
                .send()
                .await
        }.map_err(|e| e.to_string())?;

        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(
                "AUTH_EXPIRED: Токен Google Drive истек. Пожалуйста, войдите снова.".to_string(),
            );
        }

        if res.status().is_success() {
            Ok(())
        } else {
            Err(format!("Google Drive Sync HTTP {}", res.status()))
        }
    }

    /// Чтение состояния (pull) напрямую с Google Drive (1.6)
    pub async fn pull_from_google_drive(
        oauth_bearer_token: &str,
    ) -> Result<Option<CrossDeviceSyncState>, String> {
        if oauth_bearer_token.trim().is_empty() {
            return Ok(None);
        }
        let client = reqwest::Client::new();
        let file_id = match Self::find_drive_file_id(&client, oauth_bearer_token).await {
            Some(id) => id,
            None => return Ok(None),
        };

        let res = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}?alt=media",
                file_id
            ))
            .bearer_auth(oauth_bearer_token)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("AUTH_EXPIRED: Токен Google Drive истек.".to_string());
        }

        if res.status().is_success() {
            let state = res
                .json::<CrossDeviceSyncState>()
                .await
                .map_err(|e| e.to_string())?;
            Ok(Some(state))
        } else {
            Err(format!("Google Drive Pull HTTP {}", res.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_state_camel_and_snake_case() {
        let camel = r#"{
            "trackId": "xyz123",
            "title": "Camel Track",
            "artist": "Camel Artist",
            "positionSecs": 42.0,
            "durationSecs": 180.0,
            "isPlaying": true,
            "playbackSpeed": 1.25,
            "updatedAt": "2026-11-09T12:00:00Z"
        }"#;
        let s1: CrossDeviceSyncState =
            serde_json::from_str(camel).expect("failed to parse camelCase");
        assert_eq!(s1.track_id, "xyz123");
        assert_eq!(s1.position_secs, 42.0);
        assert_eq!(s1.playback_speed, 1.25);

        let snake = r#"{
            "track_id": "abc789",
            "title": "Snake Track",
            "artist": "Snake Artist",
            "position_secs": 15.0,
            "duration_secs": 200.0,
            "is_playing": false,
            "playback_speed": 1.0,
            "updated_at": "2026-11-09T12:00:00Z"
        }"#;
        let s2: CrossDeviceSyncState =
            serde_json::from_str(snake).expect("failed to parse snake_case");
        assert_eq!(s2.track_id, "abc789");
        assert_eq!(s2.position_secs, 15.0);
        assert!(!s2.is_playing);
    }

    #[test]
    fn test_effective_position_secs() {
        let state = CrossDeviceSyncState {
            track_id: "t1".to_string(),
            title: "T".to_string(),
            artist: "A".to_string(),
            album: None,
            thumbnail: None,
            position_secs: 30.0,
            duration_secs: 100.0,
            is_playing: false,
            playback_speed: 1.0,
            sleep_timer_mins: None,
            queue: vec![],
            queue_index: 0,
            repeat_mode: "all".to_string(),
            shuffle_enabled: false,
            source_device: "test".to_string(),
            updated_at: "2026-11-09T12:00:00Z".to_string(),
            updated_at_unix: 0,
        };
        assert_eq!(state.effective_position_secs(), 30.0);
    }
}
