use crate::models::{AccountProfile, Playlist, Track};
use serde_json::Value;
use sha1::{Sha1, Digest};

pub struct InnertubeClient {
    client: reqwest::Client,
}

impl InnertubeClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Helper to compute SAPISIDHASH auth header if SAPISID or __Secure-3PAPISID cookie exists
    fn get_sapisid_auth(cookie: &str) -> Option<(String, &'static str, &'static str)> {
        let sapisid = cookie.split(';')
            .map(|s| s.trim())
            .find_map(|s| {
                if let Some(val) = s.strip_prefix("SAPISID=") {
                    Some(val)
                } else if let Some(val) = s.strip_prefix("__Secure-3PAPISID=") {
                    Some(val)
                } else {
                    None
                }
            })?;

        let origin = "https://music.youtube.com";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let to_hash = format!("{} {} {}", now, sapisid, origin);
        let mut hasher = Sha1::new();
        hasher.update(to_hash.as_bytes());
        let hash_hex = format!("{:x}", hasher.finalize());

        let auth_header = format!("SAPISIDHASH {}_{}", now, hash_hex);
        Some((auth_header, origin, "https://music.youtube.com/"))
    }

    /// Discovers all Google/YouTube accounts associated with this cookie session (e.g. authuser 0, 1)
    pub async fn discover_accounts(&self, cookie: &str) -> Vec<AccountProfile> {
        if cookie.is_empty() {
            return Vec::new();
        }
        let mut list = Vec::new();

        for u in 0..4 {
            let payload = serde_json::json!({
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        "clientVersion": "1.20240101.01.00",
                        "hl": "ru",
                        "gl": "RU"
                    }
                }
            });

            let mut req = self.client
                .post(format!("https://music.youtube.com/youtubei/v1/account/account_menu?prettyPrint=false&authuser={}", u))
                .header("Cookie", cookie)
                .header("X-YouTube-Client-Name", "67")
                .header("X-YouTube-Client-Version", "1.20240101.01.00")
                .header("X-Goog-AuthUser", u.to_string())
                .json(&payload);

            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                req = req.header("Authorization", auth)
                         .header("Origin", origin)
                         .header("Referer", referer);
            }

            if let Ok(res) = req.send().await {
                if let Ok(json) = res.json::<Value>().await {
                    let header = json.pointer("/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer");
                    let name = header.and_then(|h| h.pointer("/accountName/runs/0/text")).and_then(|t| t.as_str());
                    let email = header.and_then(|h| h.pointer("/email/runs/0/text")).and_then(|t| t.as_str()).unwrap_or("");

                    if let Some(n) = name {
                        if !n.trim().is_empty() {
                            list.push(AccountProfile {
                                name: n.to_string(),
                                email: email.to_string(),
                                auth_user: u,
                            });
                            continue;
                        }
                    }
                }
            }

            if u > 0 {
                break;
            }
        }

        list
    }

    pub async fn search(&self, query: &str, country: &str, cookie: &str, auth_user: usize) -> Result<Vec<Track>, String> {
        let payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": "1.20240101.01.00",
                    "hl": "ru",
                    "gl": country
                }
            },
            "query": query
        });

        let mut req = self.client
            .post(format!("https://music.youtube.com/youtubei/v1/search?prettyPrint=false&authuser={}", auth_user))
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if !cookie.is_empty() {
            req = req.header("Cookie", cookie);
            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                req = req.header("Authorization", auth)
                         .header("Origin", origin)
                         .header("Referer", referer);
            }
        }

        let res = req.send()
            .await
            .map_err(|e| format!("Сетевая ошибка: {}", e))?;

        let json: Value = res.json()
            .await
            .map_err(|e| format!("Ошибка парсинга ответа: {}", e))?;

        let mut tracks = Vec::new();

        let tabs = json.pointer("/contents/tabbedSearchResultsRenderer/tabs")
            .and_then(|v| v.as_array());

        if let Some(tabs) = tabs {
            if let Some(tab0) = tabs.first() {
                let sections = tab0.pointer("/tabRenderer/content/sectionListRenderer/contents")
                    .and_then(|v| v.as_array());

                if let Some(sections) = sections {
                    for section in sections {
                        if let Some(shelf) = section.get("musicShelfRenderer") {
                            if let Some(contents) = shelf.get("contents").and_then(|v| v.as_array()) {
                                for item in contents {
                                    if let Some(track) = Self::parse_item(item) {
                                        tracks.push(track);
                                    }
                                }
                            }
                        } else if let Some(card) = section.get("musicCardShelfRenderer") {
                            if let Some(track) = Self::parse_card(card) {
                                tracks.push(track);
                            }
                        }
                    }
                }
            }
        }

        Ok(tracks)
    }

    pub async fn fetch_liked_music(&self, cookie: &str, auth_user: usize) -> Result<Vec<Track>, String> {
        if cookie.is_empty() {
            return Err("Для загрузки любимых треков с YouTube войдите в аккаунт во вкладке Настройки".to_string());
        }

        let payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": "1.20240101.01.00",
                    "hl": "ru",
                    "gl": "RU"
                }
            },
            "browseId": "FEmusic_liked_videos"
        });

        let mut req = self.client
            .post(format!("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}", auth_user))
            .header("Cookie", cookie)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
            req = req.header("Authorization", auth)
                     .header("Origin", origin)
                     .header("Referer", referer);
        }

        let res = req.send()
            .await
            .map_err(|e| format!("Ошибка загрузки: {}", e))?;

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        let mut tracks = Vec::new();

        if let Some(tabs) = json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs").and_then(|v| v.as_array()) {
            for tab in tabs {
                if let Some(sections) = tab.pointer("/tabRenderer/content/sectionListRenderer/contents").and_then(|v| v.as_array()) {
                    for sec in sections {
                        if let Some(items) = sec.pointer("/musicPlaylistShelfRenderer/contents")
                            .or_else(|| sec.pointer("/musicShelfRenderer/contents"))
                            .and_then(|v| v.as_array()) {
                            for item in items {
                                if let Some(t) = Self::parse_item(item) {
                                    tracks.push(t);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(tracks)
    }

    pub async fn fetch_user_playlists(&self, cookie: &str, auth_user: usize) -> Result<Vec<Playlist>, String> {
        if cookie.is_empty() {
            return Err("Для загрузки плейлистов требуется войти в аккаунт".to_string());
        }

        let payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": "1.20240101.01.00",
                    "hl": "ru",
                    "gl": "RU"
                }
            },
            "browseId": "FEmusic_liked_playlists"
        });

        let mut req = self.client
            .post(format!("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}", auth_user))
            .header("Cookie", cookie)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
            req = req.header("Authorization", auth)
                     .header("Origin", origin)
                     .header("Referer", referer);
        }

        let res = req.send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let mut playlists = Vec::new();

        fn extract_playlists(val: &Value, out: &mut Vec<Playlist>) {
            if let Some(obj) = val.as_object() {
                if let Some(it) = obj.get("musicTwoRowItemRenderer") {
                    let title = it.pointer("/title/runs/0/text").and_then(|t| t.as_str()).unwrap_or("");
                    let subtitle = it.pointer("/subtitle/runs")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|x| x.pointer("/text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(""))
                        .unwrap_or_default();
                    let browse_id = it.pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(|b| b.as_str())
                        .unwrap_or("");
                    let thumbnail = it.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.last())
                        .and_then(|t| t.get("url"))
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string());

                    if !browse_id.is_empty() 
                        && !title.eq_ignore_ascii_case("новый") 
                        && !title.eq_ignore_ascii_case("new playlist")
                        && !out.iter().any(|p| p.id == browse_id) {
                        out.push(Playlist {
                            id: browse_id.to_string(),
                            title: title.to_string(),
                            subtitle,
                            thumbnail_url: thumbnail,
                        });
                    }
                }
                for v in obj.values() {
                    extract_playlists(v, out);
                }
            } else if let Some(arr) = val.as_array() {
                for item in arr {
                    extract_playlists(item, out);
                }
            }
        }

        extract_playlists(&json, &mut playlists);

        // Fallback: If liked playlists didn't return anything or for additional playlists, check library landing
        if playlists.is_empty() {
            let landing_payload = serde_json::json!({
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        "clientVersion": "1.20240101.01.00",
                        "hl": "ru",
                        "gl": "RU"
                    }
                },
                "browseId": "FEmusic_library_landing"
            });

            let mut l_req = self.client
                .post(format!("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}", auth_user))
                .header("Cookie", cookie)
                .header("X-YouTube-Client-Name", "67")
                .header("X-YouTube-Client-Version", "1.20240101.01.00")
                .header("X-Goog-AuthUser", auth_user.to_string())
                .json(&landing_payload);

            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                l_req = l_req.header("Authorization", auth)
                             .header("Origin", origin)
                             .header("Referer", referer);
            }

            if let Ok(l_res) = l_req.send().await {
                if let Ok(l_json) = l_res.json::<Value>().await {
                    extract_playlists(&l_json, &mut playlists);
                }
            }
        }

        Ok(playlists)
    }

    pub async fn fetch_playlist_tracks(&self, browse_id: &str, cookie: &str, auth_user: usize) -> Result<Vec<Track>, String> {
        let actual_id = if browse_id.starts_with("VL") || browse_id.starts_with("FE") {
            browse_id.to_string()
        } else {
            format!("VL{}", browse_id)
        };

        let payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": "1.20240101.01.00",
                    "hl": "ru",
                    "gl": "RU"
                }
            },
            "browseId": actual_id
        });

        let mut req = self.client
            .post(format!("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}", auth_user))
            .header("Cookie", cookie)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if !cookie.is_empty() {
            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                req = req.header("Authorization", auth)
                         .header("Origin", origin)
                         .header("Referer", referer);
            }
        }

        let res = req.send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        println!("DEBUG JSON: {}", serde_json::to_string(&json).unwrap());

        let mut tracks = Vec::new();

        fn extract_tracks(val: &Value, out: &mut Vec<Track>) {
            if let Some(obj) = val.as_object() {
                if obj.contains_key("musicResponsiveListItemRenderer") {
                    if let Some(t) = InnertubeClient::parse_item(val) {
                        out.push(t);
                    }
                    return;
                }
                for v in obj.values() {
                    extract_tracks(v, out);
                }
            } else if let Some(arr) = val.as_array() {
                for item in arr {
                    extract_tracks(item, out);
                }
            }
        }

        extract_tracks(&json, &mut tracks);
        Ok(tracks)
    }

    fn parse_item(item: &Value) -> Option<Track> {
        let r = item.get("musicResponsiveListItemRenderer")?;

        let flex = r.get("flexColumns")?.as_array()?;
        let col0 = flex.first()?;
        let runs0 = col0.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")?.as_array()?;
        
        let title = runs0.iter()
            .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");

        let mut artist = String::new();
        let mut duration = String::new();

        if flex.len() > 1 {
            if let Some(runs1) = flex[1].pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs").and_then(|v| v.as_array()) {
                let parts: Vec<&str> = runs1.iter()
                    .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
                    .filter(|&t| t != " • ")
                    .collect();
                if let Some(first) = parts.first() {
                    artist = first.to_string();
                }
                if let Some(last) = parts.last() {
                    if last.contains(':') {
                        duration = last.to_string();
                    }
                }
            }
        }

        let video_id = r.pointer("/playlistItemData/videoId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })?;

        let thumbnail = r.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        Some(Track {
            id: video_id,
            title,
            artist: if artist.is_empty() { "Неизвестный исполнитель".to_string() } else { artist },
            duration_text: duration,
            thumbnail_url: thumbnail,
        })
    }

    fn parse_card(card: &Value) -> Option<Track> {
        let title = card.pointer("/title/runs/0/text")
            .and_then(|v| v.as_str())?
            .to_string();

        let artist = card.pointer("/subtitle/runs/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("Неизвестный")
            .to_string();

        let video_id = card.pointer("/buttons/0/buttonRenderer/command/watchEndpoint/videoId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                card.pointer("/title/runs/0/navigationEndpoint/watchEndpoint/videoId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })?;

        let thumbnail = card.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        Some(Track {
            id: video_id,
            title,
            artist,
            duration_text: String::new(),
            thumbnail_url: thumbnail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_auth::BrowserAuth;

    #[tokio::test]
    async fn test_fetch_playlists() {
        let client = InnertubeClient::new();
        let cookie = BrowserAuth::extract_firefox_cookies().expect("Cookies from Firefox");
        let accounts = client.discover_accounts(&cookie).await;
        println!("Discovered accounts: {:?}", accounts);

        for authuser in [0, 1] {
            let res = client.fetch_user_playlists(&cookie, authuser).await;
            println!("Fetch user playlists result (authuser={}): {:?}", authuser, res);
            assert!(res.is_ok());
        }
    }
}
