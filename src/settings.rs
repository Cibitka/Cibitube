use crate::models::AccountProfile;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub account_name: Option<String>,
    pub google_cookie: String,
    pub auth_user_index: usize,
    pub available_accounts: Vec<AccountProfile>,

    // Язык интерфейса / UI Language ("ru" | "en")
    pub ui_language: String,

    // 1. Автосозданные плейлисты
    pub show_liked_playlist: bool,
    pub show_downloaded_playlist: bool,
    pub show_top_playlist: bool,
    pub show_cached_playlist: bool,
    pub show_uploaded_playlist: bool,

    // 2. Тема и оформление
    pub theme_mode: usize, // 0 = AMOLED Black, 1 = Dark Slate, 2 = Light
    pub accent_color_idx: usize,
    pub custom_accent_color: [u8; 3],
    pub use_custom_accent: bool,
    pub high_refresh_rate: bool,
    pub landscape_scaling: bool,
    pub dynamic_icon_colors: bool,
    pub density_scale: f32,

    // 3. Мини-плеер
    pub new_mini_player_design: bool,
    pub mini_player_bg_style: usize,
    pub mini_player_pure_black: bool,
    pub swipe_track_switch: bool,
    pub swipe_sensitivity: f32,

    // 4. Плеер и полноэкранный вид
    pub new_player_design: bool,
    pub player_bg_style: usize,
    pub hide_player_thumbnail: bool,
    pub crop_album_art: bool,
    pub player_buttons_color_style: usize,
    pub slider_style: usize,
    pub show_remaining_time: bool,
    pub album_art_rounding: f32,

    // 5. Текст песни
    pub lyrics_position: usize,
    pub lyrics_respect_artist_position: bool,
    pub lyrics_change_on_tap: bool,
    pub lyrics_auto_scroll: bool,
    pub experimental_lyrics: bool,

    // 6. Навигация и экран
    pub hide_status_bar_fullscreen: bool,
    pub default_nav_tab: usize,
    pub slim_nav_bar: bool,
    pub grid_cell_size: usize,

    // 7. Аудио и качество
    pub audio_quality: String,
    pub audio_normalization: bool,
    pub skip_silence: bool,
    pub sponsorblock_enabled: bool,

    // 8. Контент и язык
    pub content_country: String,
    pub content_language: String,

    // 9. Discord Integration
    pub discord_rpc_enabled: bool,
    pub discord_show_buttons: bool,
    pub discord_activity_type: usize,

    // 10. Загрузки
    pub download_dir: String,
    pub download_embed_cover: bool,
    pub download_save_cover_file: bool,
    pub download_save_lyrics_lrc: bool,
    pub download_audio_format: String,

    // 11. Синхронизация ПК <-> Android (Google Drive)
    pub cross_device_sync_enabled: bool,
    pub google_drive_sync_folder: String,

    // 12. Воспроизведение и таймер
    pub playback_speed: f64,
    pub preserve_pitch: bool,
    pub sleep_timer_minutes: u32,
    pub autoplay_similar_radio: bool,

    // 13. Громкость
    pub volume: f64,
}

impl Default for AppSettings {
    fn default() -> Self {
        let music_dir = dirs::audio_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Cibitube")
            .to_string_lossy()
            .to_string();

        let default_sync_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cibitube")
            .join("Cibitube-Sync")
            .to_string_lossy()
            .to_string();

        let sys_lang = std::env::var("LANG").unwrap_or_default().to_lowercase();
        let (ui_lang, def_country, def_lang) = if sys_lang.starts_with("ru") {
            ("ru".to_string(), "RU".to_string(), "ru".to_string())
        } else {
            ("en".to_string(), "US".to_string(), "en".to_string())
        };

        Self {
            account_name: None,
            google_cookie: String::new(),
            auth_user_index: 0,
            available_accounts: Vec::new(),

            ui_language: ui_lang,

            show_liked_playlist: true,
            show_downloaded_playlist: true,
            show_top_playlist: true,
            show_cached_playlist: false,
            show_uploaded_playlist: true,

            theme_mode: 0,
            accent_color_idx: 0,
            custom_accent_color: [29, 185, 84],
            use_custom_accent: false,
            high_refresh_rate: true,
            landscape_scaling: false,
            dynamic_icon_colors: true,
            density_scale: 1.0,

            new_mini_player_design: true,
            mini_player_bg_style: 0,
            mini_player_pure_black: false,
            swipe_track_switch: true,
            swipe_sensitivity: 1.0,

            new_player_design: true,
            player_bg_style: 0,
            hide_player_thumbnail: false,
            crop_album_art: false,
            player_buttons_color_style: 0,
            slider_style: 0,
            show_remaining_time: false,
            album_art_rounding: 8.0,

            lyrics_position: 0,
            lyrics_respect_artist_position: true,
            lyrics_change_on_tap: true,
            lyrics_auto_scroll: true,
            experimental_lyrics: false,

            hide_status_bar_fullscreen: true,
            default_nav_tab: 0,
            slim_nav_bar: false,
            grid_cell_size: 1,

            audio_quality: "High (320 kbps)".to_string(),
            audio_normalization: true,
            skip_silence: false,
            sponsorblock_enabled: true,

            content_country: def_country,
            content_language: def_lang,

            discord_rpc_enabled: false,
            discord_show_buttons: true,
            discord_activity_type: 0,

            download_dir: music_dir,
            download_embed_cover: true,
            download_save_cover_file: true,
            download_save_lyrics_lrc: true,
            download_audio_format: "mp3".to_string(),

            cross_device_sync_enabled: true,
            google_drive_sync_folder: default_sync_dir,

            playback_speed: 1.0,
            preserve_pitch: true,
            sleep_timer_minutes: 0,
            autoplay_similar_radio: true,

            volume: 80.0,
        }
    }
}

impl AppSettings {
    pub fn config_path() -> PathBuf {
        let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
        let new_dir = base.join("cibitube");
        let old_dir = base.join("metrolist");

        // Авто-миграция настроек из старой папки metrolist в cibitube
        if !new_dir.exists() && old_dir.exists() {
            let _ = fs::rename(&old_dir, &new_dir);
        }
        let _ = fs::create_dir_all(&new_dir);
        new_dir.join("settings.json")
    }

    pub fn load_from(path: &Path) -> Self {
        if let Ok(data) = fs::read_to_string(path) {
            match serde_json::from_str::<AppSettings>(&data) {
                Ok(settings) => return settings,
                Err(err) => {
                    eprintln!(
                        "Ошибка чтения settings.json: {}. Сохраняем резервную копию.",
                        err
                    );
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let bak_path = path.with_extension(format!("json.bak-{}", now));
                    let _ = fs::rename(path, &bak_path);
                }
            }
        }
        let default = Self::default();
        default.save_to(path);
        default
    }

    pub fn load() -> Self {
        Self::load_from(&Self::config_path())
    }

    pub fn save_to(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp_path = path.with_extension("tmp");

            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = OpenOptions::new();
                options.write(true).create(true).truncate(true).mode(0o600);
                if let Ok(mut f) = options.open(&tmp_path) {
                    if f.write_all(json.as_bytes()).is_ok() && f.flush().is_ok() {
                        let _ = fs::rename(&tmp_path, path);
                        return;
                    }
                }
            }

            #[cfg(not(unix))]
            {
                if fs::write(&tmp_path, &json).is_ok() {
                    let _ = fs::rename(&tmp_path, path);
                }
            }
        }
    }

    pub fn save(&self) {
        self.save_to(&Self::config_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_corrupted_settings_creates_backup() {
        let temp_dir =
            std::env::temp_dir().join(format!("cibitube_test_cfg_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let settings_path = temp_dir.join("settings.json");

        // Write corrupt/invalid JSON
        fs::write(&settings_path, "{ corrupt json data ...").unwrap();

        // Loading should fallback to defaults and create a .bak-* file
        let loaded = AppSettings::load_from(&settings_path);
        assert_eq!(loaded.volume, 80.0);

        let mut found_bak = false;
        if let Ok(entries) = fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("settings.json.bak-") {
                    found_bak = true;
                    let bak_content = fs::read_to_string(entry.path()).unwrap();
                    assert_eq!(bak_content, "{ corrupt json data ...");
                }
            }
        }
        assert!(found_bak, "Backup file settings.json.bak-* must be created");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
