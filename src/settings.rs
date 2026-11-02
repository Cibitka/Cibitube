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

    // 1. Автосозданные плейлисты (photo_1)
    #[serde(default = "default_true")]
    pub show_liked_playlist: bool,      // «Понравившиеся»
    #[serde(default = "default_true")]
    pub show_downloaded_playlist: bool, // «Скачанные»
    #[serde(default = "default_true")]
    pub show_top_playlist: bool,        // «Топ»
    #[serde(default)]
    pub show_cached_playlist: bool,     // «Кэшированные»
    #[serde(default = "default_true")]
    pub show_uploaded_playlist: bool,   // «Загруженные»

    // 2. Тема и оформление (photo_3)
    pub theme_mode: usize, // 0 = AMOLED Black, 1 = Dark Slate, 2 = Light
    pub accent_color_idx: usize, // 0 = Emerald (#1DB954), 1 = Blue (#3B82F6), 2 = Purple (#8B5CF6), 3 = Crimson (#E11D48), 4 = Orange (#F97316)
    #[serde(default = "default_custom_color")]
    pub custom_accent_color: [u8; 3], // [R, G, B]
    #[serde(default)]
    pub use_custom_accent: bool,
    #[serde(default = "default_true")]
    pub high_refresh_rate: bool, // 120 Гц
    #[serde(default)]
    pub landscape_scaling: bool,
    #[serde(default = "default_true")]
    pub dynamic_icon_colors: bool,
    #[serde(default = "default_density")]
    pub density_scale: f32,

    // 3. Мини-плеер (photo_4, photo_6 & user screenshot)
    #[serde(default = "default_true")]
    pub new_mini_player_design: bool, // «Новый дизайн мини-плеера»
    #[serde(default)]
    pub mini_player_bg_style: usize, // 0 = Градиент, 1 = Чистый черный, 2 = Сплошной
    #[serde(default)]
    pub mini_player_pure_black: bool,
    #[serde(default = "default_true")]
    pub swipe_track_switch: bool,
    #[serde(default = "default_one_f32")]
    pub swipe_sensitivity: f32,

    // 4. Плеер и полноэкранный вид (photo_4, photo_5, photo_6, photo_2)
    #[serde(default = "default_true")]
    pub new_player_design: bool, // «Новый дизайн плеера»
    #[serde(default)]
    pub player_bg_style: usize, // 0 = Градиент, 1 = Чистый черный, 2 = Сплошной
    #[serde(default)]
    pub hide_player_thumbnail: bool, // «Скрыть миниатюру плеера»
    #[serde(default)]
    pub crop_album_art: bool, // «Обрезать обложку альбома»
    #[serde(default)]
    pub player_buttons_color_style: usize, // 0 = Акцентный, 1 = Монохромный, 2 = Адаптивный
    #[serde(default)]
    pub slider_style: usize, // 0 = Прямой, 1 = Волнистый, 2 = Тонкий
    #[serde(default)]
    pub show_remaining_time: bool,
    #[serde(default = "default_rounding")]
    pub album_art_rounding: f32,

    // 5. Текст песни (photo_7, photo_8)
    #[serde(default)]
    pub lyrics_position: usize, // 0 = По центру, 1 = По левому краю
    #[serde(default = "default_true")]
    pub lyrics_respect_artist_position: bool,
    #[serde(default = "default_true")]
    pub lyrics_change_on_tap: bool,
    #[serde(default = "default_true")]
    pub lyrics_auto_scroll: bool,
    #[serde(default)]
    pub experimental_lyrics: bool,

    // 6. Навигация и экран (photo_8, photo_9, photo_10)
    #[serde(default = "default_true")]
    pub hide_status_bar_fullscreen: bool,
    #[serde(default)]
    pub default_nav_tab: usize, // 0 = Главная, 1 = Плейлисты, 2 = Любимые треки, 3 = Поиск
    #[serde(default)]
    pub slim_nav_bar: bool,
    #[serde(default = "default_one_usize")]
    pub grid_cell_size: usize, // 0 = Маленький, 1 = Средний, 2 = Большой

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
    pub discord_activity_type: usize, // 0 = Listening, 1 = Playing

    // 10. Загрузки
    pub download_dir: String,
    #[serde(default = "default_volume")]
    pub volume: f64,
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

            show_liked_playlist: true,
            show_downloaded_playlist: true,
            show_top_playlist: true,
            show_cached_playlist: false,
            show_uploaded_playlist: true,

            theme_mode: 0, // Default AMOLED Black like Metrolist
            accent_color_idx: 0, // Metrolist Emerald Accent
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

            content_country: "RU".to_string(),
            content_language: "ru".to_string(),

            discord_rpc_enabled: true,
            discord_show_buttons: true,
            discord_activity_type: 0, // Listening

            download_dir: music_dir,
            volume: 80.0,
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

fn default_one_usize() -> usize {
    1
}

fn default_one_f32() -> f32 {
    1.0
}

fn default_custom_color() -> [u8; 3] {
    [29, 185, 84]
}

fn default_volume() -> f64 {
    80.0
}
