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
    nav_search: "Главная",
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
    nav_search: "Home",
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
    settings_language_label: "Interface language",
};

pub fn tr(lang_code: &str) -> &'static I18nStrings {
    match UiLanguage::from_code(lang_code) {
        UiLanguage::En => &STRINGS_EN,
        UiLanguage::Ru => &STRINGS_RU,
    }
}

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};

// Cibitube has one UI locale. Workers use it for messages and native dialogs too.
static ENGLISH: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_language(code: &str) {
    ENGLISH.store(
        UiLanguage::from_code(code) == UiLanguage::En,
        Ordering::Relaxed,
    );
}

pub(crate) fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

fn english_strings() -> &'static HashMap<String, String> {
    static STRINGS: OnceLock<HashMap<String, String>> = OnceLock::new();
    STRINGS.get_or_init(|| {
        serde_json::from_str(include_str!("translations/en.json"))
            .expect("English translations must be valid JSON")
    })
}

pub(crate) fn text(source: &str) -> &str {
    if is_english() {
        english_strings().get(source).map_or(source, String::as_str)
    } else {
        source
    }
}

#[macro_export]
macro_rules! localized_format {
    ($ru:literal, $en:literal $(, $($args:tt)*)?) => {
        if $crate::i18n::is_english() {
            format!($en $(, $($args)*)?)
        } else {
            format!($ru $(, $($args)*)?)
        }
    };
}

#[derive(Debug)]
enum MessagePart {
    Text(String),
    Value(String),
}

fn message_parts(template: &str) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    let mut remaining = template;
    let mut index = 0;
    while let Some(start) = remaining.find('{') {
        let Some(end) = remaining[start..].find('}') else {
            break;
        };
        let end = start + end;
        parts.push(MessagePart::Text(remaining[..start].to_owned()));
        let key = remaining[start + 1..end].split(':').next().unwrap_or("");
        parts.push(MessagePart::Value(if key.is_empty() {
            let key = index.to_string();
            index += 1;
            key
        } else {
            key.to_owned()
        }));
        remaining = &remaining[end + 1..];
    }
    parts.push(MessagePart::Text(remaining.to_owned()));
    parts
}

fn translate_message_parts(
    from: &[MessagePart],
    to: &[MessagePart],
    source: &str,
) -> Option<String> {
    let mut remaining = source;
    let mut values = HashMap::new();
    for (index, part) in from.iter().enumerate() {
        match part {
            MessagePart::Text(text) => remaining = remaining.strip_prefix(text)?,
            MessagePart::Value(key) => {
                let end = match from.get(index + 1) {
                    Some(MessagePart::Text(text)) if !text.is_empty() => remaining.find(text)?,
                    _ => remaining.len(),
                };
                values.insert(key.as_str(), &remaining[..end]);
                remaining = &remaining[end..];
            }
        }
    }
    if !remaining.is_empty() {
        return None;
    }
    let mut output = String::new();
    for part in to {
        match part {
            MessagePart::Text(text) => output.push_str(text),
            MessagePart::Value(key) => output.push_str(values.get(key.as_str())?),
        }
    }
    Some(output)
}

fn message_in(source: &str, english: bool) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let dictionary = english_strings();
    if english {
        if let Some(value) = dictionary.get(source) {
            return Cow::Borrowed(value);
        }
    } else if let Some((russian, _)) = dictionary
        .iter()
        .find(|(_, value)| value.as_str() == source)
    {
        return Cow::Borrowed(russian);
    }
    type MessageTemplates = Vec<(Vec<MessagePart>, Vec<MessagePart>)>;
    static TEMPLATES: OnceLock<MessageTemplates> = OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| {
        let mut entries: Vec<_> = dictionary
            .iter()
            .filter(|(key, _)| key.contains('{'))
            .collect();
        entries.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));
        entries
            .into_iter()
            .map(|(ru, en)| (message_parts(ru), message_parts(en)))
            .collect()
    });
    for (ru, en) in templates {
        let (from, to) = if english { (ru, en) } else { (en, ru) };
        if let Some(translated) = translate_message_parts(from, to, source) {
            return Cow::Owned(translated);
        }
    }
    Cow::Borrowed(source)
}

// Only application messages go through this function. Track titles and lyrics stay untouched.
pub(crate) fn message(source: &str) -> std::borrow::Cow<'_, str> {
    message_in(source, is_english())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_catalog_has_no_russian_text_and_preserves_format_arguments() {
        for (russian, english) in english_strings() {
            assert!(
                !english
                    .chars()
                    .any(|ch| ('А'..='я').contains(&ch) || ch == 'ё' || ch == 'Ё'),
                "{russian}"
            );
            let keys = |template: &str| {
                let mut keys: Vec<_> = message_parts(template)
                    .into_iter()
                    .filter_map(|part| {
                        if let MessagePart::Value(key) = part {
                            Some(key)
                        } else {
                            None
                        }
                    })
                    .collect();
                keys.sort();
                keys
            };
            assert_eq!(keys(russian), keys(english), "{russian}");
        }
    }

    #[test]
    fn existing_messages_change_language_without_translating_song_names() {
        assert_eq!(message_in("Добавлен трек «Пауза»", true), "Added “Пауза”");
        assert_eq!(message_in("Added “Пауза”", false), "Добавлен трек «Пауза»");
        assert_eq!(message_in("Локальный файл (LRC)", true), "Local file (LRC)");
        assert_eq!(
            message_in("Artist — Название песни", true),
            "Artist — Название песни"
        );
    }
}
