use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiLanguage {
    Ru,
    En,
}

#[allow(dead_code)]
impl UiLanguage {
    pub fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("en") {
            Self::En
        } else {
            Self::Ru
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Ru => "ru",
            Self::En => "en",
        }
    }
}

#[allow(dead_code)]
pub struct I18nStrings {
    pub nav_home: &'static str,
    pub nav_search: &'static str,
    pub nav_library: &'static str,
    pub nav_downloads: &'static str,
    pub nav_settings: &'static str,
    pub player_lyrics_tab: &'static str,
    pub player_cover_tab: &'static str,
    pub player_sleep_timer: &'static str,
    pub player_speed: &'static str,
    pub sync_title: &'static str,
    pub sync_push_btn: &'static str,
    pub sync_pull_btn: &'static str,
    pub sync_how_to_connect: &'static str,
    pub sync_guide_title: &'static str,
    pub sync_guide_body: &'static str,
    pub sync_choose_folder_btn: &'static str,
    pub sync_guide_docs_btn: &'static str,
    pub downloads_embed_cover: &'static str,
    pub downloads_save_cover_jpg: &'static str,
    pub downloads_save_lyrics_lrc: &'static str,
    pub updater_check_btn: &'static str,
    pub updater_open_releases: &'static str,
    pub donate_title: &'static str,
    pub donate_subtitle: &'static str,
    pub donate_boosty_btn: &'static str,
    pub donate_copy_ton_btn: &'static str,
    pub settings_language_label: &'static str,
}

pub const STRINGS_RU: I18nStrings = I18nStrings {
    nav_home: "Главная",
    nav_search: "Поиск",
    nav_library: "Медиатека",
    nav_downloads: "Загрузки",
    nav_settings: "Настройки",
    player_lyrics_tab: "Текст песни",
    player_cover_tab: "Обложка трека",
    player_sleep_timer: "Таймер сна",
    player_speed: "Скорость воспроизведения",
    sync_title: "Синхронизация между устройствами (через Google Drive)",
    sync_push_btn: "Сохранить секунду в Google Drive",
    sync_pull_btn: "Продолжить с другого устройства",
    sync_how_to_connect: "Укажите папку, которая синхронизируется с Google Диском (Google Диск для ПК или rclone).",
    sync_guide_title: "Как настроить синхронизацию через Google Диск",
    sync_guide_body: "Cibitube кладёт маленький файл cibitube_sync_state.json в выбранную папку. Если эта папка синхронизируется с Google Диском, другой компьютер с Cibitube подхватит позицию и очередь.\n\n1. Создайте в Google Диске папку Cibitube-Sync.\n2. Windows/macOS: установите «Google Диск для ПК». Linux: установите rclone и fuse3, выполните rclone config (новый remote gdrive, тип drive), затем: rclone mount gdrive:Cibitube-Sync ~/GoogleDrive/Cibitube-Sync --vfs-cache-mode writes --daemon\n3. Нажмите «Выбрать папку синхронизации» и укажите эту папку. Повторите на каждом компьютере.\n4. «Сохранить секунду в Google Drive» записывает позицию, «Продолжить с другого устройства» читает её.\n\nCibitube не запрашивает доступ к вашему Google-аккаунту и не видит пароль: вход выполняют Google Диск для ПК или rclone. Синхронизация с телефоном появится вместе с Android-клиентом.",
    sync_choose_folder_btn: "Выбрать папку синхронизации",
    sync_guide_docs_btn: "Полная инструкция на GitHub",
    downloads_embed_cover: "Вшивать обложку в аудиофайл (--embed-thumbnail)",
    downloads_save_cover_jpg: "Сохранять обложку отдельным файлом (.jpg)",
    downloads_save_lyrics_lrc: "Скачивать текст песни",
    updater_check_btn: "Проверить последний релиз на GitHub",
    updater_open_releases: "Открыть страницу релизов (GitHub Releases)",
    donate_title: "Поддержать автора (Добровольный донат в дар)",
    donate_subtitle: "Безвозмездная поддержка открытого проекта Cibitube (Boosty РФ/Мир и USDT TON)",
    donate_boosty_btn: "Поддержать на Boosty (Карты РФ / СБП / Мир)",
    donate_copy_ton_btn: "Скопировать кошелек USDT (TON)",
    settings_language_label: "Язык интерфейса / Interface Language",
};

pub const STRINGS_EN: I18nStrings = I18nStrings {
    nav_home: "Home",
    nav_search: "Search",
    nav_library: "Library",
    nav_downloads: "Downloads",
    nav_settings: "Settings",
    player_lyrics_tab: "Lyrics",
    player_cover_tab: "Album Artwork",
    player_sleep_timer: "Sleep Timer",
    player_speed: "Playback Speed",
    sync_title: "Cross-device sync (via Google Drive)",
    sync_push_btn: "Save Position to Google Drive",
    sync_pull_btn: "Resume from another device",
    sync_how_to_connect: "Choose a folder that is synced with Google Drive (Google Drive for Desktop or rclone).",
    sync_guide_title: "How to set up Google Drive sync",
    sync_guide_body: "Cibitube writes a small file cibitube_sync_state.json into a folder you choose. If that folder is synced with Google Drive, another computer running Cibitube can pick up your position and queue.\n\n1. Create a folder named Cibitube-Sync in Google Drive.\n2. Windows/macOS: install Google Drive for Desktop. Linux: install rclone and fuse3, run rclone config (new remote gdrive, type drive), then: rclone mount gdrive:Cibitube-Sync ~/GoogleDrive/Cibitube-Sync --vfs-cache-mode writes --daemon\n3. Press \"Choose sync folder\" and select that folder. Repeat on every computer.\n4. \"Save Position to Google Drive\" writes your position, \"Resume from another device\" reads it.\n\nCibitube does not ask for access to your Google account and never sees your password (signing in is done by Google Drive for Desktop or rclone). Phone sync will arrive with the Android client.",
    sync_choose_folder_btn: "Choose sync folder",
    sync_guide_docs_btn: "Full guide on GitHub",
    downloads_embed_cover: "Embed cover art into audio file (--embed-thumbnail)",
    downloads_save_cover_jpg: "Save cover art as standalone image (.jpg)",
    downloads_save_lyrics_lrc: "Download lyrics file",
    updater_check_btn: "Check Latest Release on GitHub",
    updater_open_releases: "Open GitHub Releases Page",
    donate_title: "Support the Author (Voluntary Gift / Donation)",
    donate_subtitle: "Support open-source development of Cibitube via Boosty or USDT (TON)",
    donate_boosty_btn: "Donate on Boosty (Cards Worldwide & RU)",
    donate_copy_ton_btn: "Copy USDT (TON) Wallet Address",
    settings_language_label: "Interface Language / Язык интерфейса",
};

pub fn tr(lang_code: &str) -> &'static I18nStrings {
    match UiLanguage::from_code(lang_code) {
        UiLanguage::En => &STRINGS_EN,
        UiLanguage::Ru => &STRINGS_RU,
    }
}
