use std::path::{Path, PathBuf};

pub(crate) fn pick_file(title: &str, filter: &str) -> Result<Option<PathBuf>, String> {
    let mut command = file_dialog(title, filter);
    let result = command.output().map_err(|_| {
        "Диалог выбора файла недоступен. На Linux можно установить Zenity или перетащить файл в окно.".to_owned()
    })?;
    if !result.status.success() {
        return if result.status.code() == Some(1) {
            Ok(None)
        } else {
            Err("Не удалось открыть диалог выбора файла.".into())
        };
    }
    let path = String::from_utf8(result.stdout).map_err(|_| "Путь к файлу не читается.")?;
    let path = path.trim_end_matches(['\r', '\n']);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[cfg(target_os = "windows")]
fn file_dialog(title: &str, filter: &str) -> std::process::Command {
    let mut command = std::process::Command::new("powershell.exe");
    let filter =
        filter
            .split_once('|')
            .map_or("Все файлы|*.*".to_owned(), |(title, patterns)| {
                format!(
                    "{}|{}",
                    title.trim(),
                    patterns.split_whitespace().collect::<Vec<_>>().join(";")
                )
            });
    command
        .env("CIBITUBE_DIALOG_TITLE", title)
        .env("CIBITUBE_DIALOG_FILTER", filter)
        .args([
            "-NoProfile",
            "-STA",
            "-Command",
            r#"
            $ErrorActionPreference = 'Stop'
            [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
            Add-Type -AssemblyName System.Windows.Forms
            $dialog = New-Object System.Windows.Forms.OpenFileDialog
            $dialog.Title = $env:CIBITUBE_DIALOG_TITLE
            $dialog.Filter = $env:CIBITUBE_DIALOG_FILTER
            if ($dialog.ShowDialog() -eq 'OK') { [Console]::WriteLine($dialog.FileName) }
        "#,
        ]);
    command
}

#[cfg(target_os = "macos")]
fn file_dialog(title: &str, _filter: &str) -> std::process::Command {
    let mut command = std::process::Command::new("osascript");
    command.args([
        "-e",
        r#"
        on run argv
            try
                return POSIX path of (choose file with prompt (item 1 of argv))
            on error message number errorNumber
                if errorNumber is -128 then return ""
                error message number errorNumber
            end try
        end run
    "#,
        "--",
        title,
    ]);
    command
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn file_dialog(title: &str, filter: &str) -> std::process::Command {
    let mut command = std::process::Command::new("zenity");
    command.args([
        "--file-selection",
        "--title",
        title,
        "--file-filter",
        filter,
    ]);
    command
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

pub(crate) fn validate_audio(path: &Path) -> Result<PathBuf, String> {
    if !is_audio(path) || !path.is_file() {
        return Err("Выберите существующий аудиофайл.".into());
    }
    let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    let input = path
        .to_str()
        .ok_or("Путь к файлу содержит неподдерживаемые символы.")?;
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
            "-i",
            input,
        ])
        .output()
        .map_err(|_| "Для проверки аудиофайла нужен ffprobe из комплекта FFmpeg.")?;
    if !output.status.success() || output.stdout.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("В файле не найдено читаемой аудиодорожки.".into());
    }
    Ok(path)
}

pub(crate) fn open_folder(path: &str) -> Result<(), String> {
    let application = if cfg!(target_os = "windows") {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(application)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Не удалось открыть папку: {error}"))
}

pub(crate) fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let dir = path.parent().ok_or("У файла нет папки")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut temporary, value).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
