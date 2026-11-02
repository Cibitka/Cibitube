use std::path::PathBuf;
use std::process::Command;

pub struct BrowserAuth;

impl BrowserAuth {
    /// Attempts to extract YouTube cookies automatically from the user's Firefox profile.
    pub fn extract_firefox_cookies() -> Result<String, String> {
        let home = dirs::home_dir().ok_or_else(|| "Домашняя директория не найдена".to_string())?;

        let search_dirs = [
            home.join(".config/mozilla/firefox"),
            home.join(".mozilla/firefox"),
        ];

        let mut cookie_db: Option<PathBuf> = None;

        for dir in &search_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.contains("default") {
                        let candidate = path.join("cookies.sqlite");
                        if candidate.exists() {
                            cookie_db = Some(candidate);
                            break;
                        }
                    }
                }
            }
            if cookie_db.is_some() {
                break;
            }
        }

        let db_path = cookie_db.ok_or_else(|| {
            "Профиль Firefox с файлом cookies.sqlite не найден. Убедитесь, что Firefox установлен.".to_string()
        })?;

        // Use sqlite3 CLI with URI filename immutable=1 so it reads even if Firefox is currently running!
        let uri = format!("file:{}?immutable=1", db_path.display());
        let query = "SELECT host, name, value FROM moz_cookies WHERE host LIKE '%youtube.com%' OR host LIKE '%google.com%' ORDER BY CASE WHEN host LIKE '%youtube.com%' THEN 2 ELSE 1 END ASC;";

        let output = Command::new("sqlite3")
            .arg(&uri)
            .arg("-separator")
            .arg("|||")
            .arg(query)
            .output()
            .map_err(|e| format!("Ошибка вызова sqlite3: {}. Убедитесь, что пакет sqlite3 установлен.", e))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Не удалось прочитать куки из Firefox: {}", err));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut cookie_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split("|||").collect();
            if parts.len() >= 3 {
                let name = parts[1].trim();
                let value = parts[2].trim();
                if !name.is_empty() {
                    cookie_map.insert(name.to_string(), value.to_string());
                }
            }
        }

        if cookie_map.is_empty() {
            return Err("В профиле Firefox не найдено куки для YouTube / Google. Войдите в Google / YouTube в Firefox и попробуйте снова.".to_string());
        }

        let cookie_string: String = cookie_map
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");

        if !cookie_string.contains("SAPISID") && !cookie_string.contains("LOGIN_INFO") {
            return Err("В Firefox нет активной сессии YouTube / Google. Войдите в аккаунт на https://music.youtube.com в браузере Firefox.".to_string());
        }

        Ok(cookie_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_firefox_cookies() {
        let res = BrowserAuth::extract_firefox_cookies();
        assert!(res.is_ok(), "Expected cookies extraction to succeed: {:?}", res.err());
        let cookies = res.unwrap();
        assert!(cookies.contains("SAPISID") || cookies.contains("LOGIN_INFO"));
    }
}
