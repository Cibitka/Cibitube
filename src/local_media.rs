use std::path::{Path, PathBuf};

pub(crate) fn pick_file(title: &str, filter: &str) -> Result<Option<PathBuf>, String> {
    let result = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--title",
            title,
            "--file-filter",
            filter,
        ])
        .output()
        .map_err(|_| {
            "Диалог выбора файла недоступен. В Linux установите Zenity или перетащите файл в окно."
                .to_owned()
        })?;
    if !result.status.success() {
        return if result.status.code() == Some(1) {
            Ok(None)
        } else {
            Err("Не удалось открыть диалог выбора файла.".into())
        };
    }
    let path = String::from_utf8(result.stdout).map_err(|_| "Путь к файлу не читается.")?;
    Ok(Some(PathBuf::from(path.trim_end_matches(['\r', '\n']))))
}

pub(crate) fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(crate) fn is_audio(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" | "aiff" | "webm"
    )
}

pub(crate) fn is_lyrics(path: &Path) -> bool {
    matches!(extension(path).as_str(), "lrc" | "ttml" | "txt" | "qrc")
}
