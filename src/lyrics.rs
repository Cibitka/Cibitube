use crate::models::{Lyrics, LyricsLine};
use serde_json::Value;

pub struct LyricsClient {
    client: reqwest::Client,
}

impl LyricsClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn clean_string(s: &str) -> String {
        let mut text = s.to_string();
        for marker in &[
            "(", "[", "feat.", "ft.", "prod.", "Official", "Video", "Audio",
        ] {
            if let Some(pos) = text.to_lowercase().find(&marker.to_lowercase()) {
                text = text[..pos].to_string();
            }
        }
        text.trim().to_string()
    }

    fn parse_synced(json: &Value) -> Option<Lyrics> {
        let synced = json.get("syncedLyrics").and_then(|v| v.as_str())?;
        let mut lines = Vec::new();
        for line in synced.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                if let Some(end_bracket) = trimmed.find(']') {
                    let time_str = &trimmed[1..end_bracket];
                    let text = trimmed[end_bracket + 1..].trim().to_string();
                    if !text.is_empty() {
                        if let Some(secs) = Self::parse_timestamp(time_str) {
                            lines.push(LyricsLine {
                                seconds: secs,
                                text,
                            });
                        }
                    }
                }
            }
        }
        if !lines.is_empty() {
            lines.sort_by(|a, b| {
                a.seconds
                    .partial_cmp(&b.seconds)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            return Some(Lyrics {
                is_synced: true,
                plain: json
                    .get("plainLyrics")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                lines,
            });
        }
        None
    }

    pub async fn fetch(&self, title: &str, artist: &str) -> Option<Lyrics> {
        let clean_t = Self::clean_string(title);
        let clean_a = Self::clean_string(artist);
        let search_title = if clean_t.is_empty() { title } else { &clean_t };
        let search_artist = if clean_a.is_empty() { artist } else { &clean_a };

        // 1. Попытка прямого запроса /api/get
        let get_url = format!(
            "https://lrclib.net/api/get?track_name={}&artist_name={}",
            urlencoding::encode(search_title),
            urlencoding::encode(search_artist),
        );

        let mut fallback_plain = None;

        if let Ok(res) = self
            .client
            .get(&get_url)
            .header(
                "User-Agent",
                "Metrolist-Desktop/0.1 (https://github.com/metrolistgroup/metrolist)",
            )
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(json) = res.json::<Value>().await {
                    if let Some(lyrics) = Self::parse_synced(&json) {
                        return Some(lyrics);
                    }
                    if let Some(plain) = json.get("plainLyrics").and_then(|v| v.as_str()) {
                        fallback_plain = Some(plain.to_string());
                    }
                }
            }
        }

        // 2. Fallback: поиск через /api/search при отсутствии точного совпадения
        let search_query = format!("{} {}", search_title, search_artist);
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(search_query.trim())
        );

        if let Ok(res) = self
            .client
            .get(&search_url)
            .header(
                "User-Agent",
                "Metrolist-Desktop/0.1 (https://github.com/metrolistgroup/metrolist)",
            )
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(items) = res.json::<Vec<Value>>().await {
                    // Ищем сначала синхронизированный трек
                    for item in &items {
                        if let Some(lyrics) = Self::parse_synced(item) {
                            return Some(lyrics);
                        }
                        if fallback_plain.is_none() {
                            if let Some(plain) = item.get("plainLyrics").and_then(|v| v.as_str()) {
                                fallback_plain = Some(plain.to_string());
                            }
                        }
                    }
                }
            }
        }

        // 3. Если синхронизированного нет, но есть обычный текст
        if let Some(plain) = fallback_plain {
            return Some(Lyrics {
                is_synced: false,
                plain,
                lines: Vec::new(),
            });
        }

        None
    }

    fn parse_timestamp(s: &str) -> Option<f64> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            let mins: f64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            Some(mins * 60.0 + secs)
        } else {
            None
        }
    }
}
