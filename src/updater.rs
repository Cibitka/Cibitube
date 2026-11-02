use serde::{Deserialize, Serialize};
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
    pub name: Option<String>,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: Option<String>,
    pub notes: String,
}

pub struct Updater;

impl Updater {
    pub const REPO: &'static str = "Cibitka/Cibitube";
    pub const CURRENT_VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Checks if a newer version exists on GitHub
    pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
        let client = reqwest::Client::builder()
            .user_agent("Cibitube-Updater")
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .map_err(|e| e.to_string())?;

        let url = format!("https://api.github.com/repos/{}/releases/latest", Self::REPO);
        let resp = client.get(&url).send().await;

        if let Ok(res) = resp {
            if res.status().is_success() {
                if let Ok(release) = res.json::<GitHubRelease>().await {
                    let remote_ver = release.tag_name.trim_start_matches('v');
                    if Self::is_newer(remote_ver, Self::CURRENT_VERSION) {
                        let dl_url = release.assets.iter()
                            .find(|a| a.name == "metrolist" || a.name == "cibitube" || a.name.contains("linux"))
                            .map(|a| a.browser_download_url.clone());

                        return Ok(Some(UpdateInfo {
                            version: release.tag_name,
                            download_url: dl_url,
                            notes: release.body.unwrap_or_default(),
                        }));
                    } else {
                        return Ok(None);
                    }
                }
            }
        }

        // Fallback for private repo or rate limit: query remote tags via git ls-remote (uses SSH key)
        let output = tokio::task::spawn_blocking(|| {
            Command::new("git")
                .args(["ls-remote", "--tags", "origin"])
                .output()
        }).await.map_err(|e| e.to_string())?;

        if let Ok(out) = output {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut latest_tag: Option<String> = None;
                for line in stdout.lines() {
                    if let Some(tag_ref) = line.split_whitespace().nth(1) {
                        if let Some(tag) = tag_ref.strip_prefix("refs/tags/") {
                            let clean_tag = tag.trim_end_matches("^{}");
                            if clean_tag.starts_with('v') {
                                let ver = clean_tag.trim_start_matches('v');
                                if Self::is_newer(ver, Self::CURRENT_VERSION) {
                                    latest_tag = Some(clean_tag.to_string());
                                }
                            }
                        }
                    }
                }

                if let Some(tag) = latest_tag {
                    let dl_url = format!("https://github.com/{}/releases/download/{}/metrolist", Self::REPO, tag);
                    return Ok(Some(UpdateInfo {
                        version: tag,
                        download_url: Some(dl_url),
                        notes: "Свежий релиз из GitHub".to_string(),
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Compare semver-like versions
    fn is_newer(remote: &str, current: &str) -> bool {
        let parse_ver = |s: &str| -> Vec<u32> {
            s.split('.')
                .filter_map(|p| p.parse::<u32>().ok())
                .collect()
        };
        let r = parse_ver(remote);
        let c = parse_ver(current);
        r > c
    }

    /// Performs update: downloads new binary, sets +x, replaces current binary atomically
    pub async fn apply_update(download_url: Option<String>) -> Result<(), String> {
        let install_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka"))
            .join(".local/bin/metrolist");

        let symlink_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka"))
            .join(".local/bin/cibitube");

        let tmp_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka"))
            .join(".local/bin/metrolist.download");

        let mut downloaded = false;
        if let Some(url) = download_url {
            let client = reqwest::Client::builder()
                .user_agent("Cibitube-Updater")
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| e.to_string())?;

            if let Ok(res) = client.get(&url).send().await {
                if res.status().is_success() {
                    if let Ok(bytes) = res.bytes().await {
                        if bytes.len() > 1_000_000 {
                            std::fs::write(&tmp_path, &bytes).map_err(|e| e.to_string())?;
                            downloaded = true;
                        }
                    }
                }
            }
        }

        // If direct HTTP download was blocked (private repo), build via git & cargo
        if !downloaded {
            let tmp_dest = tmp_path.clone();
            tokio::task::spawn_blocking(move || {
                let repo_dir = PathBuf::from("/home/cibitka/metrolist-desktop");
                let pull = Command::new("git")
                    .current_dir(&repo_dir)
                    .args(["pull", "origin", "main"])
                    .status();
                if pull.is_err() || !pull.unwrap().success() {
                    return Err("Не удалось загрузить изменения через git pull".to_string());
                }
                let build = Command::new("cargo")
                    .current_dir(&repo_dir)
                    .args(["build", "--release"])
                    .status();
                if build.is_err() || !build.unwrap().success() {
                    return Err("Ошибка при компиляции новой версии".to_string());
                }
                let built_bin = repo_dir.join("target/release/metrolist-desktop");
                if built_bin.exists() {
                    let _ = std::fs::copy(&built_bin, &tmp_dest);
                    let _ = Command::new("cargo").current_dir(&repo_dir).arg("clean").status();
                    Ok(())
                } else {
                    Err("Скомпилированный бинарник не найден".to_string())
                }
            }).await.map_err(|e| e.to_string())??;
        }

        // Set executable permission (chmod +x)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }

        // Atomically replace existing binary
        std::fs::rename(&tmp_path, &install_path).map_err(|e| format!("Не удалось заменить файл: {}", e))?;

        // Update symlink
        let _ = std::fs::remove_file(&symlink_path);
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&install_path, &symlink_path);

        Ok(())
    }

    /// Restarts application with new binary
    pub fn restart() {
        let bin = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/cibitka"))
            .join(".local/bin/metrolist");
        let _ = Command::new(bin).spawn();
        std::process::exit(0);
    }
}
