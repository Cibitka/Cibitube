use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub html_url: Option<String>,
    pub name: Option<String>,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct UpdateInfo {
    pub version: String,
    pub release_page_url: String,
    /// Ссылка на бинарник для автообновления (только Linux; на других ОС None)
    pub download_url: Option<String>,
    pub apk_download_url: Option<String>,
    pub notes: String,
    pub expected_sha256: Option<String>,
}

pub struct Updater;

impl Updater {
    pub const REPO: &'static str = "Cibitka/Cibitube";
    pub const RELEASES_URL: &'static str = "https://github.com/Cibitka/Cibitube/releases";
    pub const CURRENT_VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn http_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .user_agent("Cibitube-Updater")
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| e.to_string())
    }

    pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
        let client = Self::http_client(8)?;
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            Self::REPO
        );
        let resp = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(
                "Лимит запросов к GitHub API исчерпан. Откройте страницу релизов в браузере."
                    .to_string(),
            );
        }
        if !status.is_success() {
            return Err(format!("GitHub API вернул статус: {status}"));
        }

        let release = resp
            .json::<GitHubRelease>()
            .await
            .map_err(|e| e.to_string())?;
        let remote_ver = release.tag_name.trim_start_matches('v');
        if !Self::is_newer(remote_ver, Self::CURRENT_VERSION) {
            return Ok(None);
        }

        // Автообновление бинарника поддерживается только на Linux (raw-файл "cibitube").
        // На Windows/macOS в релизе лежат архивы: пользователь скачивает их вручную.
        let download_url = if cfg!(target_os = "linux") {
            release
                .assets
                .iter()
                .find(|a| a.name == "cibitube")
                .map(|a| a.browser_download_url.clone())
        } else {
            None
        };

        let apk_download_url = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(".apk"))
            .map(|a| a.browser_download_url.clone());

        let expected_sha256 = if let Some(ref notes) = release.body {
            Self::extract_sha256_from_release_body(notes, "cibitube").ok()
        } else {
            None
        };

        Ok(Some(UpdateInfo {
            version: release.tag_name,
            release_page_url: release
                .html_url
                .unwrap_or_else(|| Self::RELEASES_URL.to_string()),
            download_url,
            apk_download_url,
            notes: release.body.unwrap_or_default(),
            expected_sha256,
        }))
    }

    pub fn is_newer(remote: &str, current: &str) -> bool {
        // Берём числовой префикс каждой части ("9-beta" -> 9) и выравниваем до 3 чисел
        let parse_ver = |s: &str| -> Vec<u32> {
            let mut v: Vec<u32> = s
                .split('.')
                .map(|p| {
                    p.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u32>()
                        .unwrap_or(0)
                })
                .collect();
            if v.len() < 3 {
                v.resize(3, 0);
            }
            v
        };
        parse_ver(remote) > parse_ver(current)
    }

    /// Извлечение хэша SHA-256 для указанного файла из описания релиза GitHub.
    /// Поддерживает секцию «### Контрольные суммы SHA-256» или любой markdown-блок/список.
    pub fn extract_sha256_from_release_body(
        body: &str,
        asset_name: &str,
    ) -> Result<String, String> {
        let clean_asset = asset_name.trim();
        let search_text = if let Some((_, section)) = body.split_once("Контрольные суммы SHA-256")
        {
            section
        } else {
            body
        };

        for line in search_text.lines() {
            let trimmed = line.trim().trim_matches('`').trim();
            if trimmed.is_empty() {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                // Формат: <64-hex-хэш> <имя_файла>
                let hash_cand = parts[0];
                let name_cand = parts[1].trim_start_matches('*');
                if hash_cand.len() == 64
                    && hash_cand.chars().all(|c| c.is_ascii_hexdigit())
                    && name_cand.eq_ignore_ascii_case(clean_asset)
                {
                    return Ok(hash_cand.to_ascii_lowercase());
                }
                // Формат: <имя_файла>: <64-hex-хэш> или - <имя_файла>: <64-hex-хэш>
                let last = parts[parts.len() - 1];
                if last.len() == 64 && last.chars().all(|c| c.is_ascii_hexdigit()) {
                    let first = parts[0].trim_matches(':').trim_start_matches('-').trim();
                    if first.eq_ignore_ascii_case(clean_asset) {
                        return Ok(last.to_ascii_lowercase());
                    }
                }
            }
        }

        Err(format!(
            "Контрольная сумма SHA-256 для файла «{clean_asset}» не найдена в описании релиза"
        ))
    }

    /// Сверка вычисленного SHA-256 с ожидаемым хэшем
    pub fn verify_sha256(bytes: &[u8], expected_hash: &str) -> bool {
        let computed = hex::encode(Sha256::digest(bytes));
        computed.eq_ignore_ascii_case(expected_hash.trim())
    }

    async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
        let res = client.get(url).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }
        res.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }

    /// Путь к текущему исполняемому файлу (без суффикса " (deleted)" после замены на Linux)
    fn exe_path() -> Result<PathBuf, String> {
        let p = std::env::current_exe().map_err(|e| e.to_string())?;
        let s = p.to_string_lossy();
        Ok(PathBuf::from(s.trim_end_matches(" (deleted)")))
    }

    /// Скачивает новый бинарник, проверяет SHA-256 по описанию релиза и ELF-заголовок,
    /// заменяет текущий файл (с резервной копией `.bak`). Только Linux.
    pub async fn apply_update(
        download_url: Option<String>,
        expected_sha256: Option<String>,
    ) -> Result<(), String> {
        if !cfg!(target_os = "linux") {
            return Err(
                "Автообновление доступно только на Linux. Скачайте новую версию со страницы релизов."
                    .to_string(),
            );
        }
        let url = download_url.ok_or_else(|| "URL для скачивания релиза не найден".to_string())?;
        let (_base, asset_name) = url
            .rsplit_once('/')
            .ok_or_else(|| "Некорректный URL релиза".to_string())?;

        let client = Self::http_client(120)?;

        let expected_hash = match expected_sha256 {
            Some(h) if !h.trim().is_empty() => h,
            _ => {
                let rel_url = format!(
                    "https://api.github.com/repos/{}/releases/latest",
                    Self::REPO
                );
                let resp = client
                    .get(&rel_url)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .map_err(|e| format!("Не удалось запросить релиз для проверки SHA-256: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("GitHub API вернул статус: {}", resp.status()));
                }
                let rel = resp
                    .json::<GitHubRelease>()
                    .await
                    .map_err(|e| format!("Ошибка парсинга релиза: {e}"))?;
                let body = rel.body.unwrap_or_default();
                Self::extract_sha256_from_release_body(&body, asset_name)?
            }
        };

        let bytes = Self::get_bytes(&client, &url)
            .await
            .map_err(|e| format!("Ошибка скачивания: {e}"))?;

        if !Self::verify_sha256(&bytes, &expected_hash) {
            let computed = hex::encode(Sha256::digest(&bytes));
            return Err(format!(
                "Контрольная сумма SHA-256 не совпадает (ожидалось: {}, получено: {}): обновление отменено",
                expected_hash.trim(),
                computed
            ));
        }

        if bytes.len() < 500_000 || !bytes.starts_with(b"\x7fELF") {
            return Err("Скачанный файл не является исполняемым бинарником Linux".to_string());
        }

        Self::install_binary(&bytes)
    }

    fn install_binary(bytes: &[u8]) -> Result<(), String> {
        let exe = Self::exe_path()?;
        let backup = exe.with_extension("bak");
        let tmp = exe.with_extension("new");

        fs::write(&tmp, bytes)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("Не удалось выставить права: {e}"))?;
        }

        let _ = fs::copy(&exe, &backup);
        fs::rename(&tmp, &exe).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Не удалось заменить бинарник {}: {e}", exe.display())
        })
    }

    pub fn open_releases_page() {
        #[cfg(target_os = "linux")]
        let _ = Command::new("xdg-open").arg(Self::RELEASES_URL).spawn();
        #[cfg(target_os = "macos")]
        let _ = Command::new("open").arg(Self::RELEASES_URL).spawn();
        #[cfg(target_os = "windows")]
        let _ = Command::new("cmd")
            .args(["/C", "start", "", Self::RELEASES_URL])
            .spawn();
    }

    pub fn restart() {
        if let Ok(exe) = Self::exe_path() {
            let _ = Command::new(exe).spawn();
        }
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(Updater::is_newer("0.2.10", "0.2.9"));
        assert!(Updater::is_newer("0.3", "0.2.9"));
        assert!(Updater::is_newer("0.3.0", "0.2.9"));
        assert!(!Updater::is_newer("0.2.9", "0.2.9"));
        assert!(!Updater::is_newer("0.2.8", "0.2.9"));
        assert!(Updater::is_newer("0.2.10-beta", "0.2.9"));
    }

    #[test]
    fn test_extract_sha256_from_release_body() {
        let release_body = "\
## Изменения в Cibitube v0.2.10

- Улучшена надежность текстов
- Исправлена сборка на Windows

### Контрольные суммы SHA-256
```
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  cibitube
a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90  cibitube-linux-x86_64.tar.gz
```
";
        let extracted = Updater::extract_sha256_from_release_body(release_body, "cibitube");
        assert_eq!(
            extracted,
            Ok("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string())
        );

        let tar_extracted =
            Updater::extract_sha256_from_release_body(release_body, "cibitube-linux-x86_64.tar.gz");
        assert_eq!(
            tar_extracted,
            Ok("a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90".to_string())
        );

        // Missing file
        let missing = Updater::extract_sha256_from_release_body(release_body, "nonexistent");
        assert!(missing.is_err());
        assert!(missing
            .unwrap_err()
            .contains("Контрольная сумма SHA-256 для файла «nonexistent» не найдена"));

        // Empty body or no checksums
        let empty_res = Updater::extract_sha256_from_release_body("", "cibitube");
        assert!(empty_res.is_err());
    }

    #[test]
    fn test_verify_sha256() {
        let content = b"cibitube update binary test";
        let hash = hex::encode(Sha256::digest(content));
        assert!(Updater::verify_sha256(content, &hash));
        assert!(!Updater::verify_sha256(b"wrong bytes", &hash));
        assert!(!Updater::verify_sha256(
            content,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }
}
