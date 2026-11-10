use crate::models::{AccountProfile, Playlist, Track};
use serde_json::Value;
use sha1::{Digest, Sha1};

pub struct InnertubeClient {
    client: reqwest::Client,
}

pub(crate) struct PlaylistChoice {
    pub playlist: Playlist,
    pub membership: PlaylistMembership,
}

pub(crate) enum PlaylistMembership {
    Present(Option<String>),
    Absent,
    Unknown,
}

impl InnertubeClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Automatically upscales YouTube Music thumbnails to HD / 800x800 master resolution
    pub fn upgrade_thumbnail_quality(url: &str) -> String {
        if url.contains("googleusercontent.com") || url.contains("ggpht.com") {
            if let Some(idx) = url.rfind('=') {
                let base = &url[..idx];
                return format!("{}=w800-h800-l90-rj", base);
            }
        } else if url.contains("i.ytimg.com/vi/") {
            if url.contains("hqdefault.jpg") {
                return url.replace("hqdefault.jpg", "maxresdefault.jpg");
            } else if url.contains("mqdefault.jpg") {
                return url.replace("mqdefault.jpg", "maxresdefault.jpg");
            }
        }
        url.to_string()
    }

    /// Helper to compute SAPISIDHASH auth header if SAPISID or __Secure-3PAPISID cookie exists
    fn get_sapisid_auth(cookie: &str) -> Option<(String, &'static str, &'static str)> {
        let sapisid = cookie.split(';').map(|s| s.trim()).find_map(|s| {
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
                req = req
                    .header("Authorization", auth)
                    .header("Origin", origin)
                    .header("Referer", referer);
            }

            if let Ok(res) = req.send().await {
                if let Ok(json) = res.json::<Value>().await {
                    let header = json.pointer("/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer");
                    let name = header
                        .and_then(|h| h.pointer("/accountName/runs/0/text"))
                        .and_then(|t| t.as_str());
                    let email = header
                        .and_then(|h| h.pointer("/email/runs/0/text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("");

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

    pub async fn search(
        &self,
        query: &str,
        country: &str,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Vec<Track>, String> {
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

        let mut req = self
            .client
            .post(format!(
                "https://music.youtube.com/youtubei/v1/search?prettyPrint=false&authuser={}",
                auth_user
            ))
            .timeout(std::time::Duration::from_secs(20))
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if !cookie.is_empty() {
            req = req.header("Cookie", cookie);
            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                req = req
                    .header("Authorization", auth)
                    .header("Origin", origin)
                    .header("Referer", referer);
            }
        }

        let res = req
            .send()
            .await
            .map_err(|e| format!("Сетевая ошибка: {}", e))?
            .error_for_status()
            .map_err(|e| format!("YouTube Music отклонил поиск: {e}"))?;

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Ошибка парсинга ответа: {}", e))?;

        let mut tracks = Vec::new();

        let tabs = json
            .pointer("/contents/tabbedSearchResultsRenderer/tabs")
            .and_then(|v| v.as_array());

        if let Some(tabs) = tabs {
            if let Some(tab0) = tabs.first() {
                let sections = tab0
                    .pointer("/tabRenderer/content/sectionListRenderer/contents")
                    .and_then(|v| v.as_array());

                if let Some(sections) = sections {
                    for section in sections {
                        if let Some(shelf) = section.get("musicShelfRenderer") {
                            if let Some(contents) = shelf.get("contents").and_then(|v| v.as_array())
                            {
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

    pub async fn fetch_radio(&self, video_id: &str, country: &str) -> Result<Vec<Track>, String> {
        let payload = serde_json::json!({
            "context": {"client": {"clientName": "WEB_REMIX",
                "clientVersion": "1.20240101.01.00", "hl": "ru", "gl": country}},
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "isAudioOnly": true
        });
        // Radio is public once we have the seed ID, even if it came from a private playlist.
        let response = self
            .client
            .post("https://music.youtube.com/youtubei/v1/next?prettyPrint=false")
            .timeout(std::time::Duration::from_secs(20))
            .header("Origin", "https://music.youtube.com")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Радио недоступно: {}", e.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "YouTube Music отклонил запрос радио (HTTP {}). Попробуй другой трек.",
                status.as_u16()
            ));
        }
        let json: Value = response
            .json()
            .await
            .map_err(|e| format!("Ответ радио не распознан: {e}"))?;
        let tracks = Self::parse_radio_tracks(&json);
        if tracks.is_empty() {
            return Err(
                "YouTube Music не вернул радио для этого трека. Попробуй другую песню.".into(),
            );
        }
        Ok(tracks)
    }

    fn parse_radio_tracks(json: &Value) -> Vec<Track> {
        fn text(value: &Value) -> String {
            value
                .get("simpleText")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    value
                        .get("runs")
                        .and_then(Value::as_array)
                        .map(|runs| {
                            runs.iter()
                                .filter_map(|r| r.get("text").and_then(Value::as_str))
                                .collect()
                        })
                        .unwrap_or_default()
                })
        }
        fn collect(value: &Value, tracks: &mut Vec<Track>) {
            if let Some(renderer) = value.get("playlistPanelVideoRenderer") {
                if renderer.get("isPlayable").and_then(Value::as_bool) == Some(false) {
                    return;
                }
                if let Some(id) = renderer.get("videoId").and_then(Value::as_str) {
                    let title = text(&renderer["title"]);
                    if !title.is_empty() {
                        tracks.push(Track {
                            id: id.to_owned(),
                            title,
                            artist: text(&renderer["shortBylineText"]),
                            duration_text: text(&renderer["lengthText"]),
                            thumbnail_url: renderer
                                .pointer("/thumbnail/thumbnails")
                                .and_then(Value::as_array)
                                .and_then(|items| items.last())
                                .and_then(|item| item.get("url"))
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        });
                    }
                }
                return;
            }
            match value {
                Value::Object(object) => {
                    for child in object.values() {
                        collect(child, tracks);
                    }
                }
                Value::Array(array) => {
                    for child in array {
                        collect(child, tracks);
                    }
                }
                _ => {}
            }
        }
        let mut tracks = Vec::new();
        collect(json, &mut tracks);
        let mut seen = std::collections::HashSet::new();
        tracks.retain(|track| seen.insert(track.id.clone()));
        tracks
    }

    pub async fn fetch_liked_music(
        &self,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Vec<Track>, String> {
        if cookie.is_empty() {
            return Err(
                "Для загрузки любимых треков с YouTube войдите в аккаунт во вкладке Настройки"
                    .to_string(),
            );
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

        let mut req = self
            .client
            .post(format!(
                "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}",
                auth_user
            ))
            .header("Cookie", cookie)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
            req = req
                .header("Authorization", auth)
                .header("Origin", origin)
                .header("Referer", referer);
        }

        let res = req
            .send()
            .await
            .map_err(|e| format!("Ошибка загрузки: {}", e))?;

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        let mut tracks = Vec::new();

        if let Some(tabs) = json
            .pointer("/contents/singleColumnBrowseResultsRenderer/tabs")
            .and_then(|v| v.as_array())
        {
            for tab in tabs {
                if let Some(sections) = tab
                    .pointer("/tabRenderer/content/sectionListRenderer/contents")
                    .and_then(|v| v.as_array())
                {
                    for sec in sections {
                        if let Some(items) = sec
                            .pointer("/musicPlaylistShelfRenderer/contents")
                            .or_else(|| sec.pointer("/musicShelfRenderer/contents"))
                            .and_then(|v| v.as_array())
                        {
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

    pub async fn fetch_user_playlists(
        &self,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Vec<Playlist>, String> {
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

        let mut req = self
            .client
            .post(format!(
                "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}",
                auth_user
            ))
            .header("Cookie", cookie)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload);

        if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
            req = req
                .header("Authorization", auth)
                .header("Origin", origin)
                .header("Referer", referer);
        }

        let res = req.send().await.map_err(|e| e.to_string())?;
        let json: Value = res.json().await.map_err(|e| e.to_string())?;

        let mut playlists = Vec::new();

        fn extract_playlists(val: &Value, out: &mut Vec<Playlist>) {
            if let Some(obj) = val.as_object() {
                if let Some(it) = obj.get("musicTwoRowItemRenderer") {
                    let title = it
                        .pointer("/title/runs/0/text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    let subtitle = it
                        .pointer("/subtitle/runs")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.pointer("/text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    let browse_id = it
                        .pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(|b| b.as_str())
                        .unwrap_or("");
                    let thumbnail = it
                        .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.last())
                        .and_then(|t| t.get("url"))
                        .and_then(|u| u.as_str())
                        .map(str::to_owned);

                    if !browse_id.is_empty()
                        && !title.eq_ignore_ascii_case("новый")
                        && !title.eq_ignore_ascii_case("new playlist")
                        && !out.iter().any(|p| p.id == browse_id)
                    {
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

            let mut l_req = self
                .client
                .post(format!(
                    "https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={}",
                    auth_user
                ))
                .header("Cookie", cookie)
                .header("X-YouTube-Client-Name", "67")
                .header("X-YouTube-Client-Version", "1.20240101.01.00")
                .header("X-Goog-AuthUser", auth_user.to_string())
                .json(&landing_payload);

            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                l_req = l_req
                    .header("Authorization", auth)
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

    async fn playlist_request(
        &self,
        endpoint: &str,
        mut payload: Value,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Value, String> {
        let (auth, origin, referer) =
            Self::get_sapisid_auth(cookie).ok_or("Войди в аккаунт YouTube Music в настройках.")?;
        payload["context"] = serde_json::json!({
            "client": {"clientName": "WEB_REMIX", "clientVersion": "1.20240101.01.00", "hl": "ru"}
        });
        let response = self
            .client
            .post(format!(
                "{origin}/youtubei/v1/{endpoint}?prettyPrint=false&authuser={auth_user}"
            ))
            .timeout(std::time::Duration::from_secs(20))
            .header("Cookie", cookie)
            .header("Authorization", auth)
            .header("Origin", origin)
            .header("X-Origin", origin)
            .header("Referer", referer)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20240101.01.00")
            .header("X-Goog-AuthUser", auth_user.to_string())
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Не удалось связаться с YouTube Music: {}", e.without_url()))?;
        let status = response.status();
        match status.as_u16() {
            401 => return Err("Сессия YouTube Music истекла. Войди в аккаунт заново.".into()),
            403 => {
                return Err(
                    "YouTube Music не разрешил это действие для выбранного аккаунта.".into(),
                )
            }
            _ if !status.is_success() => {
                return Err(format!(
                    "YouTube Music отклонил запрос (HTTP {}).",
                    status.as_u16()
                ));
            }
            _ => {}
        }
        response
            .json()
            .await
            .map_err(|_| "Не удалось прочитать ответ YouTube Music.".into())
    }

    pub(crate) async fn playlist_choices(
        &self,
        track_id: &str,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Vec<PlaylistChoice>, String> {
        let json = self
            .playlist_request(
                "playlist/get_add_to_playlist",
                serde_json::json!({"videoIds": [track_id]}),
                cookie,
                auth_user,
            )
            .await?;
        let mut choices = Self::parse_playlist_choices(&json, track_id);
        // The picker endpoint can report NONE for tracks already saved. Check
        // uncertain results against the playlist entries themselves.
        let mut checks = tokio::task::JoinSet::new();
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(5));
        let cookie: std::sync::Arc<str> = cookie.into();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
        for (index, choice) in choices.iter().enumerate() {
            if !matches!(choice.membership, PlaylistMembership::Unknown) {
                continue;
            }
            let client = Self {
                client: self.client.clone(),
            };
            let playlist_id = choice.playlist.id.clone();
            let track_id = track_id.to_owned();
            let cookie = cookie.clone();
            let slots = slots.clone();
            checks.spawn(async move {
                let _slot = slots
                    .acquire()
                    .await
                    .expect("playlist checker keeps its semaphore open");
                (
                    index,
                    client
                        .check_playlist_membership(&playlist_id, &track_id, &cookie, auth_user)
                        .await,
                )
            });
        }
        while let Ok(Some(result)) = tokio::time::timeout_at(deadline, checks.join_next()).await {
            if let Ok((index, membership)) = result {
                choices[index].membership = membership;
            }
        }
        Ok(choices)
    }

    fn parse_playlist_choices(json: &Value, track_id: &str) -> Vec<PlaylistChoice> {
        fn text(value: &Value) -> Option<String> {
            if let Some(simple) = value["simpleText"].as_str() {
                return Some(simple.to_owned());
            }
            Some(
                value["runs"]
                    .as_array()?
                    .iter()
                    .filter_map(|run| run["text"].as_str())
                    .collect(),
            )
        }

        fn collect(node: &Value, track_id: &str, choices: &mut Vec<PlaylistChoice>) {
            for name in [
                "playlistAddToOptionRenderer",
                "addToPlaylistItemRenderer",
                "musicResponsiveListItemRenderer",
                "musicTwoRowItemRenderer",
            ] {
                let Some(option) = node.get(name) else {
                    continue;
                };
                let id = [
                    "/serviceEndpoint/playlistEditEndpoint/playlistId",
                    "/navigationEndpoint/playlistEditEndpoint/playlistId",
                    "/serviceEndpoint/addToPlaylistEndpoint/playlistId",
                    "/addToPlaylistEndpoint/playlistId",
                    "/playlistId",
                ]
                .into_iter()
                .find_map(|path| option.pointer(path).and_then(Value::as_str));
                let flex_text = |index| {
                    option
                        .pointer(&format!(
                            "/flexColumns/{index}/musicResponsiveListItemFlexColumnRenderer/text"
                        ))
                        .and_then(text)
                };
                let title = text(&option["title"]).or_else(|| flex_text(0));
                if let (Some(id), Some(title)) = (id, title) {
                    let id = id.strip_prefix("VL").unwrap_or(id);
                    if !id.is_empty()
                        && !title.is_empty()
                        && !choices.iter().any(|choice| choice.playlist.id == id)
                    {
                        let thumbnail_url = [
                            "/thumbnail/thumbnails",
                            "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
                            "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
                            "/thumbnailRenderer/playlistThumbnailRenderer/thumbnail/thumbnails",
                        ]
                        .iter()
                        .find_map(|path| option.pointer(path)?.as_array()?.last()?["url"].as_str())
                        .map(str::to_owned);
                        let removal_entry_id = option
                            .pointer(
                                "/removeFromPlaylistServiceEndpoint/playlistEditEndpoint/actions",
                            )
                            .and_then(Value::as_array)
                            .and_then(|actions| {
                                actions.iter().find_map(|action| {
                                    if action["action"].as_str() == Some("ACTION_REMOVE_VIDEO")
                                        && action["removedVideoId"]
                                            .as_str()
                                            .is_none_or(|id| id == track_id)
                                    {
                                        action["setVideoId"].as_str()
                                    } else {
                                        None
                                    }
                                })
                            })
                            .map(str::to_owned);
                        let present = removal_entry_id.is_some()
                            || option["containsSelectedVideos"].as_str() == Some("ALL")
                            || (option["containsSelectedVideos"].as_str() != Some("NONE")
                                && ["selected", "checked", "isSelected"]
                                    .iter()
                                    .any(|key| option[*key].as_bool() == Some(true)));
                        choices.push(PlaylistChoice {
                            playlist: Playlist {
                                id: id.into(),
                                title,
                                subtitle: text(&option["subtitle"])
                                    .or_else(|| flex_text(1))
                                    .unwrap_or_default(),
                                thumbnail_url,
                            },
                            membership: if present {
                                PlaylistMembership::Present(removal_entry_id)
                            } else {
                                PlaylistMembership::Unknown
                            },
                        });
                    }
                }
                return;
            }
            match node {
                Value::Array(items) => items
                    .iter()
                    .for_each(|item| collect(item, track_id, choices)),
                Value::Object(items) => items
                    .values()
                    .for_each(|item| collect(item, track_id, choices)),
                _ => {}
            }
        }
        let mut choices = Vec::new();
        collect(json, track_id, &mut choices);
        choices
    }

    pub(crate) async fn add_track_to_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        allow_duplicate: bool,
        cookie: &str,
        auth_user: usize,
    ) -> Result<(), String> {
        let mut action =
            serde_json::json!({"action": "ACTION_ADD_VIDEO", "addedVideoId": track_id});
        if allow_duplicate {
            action["dedupeOption"] = "DEDUPE_OPTION_SKIP".into();
        }
        self.edit_playlist(playlist_id, action, cookie, auth_user)
            .await
    }

    fn playlist_row_video_id(row: &Value) -> Option<&str> {
        [
            "/playlistItemData/videoId",
            "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            "/navigationEndpoint/watchEndpoint/videoId",
        ].iter().find_map(|path| row.pointer(path)?.as_str())
    }

    fn playlist_row_membership(item: &Value, track_id: &str) -> Option<PlaylistMembership> {
        let row = item.get("musicResponsiveListItemRenderer")?;
        if Self::playlist_row_video_id(row)? != track_id {
            return None;
        }
        let entry_id = [
            "/playlistItemData/playlistSetVideoId",
            "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/playlistSetVideoId",
        ].iter().find_map(|path| row.pointer(path)?.as_str())
        .or_else(|| row.pointer("/menu/menuRenderer/items")?.as_array()?.iter().find_map(|item| {
            let actions = item.pointer("/menuServiceItemRenderer/serviceEndpoint/playlistEditEndpoint/actions")?.as_array()?;
            actions.iter().find_map(|action| {
                if action["action"].as_str() == Some("ACTION_REMOVE_VIDEO")
                    && action["removedVideoId"].as_str().is_none_or(|id| id == track_id)
                {
                    action["setVideoId"].as_str()
                } else {
                    None
                }
            })
        })).map(str::to_owned);
        Some(PlaylistMembership::Present(entry_id))
    }

    pub(crate) async fn check_playlist_membership(
        &self,
        playlist_id: &str,
        track_id: &str,
        cookie: &str,
        auth_user: usize,
    ) -> PlaylistMembership {
        let mut membership = PlaylistMembership::Absent;
        let read = self.scan_playlist(playlist_id, cookie, auth_user, |item| {
            if let Some(found) = Self::playlist_row_membership(item, track_id) {
                membership = found;
                true
            } else {
                false
            }
        });
        if matches!(
            tokio::time::timeout(std::time::Duration::from_secs(10), read).await,
            Ok(Ok(()))
        ) {
            membership
        } else {
            PlaylistMembership::Unknown
        }
    }

    pub(crate) async fn remove_track_from_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        entry_id: Option<String>,
        cookie: &str,
        auth_user: usize,
    ) -> Result<(), String> {
        let entry_id = if let Some(entry_id) = entry_id {
            entry_id
        } else {
            match self
                .check_playlist_membership(playlist_id, track_id, cookie, auth_user)
                .await
            {
                PlaylistMembership::Present(Some(id)) => id,
                _ => return Err(
                    "Не удалось найти запись трека для удаления. Обнови плейлист в YouTube Music."
                        .into(),
                ),
            }
        };
        let action = serde_json::json!({
            "action": "ACTION_REMOVE_VIDEO", "removedVideoId": track_id, "setVideoId": entry_id
        });
        self.edit_playlist(playlist_id, action, cookie, auth_user)
            .await
    }

    async fn edit_playlist(
        &self,
        playlist_id: &str,
        action: Value,
        cookie: &str,
        auth_user: usize,
    ) -> Result<(), String> {
        let json = self
            .playlist_request(
                "browse/edit_playlist",
                serde_json::json!({
                    "playlistId": playlist_id,
                    "actions": [action]
                }),
                cookie,
                auth_user,
            )
            .await?;
        if json["status"].as_str() == Some("STATUS_SUCCEEDED") {
            Ok(())
        } else {
            Err(
                "YouTube Music не подтвердил изменение. Проверь плейлист перед повторной попыткой."
                    .into(),
            )
        }
    }

    pub async fn fetch_playlist_tracks(
        &self,
        browse_id: &str,
        cookie: &str,
        auth_user: usize,
    ) -> Result<Vec<Track>, String> {
        let mut tracks = Vec::new();
        self.scan_playlist(browse_id, cookie, auth_user, |item| {
            tracks.extend(Self::parse_item(item));
            false
        })
        .await?;
        Ok(tracks)
    }

    async fn scan_playlist(
        &self,
        browse_id: &str,
        cookie: &str,
        auth_user: usize,
        mut visit: impl FnMut(&Value) -> bool,
    ) -> Result<(), String> {
        let actual_id = if browse_id.starts_with("VL") || browse_id.starts_with("FE") {
            browse_id.to_string()
        } else {
            format!("VL{}", browse_id)
        };

        let request_page = |mut body: Value| async move {
            body["context"] = serde_json::json!({
                "client": {"clientName": "WEB_REMIX", "clientVersion": "1.20240101.01.00", "hl": "ru", "gl": "RU"}
            });
            let mut req = self.client
                .post(format!("https://music.youtube.com/youtubei/v1/browse?prettyPrint=false&authuser={auth_user}"))
                .timeout(std::time::Duration::from_secs(15))
                .header("Cookie", cookie)
                .header("X-YouTube-Client-Name", "67")
                .header("X-YouTube-Client-Version", "1.20240101.01.00")
                .header("X-Goog-AuthUser", auth_user.to_string())
                .json(&body);

            if let Some((auth, origin, referer)) = Self::get_sapisid_auth(cookie) {
                req = req
                    .header("Authorization", auth)
                    .header("Origin", origin)
                    .header("Referer", referer);
            }
            req.send()
                .await?
                .error_for_status()?
                .json::<Value>()
                .await
                .map_err(reqwest::Error::without_url)
        };

        let mut page = request_page(serde_json::json!({"browseId": actual_id}))
            .await
            .map_err(|e| e.without_url().to_string())?;
        if page.get("contents").is_none() && page.get("header").is_none() {
            return Err("YouTube Music не вернул содержимое плейлиста.".into());
        }
        let mut seen_tokens = std::collections::HashSet::new();
        loop {
            let (items, token) = Self::playlist_page(&page);
            if items.into_iter().any(&mut visit) {
                break;
            }
            let Some(token) = token else { break };
            if !seen_tokens.insert(token.clone()) {
                return Err(
                    "YouTube Music повторил страницу плейлиста. Попробуй обновить его.".into(),
                );
            }
            page = request_page(serde_json::json!({"continuation": token}))
                .await
                .map_err(|e| e.without_url().to_string())?;
            if page.pointer("/onResponseReceivedActions").is_none()
                && page
                    .pointer("/continuationContents/musicPlaylistShelfContinuation/contents")
                    .is_none()
            {
                return Err("YouTube Music не вернул следующую страницу плейлиста.".into());
            }
        }
        Ok(())
    }

    fn playlist_page(json: &Value) -> (Vec<&Value>, Option<String>) {
        fn find_shelf(node: &Value) -> Option<&Value> {
            match node {
                Value::Object(fields) => fields
                    .get("musicPlaylistShelfRenderer")
                    .or_else(|| fields.values().find_map(find_shelf)),
                Value::Array(items) => items.iter().find_map(find_shelf),
                _ => None,
            }
        }

        fn collect_tracks<'a>(node: &'a Value, tracks: &mut Vec<&'a Value>) {
            match node {
                Value::Object(fields) if fields.contains_key("musicResponsiveListItemRenderer") => {
                    tracks.push(node);
                }
                Value::Object(fields) => fields
                    .values()
                    .for_each(|child| collect_tracks(child, tracks)),
                Value::Array(items) => items.iter().for_each(|item| collect_tracks(item, tracks)),
                _ => {}
            }
        }

        let items = json["onResponseReceivedActions"]
            .as_array()
            .and_then(|actions| {
                actions.iter().find_map(|action| {
                    action.pointer("/appendContinuationItemsAction/continuationItems")
                })
            })
            .or_else(|| {
                json.pointer("/continuationContents/musicPlaylistShelfContinuation/contents")
            })
            .or_else(|| find_shelf(json).and_then(|shelf| shelf.get("contents")));
        let mut tracks = Vec::new();
        collect_tracks(items.unwrap_or(json), &mut tracks);
        let token = items
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().rev().find_map(|item| {
                    let endpoint =
                        item.pointer("/continuationItemRenderer/continuationEndpoint")?;
                    endpoint
                        .pointer("/continuationCommand/token")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            endpoint
                                .pointer("/commandExecutorCommand/commands")?
                                .as_array()?
                                .iter()
                                .find_map(|command| {
                                    let continuation = &command["continuationCommand"];
                                    (continuation["request"].as_str()
                                        == Some("CONTINUATION_REQUEST_TYPE_BROWSE"))
                                    .then(|| continuation["token"].as_str())
                                    .flatten()
                                })
                        })
                })
            })
            .map(str::to_owned);
        (tracks, token)
    }

    fn parse_item(item: &Value) -> Option<Track> {
        let r = item.get("musicResponsiveListItemRenderer")?;

        let flex = r.get("flexColumns")?.as_array()?;
        let col0 = flex.first()?;
        let runs0 = col0
            .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")?
            .as_array()?;

        let title = runs0
            .iter()
            .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");

        let mut artist = String::new();
        let mut duration = String::new();

        if flex.len() > 1 {
            if let Some(runs1) = flex[1]
                .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                .and_then(|v| v.as_array())
            {
                let parts: Vec<&str> = runs1
                    .iter()
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

        let video_id = Self::playlist_row_video_id(r)?.to_owned();

        let thumbnail = r
            .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(str::to_owned);

        Some(Track {
            id: video_id,
            title,
            artist: if artist.is_empty() {
                "Неизвестный исполнитель".to_string()
            } else {
                artist
            },
            duration_text: duration,
            thumbnail_url: thumbnail,
        })
    }

    fn parse_card(card: &Value) -> Option<Track> {
        let title = card
            .pointer("/title/runs/0/text")
            .and_then(|v| v.as_str())?
            .to_string();

        let artist = card
            .pointer("/subtitle/runs/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("Неизвестный")
            .to_string();

        let video_id = card
            .pointer("/buttons/0/buttonRenderer/command/watchEndpoint/videoId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                card.pointer("/title/runs/0/navigationEndpoint/watchEndpoint/videoId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })?;

        let thumbnail = card
            .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(str::to_owned);

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

    #[test]
    fn playlist_pages_keep_track_order_and_exclude_suggestions() {
        fn track(id: &str) -> Value {
            serde_json::json!({"musicResponsiveListItemRenderer": {
                "playlistItemData": {"videoId": id},
                "flexColumns": [{"musicResponsiveListItemFlexColumnRenderer": {
                    "text": {"runs": [{"text": id}]}
                }}]
            }})
        }
        let first_page = serde_json::json!({"contents": [
            {"musicShelfRenderer": {"contents": [track("suggested")]}},
            {"musicPlaylistShelfRenderer": {"contents": [
                track("existing"),
                {"continuationItemRenderer": {"continuationEndpoint": {
                    "continuationCommand": {"token": "page-two"}
                }}}
            ]}}
        ]});
        let continuation = serde_json::json!({"onResponseReceivedActions": [
            {"appendContinuationItemsAction": {"continuationItems": [track("newly-added")]}}
        ]});
        let (mut tracks, next) = InnertubeClient::playlist_page(&first_page);
        assert_eq!(next.as_deref(), Some("page-two"));
        let (more, end) = InnertubeClient::playlist_page(&continuation);
        tracks.extend(more);
        assert!(end.is_none());
        assert_eq!(
            tracks
                .iter()
                .filter_map(|row| InnertubeClient::parse_item(row).map(|track| track.id))
                .collect::<Vec<_>>(),
            ["existing", "newly-added"]
        );
    }

    #[test]
    fn playlist_picker_only_returns_playlist_options_and_normalizes_ids() {
        let response = serde_json::json!({"actions": [{"openPopupAction": {"popup": {
            "addToPlaylistRenderer": {"contents": [
                {"playlistAddToOptionRenderer": {
                    "title": {"runs": [{"text": "Road "}, {"text": "trip"}]},
                    "serviceEndpoint": {"playlistEditEndpoint": {"playlistId": "VLPLroadtrip"}},
                    "containsSelectedVideos": "ALL",
                    "removeFromPlaylistServiceEndpoint": {"playlistEditEndpoint": {"actions": [{
                        "action": "ACTION_REMOVE_VIDEO", "removedVideoId": "seed", "setVideoId": "entry-1"
                    }]}},
                    "subtitle": {"simpleText": "3 songs"},
                    "thumbnail": {"thumbnails": [{"url": "https://example.com/cover.jpg"}]}
                }},
                {"addToPlaylistItemRenderer": {
                    "title": {"simpleText": "Same playlist"},
                    "playlistId": "PLroadtrip"
                }},
                {"addToPlaylistItemRenderer": {
                    "title": {"simpleText": "Evening"},
                    "addToPlaylistEndpoint": {"playlistId": "PLevening"},
                    "containsSelectedVideos": "NONE",
                    "selected": true
                }},
                {"playlistId": "PLunrelated", "title": {"simpleText": "Not an option"}}
            ]}
        }}}]});
        let choices = InnertubeClient::parse_playlist_choices(&response, "seed");
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].playlist.id, "PLroadtrip");
        assert_eq!(choices[0].playlist.title, "Road trip");
        assert_eq!(choices[0].playlist.subtitle, "3 songs");
        assert_eq!(
            choices[0].playlist.thumbnail_url.as_deref(),
            Some("https://example.com/cover.jpg")
        );
        assert!(
            matches!(&choices[0].membership, PlaylistMembership::Present(Some(id)) if id == "entry-1")
        );
        assert_eq!(choices[1].playlist.id, "PLevening");
        assert!(matches!(choices[1].membership, PlaylistMembership::Unknown));
    }

    #[test]
    fn playlist_membership_can_be_found_on_a_later_page_without_display_metadata() {
        let response = serde_json::json!({"onResponseReceivedActions": [
            {"appendContinuationItemsAction": {"continuationItems": [
                {"musicResponsiveListItemRenderer": {"playlistItemData": {
                    "videoId": "saved-song", "playlistSetVideoId": "saved-entry"
                }}}
            ]}}
        ]});
        let (rows, _) = InnertubeClient::playlist_page(&response);
        assert!(InnertubeClient::playlist_row_membership(rows[0], "other-song").is_none());
        assert!(matches!(
            InnertubeClient::playlist_row_membership(rows[0], "saved-song"),
            Some(PlaylistMembership::Present(Some(id))) if id == "saved-entry"
        ));
    }

    #[test]
    fn radio_parser_handles_nested_tracks_duplicates_and_unavailable_items() {
        let track = serde_json::json!({"playlistPanelVideoRenderer": {
            "videoId": "sample-song",
            "title": {"runs": [{"text": "Sample "}, {"text": "song"}]},
            "shortBylineText": {"runs": [{"text": "Artist"}]},
            "lengthText": {"simpleText": "3:24"},
            "thumbnail": {"thumbnails": [
                {"url": "https://example.com/small.jpg"},
                {"url": "https://example.com/large.jpg"}
            ]}
        }});
        let response = serde_json::json!({"contents": [{"playlist": {
            "contents": [track.clone(), track, {
                "playlistPanelVideoRenderer": {
                    "videoId": "unavailable", "title": {"simpleText": "Hidden"}, "isPlayable": false
                }
            }]
        }}]});
        let parsed = InnertubeClient::parse_radio_tracks(&response);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Sample song");
        assert_eq!(parsed[0].artist, "Artist");
        assert_eq!(parsed[0].duration_text, "3:24");
        assert_eq!(
            parsed[0].thumbnail_url.as_deref(),
            Some("https://example.com/large.jpg")
        );
        assert!(InnertubeClient::parse_radio_tracks(&serde_json::json!({"error": {}})).is_empty());
    }

    #[tokio::test]
    #[ignore = "requires network and Firefox cookies"]
    async fn test_fetch_playlists() {
        let client = InnertubeClient::new();
        let cookie = BrowserAuth::extract_firefox_cookies().expect("Cookies from Firefox");
        let accounts = client.discover_accounts(&cookie).await;
        println!("Discovered accounts: {:?}", accounts);

        for authuser in [0, 1] {
            let res = client.fetch_user_playlists(&cookie, authuser).await;
            println!(
                "Fetch user playlists result (authuser={}): {:?}",
                authuser, res
            );
            assert!(res.is_ok());
        }
    }
}
