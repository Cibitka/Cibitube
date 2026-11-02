use crate::models::AccountProfile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub account_name: Option<String>,
    pub google_cookie: String,
    #[serde(default)]
    pub auth_user_index: usize,
    #[serde(default)]
    pub available_accounts: Vec<AccountProfile>,

    // Appearance & Layout Customization (from Metrolist)
    pub theme_mode: usize, // 0 = Pure Black (AMOLED), 1 = Dark Slate, 2 = Light
    pub accent_color_idx: usize, // 0 = Emerald (#1DB954), 1 = Blue (#3B82F6), 2 = Purple (#8B5CF6), 3 = Crimson (#E11D48), 4 = Orange (#F97316)
    #[serde(default = "default_density")]
    pub density_scale: f32, // 0.85 = Compact, 1.0 = Normal, 1.15 = Spacious
    #[serde(default)]
    pub show_remaining_time: bool,
    #[serde(default = "default_rounding")]
    pub album_art_rounding: f32,
    #[serde(default = "default_true")]
    pub lyrics_auto_scroll: bool,
    #[serde(default)]
    pub mini_player_pure_black: bool,

    // Player & Audio
    pub audio_quality: String,
    pub audio_normalization: bool,
    pub skip_silence: bool,
    pub sponsorblock_enabled: bool,

    // Content & Language
    pub content_country: String,
    pub content_language: String,

    // Discord Integration
    pub discord_rpc_enabled: bool,
    pub discord_show_buttons: bool,
    pub discord_activity_type: usize, // 0 = Listening, 1 = Playing

    // Storage & Downloads
    pub download_dir: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let music_dir = dirs::audio_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
            .unwrap_or_else(|| PathBuf::from("/home/cibitka/Music"))
            .join("Metrolist")
            .to_string_lossy()
            .to_string();

        Self {
            account_name: None,
            google_cookie: String::new(),
            auth_user_index: 0,
            available_accounts: Vec::new(),

            theme_mode: 0, // Default AMOLED Black like Metrolist
            accent_color_idx: 0, // Metrolist Emerald Accent
            density_scale: 1.0,
            show_remaining_time: false,
            album_art_rounding: 8.0,
            lyrics_auto_scroll: true,
            mini_player_pure_black: false,

            audio_quality: "High (320 kbps)".to_string(),
            audio_normalization: true,
            skip_silence: false,
            sponsorblock_enabled: true,

            content_country: "RU".to_string(),
            content_language: "ru".to_string(),

            discord_rpc_enabled: true,
            discord_show_buttons: true,
            discord_activity_type: 0, // Listening

            download_dir: music_dir,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka/.local/share"))
            .join("metrolist");
        let _ = fs::create_dir_all(&dir);
        dir.join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&data) {
                return settings;
            }
        }
        let default = Self::default();
        default.save();
        default
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

fn default_density() -> f32 {
    1.0
}

fn default_rounding() -> f32 {
    8.0
}

fn default_true() -> bool {
    true
}
