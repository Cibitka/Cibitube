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

    pub async fn fetch(&self, title: &str, artist: &str) -> Option<Lyrics> {
        let clean_title = title.split('(').next()?.split('[').next()?.trim();
        let clean_artist = artist.split('&').next()?.split(',').next()?.trim();

        let url = format!(
            "https://lrclib.net/api/get?track_name={}&artist_name={}",
            urlencoding::encode(clean_title),
            urlencoding::encode(clean_artist),
        );

        let res = self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "Metrolist-Desktop/0.1 (https://github.com/metrolistgroup/metrolist)",
            )
            .send()
            .await
            .ok()?;

        if !res.status().is_success() {
            return None;
        }

        let json: Value = res.json().await.ok()?;

        if let Some(synced) = json.get("syncedLyrics").and_then(|v| v.as_str()) {
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
        }

        if let Some(plain) = json.get("plainLyrics").and_then(|v| v.as_str()) {
            return Some(Lyrics {
                is_synced: false,
                plain: plain.to_string(),
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
