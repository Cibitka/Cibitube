use crate::models::{Lyrics, LyricsLine, LyricsSyncType, LyricsWord};
use serde_json::Value;
use sha1::Digest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::RwLock;

// ============================================================================
// 1. LRC Parser (Построчный и Расширенный LRC)
// ============================================================================

pub struct LrcParser;

impl LrcParser {
    pub fn parse(content: &str, source_name: &str) -> Option<Lyrics> {
        let mut lines = Vec::new();
        let mut title = None;
        let mut artist = None;
        let mut album = None;
        let mut offset_ms: i64 = 0;
        let mut has_word_tags = false;

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            // Extract multiple header timestamps: e.g. [00:10.00][00:20.00]Chorus line
            let mut timestamps = Vec::new();
            let mut rest = line;

            while rest.starts_with('[') {
                if let Some(close) = rest.find(']') {
                    let tag = &rest[1..close];
                    if let Some(stripped) = tag.strip_prefix("offset:") {
                        if let Ok(off) = stripped.trim().parse::<i64>() {
                            offset_ms = off;
                        }
                        rest = rest[close + 1..].trim_start();
                        continue;
                    }
                    if let Some(stripped) = tag.strip_prefix("ti:") {
                        title = Some(stripped.trim().to_string());
                        rest = rest[close + 1..].trim_start();
                        continue;
                    }
                    if let Some(stripped) = tag.strip_prefix("ar:") {
                        artist = Some(stripped.trim().to_string());
                        rest = rest[close + 1..].trim_start();
                        continue;
                    }
                    if let Some(stripped) = tag.strip_prefix("al:") {
                        album = Some(stripped.trim().to_string());
                        rest = rest[close + 1..].trim_start();
                        continue;
                    }
                    if let Some(ts) = Self::parse_timestamp(tag) {
                        timestamps.push(ts);
                        rest = rest[close + 1..].trim_start();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if timestamps.is_empty() {
                continue;
            }

            let text_part = rest.trim();
            if text_part.is_empty() {
                continue;
            }

            let base_ts = timestamps[0];
            let (cleaned_text, base_words) = Self::parse_enhanced_words(text_part, base_ts);
            if base_words.is_some() {
                has_word_tags = true;
            }

            for &ts in &timestamps {
                let words_for_line = if let Some(ref bw) = base_words {
                    if ts == base_ts {
                        Some(bw.clone())
                    } else {
                        let shift = ts as i64 - base_ts as i64;
                        let shifted = bw
                            .iter()
                            .map(|w| LyricsWord {
                                start_ms: (w.start_ms as i64 + shift).max(0) as u64,
                                end_ms: (w.end_ms as i64 + shift).max(0) as u64,
                                text: w.text.clone(),
                            })
                            .collect();
                        Some(shifted)
                    }
                } else {
                    None
                };

                lines.push(LyricsLine {
                    seconds: ts as f64 / 1000.0,
                    start_ms: ts,
                    end_ms: None,
                    text: cleaned_text.clone(),
                    words: words_for_line,
                    translation: None,
                    romanization: None,
                });
            }
        }

        if lines.is_empty() {
            return None;
        }

        lines.sort_by_key(|l| l.start_ms);

        // Fill end_ms from next line's start_ms and bound word timings
        for i in 0..lines.len() {
            if i + 1 < lines.len() {
                let next_start = lines[i + 1].start_ms;
                lines[i].end_ms = Some(next_start);
                if let Some(ref mut words) = lines[i].words {
                    if let Some(last_word) = words.last_mut() {
                        if last_word.end_ms <= last_word.start_ms || last_word.end_ms > next_start {
                            last_word.end_ms = next_start.max(last_word.start_ms);
                        }
                    }
                }
            }
        }

        let sync_type = if has_word_tags {
            LyricsSyncType::Word
        } else {
            LyricsSyncType::Line
        };

        let plain_lines: Vec<String> = lines.iter().map(|l| l.text.clone()).collect();

        Some(Lyrics {
            source_name: source_name.to_string(),
            provider: None,
            title,
            artist,
            album,
            duration_ms: lines.last().and_then(|l| l.end_ms),
            sync_type,
            is_synced: true,
            plain: plain_lines.join("\n"),
            lines,
            offset_ms,
        })
    }

    pub fn parse_timestamp(s: &str) -> Option<u64> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            let mins: f64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            Some(((mins * 60.0 + secs) * 1000.0).round() as u64)
        } else if parts.len() == 3 {
            let hrs: f64 = parts[0].parse().ok()?;
            let mins: f64 = parts[1].parse().ok()?;
            let secs: f64 = parts[2].parse().ok()?;
            Some((((hrs * 60.0 + mins) * 60.0 + secs) * 1000.0).round() as u64)
        } else {
            None
        }
    }

    fn parse_enhanced_words(text: &str, line_start_ms: u64) -> (String, Option<Vec<LyricsWord>>) {
        if !text.contains('<') && !text.contains('(') {
            return (text.to_string(), None);
        }

        let mut words = Vec::new();
        let mut clean_text = String::new();
        let mut chars = text.chars().peekable();
        let mut cur_word = String::new();
        let mut cur_start = line_start_ms;

        while let Some(c) = chars.next() {
            if c == '<' || c == '(' {
                let closing = if c == '<' { '>' } else { ')' };
                let mut tag = String::new();
                let mut found_closing = false;
                while let Some(&inner) = chars.peek() {
                    chars.next();
                    if inner == closing {
                        found_closing = true;
                        break;
                    }
                    tag.push(inner);
                }

                if found_closing {
                    if let Some(ts) = Self::parse_timestamp(&tag) {
                        if !cur_word.is_empty() {
                            let trimmed = cur_word.trim();
                            if !trimmed.is_empty() {
                                words.push(LyricsWord {
                                    start_ms: cur_start,
                                    end_ms: ts.max(cur_start),
                                    text: cur_word.clone(),
                                });
                            }
                            clean_text.push_str(&cur_word);
                            cur_word.clear();
                        }
                        cur_start = ts;
                        continue;
                    }
                }

                // Regular parenthesis or bracket without timestamp: preserve it!
                cur_word.push(c);
                cur_word.push_str(&tag);
                if found_closing {
                    cur_word.push(closing);
                }
            } else {
                cur_word.push(c);
            }
        }

        if !cur_word.is_empty() {
            clean_text.push_str(&cur_word);
            let trimmed = cur_word.trim();
            if !trimmed.is_empty() {
                let end_ms = if let Some(last_prev) = words.last() {
                    let prev_dur = last_prev.end_ms.saturating_sub(last_prev.start_ms);
                    cur_start + prev_dur.max(100)
                } else {
                    cur_start
                };
                words.push(LyricsWord {
                    start_ms: cur_start,
                    end_ms,
                    text: cur_word,
                });
            }
        }

        if words.len() >= 2 {
            (clean_text, Some(words))
        } else {
            (text.to_string(), None)
        }
    }
}

// ============================================================================
// 2. QRC Parser (QQ Music Karaoke / Word-level)
// ============================================================================

pub struct QrcParser;

impl QrcParser {
    pub fn parse(raw: &str, provider: Option<&str>) -> Option<Lyrics> {
        let content = if let Some(idx) = raw.find("LyricContent=\"") {
            let start = idx + 14;
            let end = raw[start..].find('"')?;
            Self::unescape_xml(&raw[start..start + end])
        } else {
            raw.to_string()
        };

        let mut lines = Vec::new();
        let mut plain_lines = Vec::new();
        let mut title = None;
        let mut artist = None;
        let mut album = None;
        let mut offset_ms: i64 = 0;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("[ti:") && trimmed.ends_with(']') {
                title = Some(trimmed[4..trimmed.len() - 1].trim().to_string());
                continue;
            }
            if trimmed.starts_with("[ar:") && trimmed.ends_with(']') {
                artist = Some(trimmed[4..trimmed.len() - 1].trim().to_string());
                continue;
            }
            if trimmed.starts_with("[al:") && trimmed.ends_with(']') {
                album = Some(trimmed[4..trimmed.len() - 1].trim().to_string());
                continue;
            }
            if trimmed.starts_with("[offset:") && trimmed.ends_with(']') {
                if let Ok(off) = trimmed[8..trimmed.len() - 1].trim().parse::<i64>() {
                    offset_ms = off;
                }
                continue;
            }

            // QRC line pattern: [start_ms,dur_ms]word(wstart_ms,wdur_ms)...
            if trimmed.starts_with('[') {
                if let Some(close_bracket) = trimmed.find(']') {
                    let header = &trimmed[1..close_bracket];
                    let comma_pos = header.find(',');
                    let (line_start_ms, line_dur_ms) = if let Some(cp) = comma_pos {
                        let s: u64 = header[..cp].parse().unwrap_or(0);
                        let d: u64 = header[cp + 1..].parse().unwrap_or(0);
                        (s, d)
                    } else if let Some(ts) = LrcParser::parse_timestamp(header) {
                        (ts, 3000)
                    } else {
                        continue;
                    };

                    let body = &trimmed[close_bracket + 1..];
                    let words = Self::parse_qrc_words(body, line_start_ms);
                    let full_text = if !words.is_empty() {
                        words.iter().map(|w| w.text.as_str()).collect::<String>()
                    } else {
                        body.to_string()
                    };

                    let clean_text = full_text.trim().to_string();
                    if !clean_text.is_empty() {
                        plain_lines.push(clean_text.clone());
                        let end_ms = if line_dur_ms > 0 {
                            Some(line_start_ms + line_dur_ms)
                        } else {
                            None
                        };
                        let words_opt = if !words.is_empty() { Some(words) } else { None };
                        lines.push(LyricsLine {
                            seconds: line_start_ms as f64 / 1000.0,
                            start_ms: line_start_ms,
                            end_ms,
                            text: clean_text,
                            words: words_opt,
                            translation: None,
                            romanization: None,
                        });
                    }
                }
            }
        }

        if lines.is_empty() {
            return None;
        }

        lines.sort_by_key(|l| l.start_ms);

        Some(Lyrics {
            source_name: "Better Lyrics (QQ)".to_string(),
            provider: provider.map(String::from).or(Some("qq".to_string())),
            title,
            artist,
            album,
            duration_ms: lines.last().and_then(|l| l.end_ms),
            sync_type: LyricsSyncType::Word,
            is_synced: true,
            plain: plain_lines.join("\n"),
            lines,
            offset_ms,
        })
    }

    fn parse_qrc_words(body: &str, default_start: u64) -> Vec<LyricsWord> {
        let mut words = Vec::new();
        let mut cur_text = String::new();
        let mut chars = body.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '(' {
                let mut tag = String::new();
                let mut found_closing = false;
                while let Some(&inner) = chars.peek() {
                    chars.next();
                    if inner == ')' {
                        found_closing = true;
                        break;
                    }
                    tag.push(inner);
                }
                if found_closing {
                    if let Some(comma) = tag.find(',') {
                        if let (Ok(w_start), Ok(w_dur)) = (
                            tag[..comma].trim().parse::<u64>(),
                            tag[comma + 1..].trim().parse::<u64>(),
                        ) {
                            if !cur_text.is_empty() {
                                words.push(LyricsWord {
                                    start_ms: w_start,
                                    end_ms: w_start + w_dur,
                                    text: cur_text.clone(),
                                });
                                cur_text.clear();
                            }
                            continue;
                        }
                    }
                }
                cur_text.push(c);
                cur_text.push_str(&tag);
                if found_closing {
                    cur_text.push(')');
                }
            } else {
                cur_text.push(c);
            }
        }

        if !cur_text.is_empty() {
            words.push(LyricsWord {
                start_ms: default_start,
                end_ms: default_start,
                text: cur_text,
            });
        }
        words
    }

    fn unescape_xml(s: &str) -> String {
        s.replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&apos;", "'")
            .replace("&#10;", "\n")
            .replace("&#13;", "")
    }
}

// ============================================================================
// 3. TTML Parser (Timed Text Markup Language / Apple Music style)
// ============================================================================

pub struct TtmlParser;

impl TtmlParser {
    pub fn parse(raw: &str, provider: Option<&str>) -> Option<Lyrics> {
        let doc = match roxmltree::Document::parse(raw) {
            Ok(d) => d,
            Err(_) => return None,
        };

        let mut lines = Vec::new();
        let mut plain_lines = Vec::new();
        let mut has_syllables = false;
        let mut has_words = false;

        for p_node in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("p"))
        {
            let line_begin_ms = Self::extract_time_attr(&p_node, "begin").unwrap_or(0);
            let line_end_ms = Self::extract_time_attr(&p_node, "end").or_else(|| {
                let dur = Self::extract_time_attr(&p_node, "dur")?;
                Some(line_begin_ms + dur)
            });

            // Extract timed words/spans
            let timed_segments = Self::extract_timed_spans(&p_node, line_begin_ms, line_end_ms);

            // Extract full line text preserving punctuation and spacing
            let full_text = Self::extract_p_text(&p_node);
            let trimmed = full_text.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }

            plain_lines.push(trimmed.clone());

            let words_opt = if !timed_segments.is_empty() {
                let word_count = trimmed.split_whitespace().count();
                let syllable = timed_segments.len() > word_count
                    || Self::has_subword_segments(&timed_segments, &trimmed);
                if syllable {
                    has_syllables = true;
                } else {
                    has_words = true;
                }
                Some(timed_segments)
            } else {
                None
            };

            lines.push(LyricsLine {
                seconds: line_begin_ms as f64 / 1000.0,
                start_ms: line_begin_ms,
                end_ms: line_end_ms,
                text: trimmed,
                words: words_opt,
                translation: None,
                romanization: None,
            });
        }

        if lines.is_empty() {
            return None;
        }

        lines.sort_by_key(|l| l.start_ms);

        // Fill missing end_ms from next line's start_ms and bound word timings
        for i in 0..lines.len() {
            if i + 1 < lines.len() {
                let next_start = lines[i + 1].start_ms;
                if lines[i].end_ms.is_none() {
                    lines[i].end_ms = Some(next_start);
                }
                if let Some(ref mut words) = lines[i].words {
                    if let Some(last_word) = words.last_mut() {
                        if last_word.end_ms <= last_word.start_ms || last_word.end_ms > next_start {
                            last_word.end_ms = next_start.max(last_word.start_ms);
                        }
                    }
                }
            }
        }

        let sync_type = if has_syllables {
            LyricsSyncType::Syllable
        } else if has_words {
            LyricsSyncType::Word
        } else {
            LyricsSyncType::Line
        };

        Some(Lyrics {
            source_name: "Better Lyrics (TTML)".to_string(),
            provider: provider.map(String::from).or(Some("ttml".to_string())),
            title: None,
            artist: None,
            album: None,
            duration_ms: lines.last().and_then(|l| l.end_ms),
            sync_type,
            is_synced: true,
            plain: plain_lines.join("\n"),
            lines,
            offset_ms: 0,
        })
    }

    fn extract_time_attr(node: &roxmltree::Node, attr_name: &str) -> Option<u64> {
        for attr in node.attributes() {
            if attr.name().eq_ignore_ascii_case(attr_name) {
                return Self::parse_ttml_time(attr.value());
            }
        }
        None
    }

    fn extract_p_text(p_node: &roxmltree::Node) -> String {
        let mut raw = String::new();
        for d in p_node.descendants() {
            if d.is_text() {
                if let Some(t) = d.text() {
                    raw.push_str(t);
                }
            }
        }
        Self::normalize_whitespace(&raw)
    }

    fn normalize_whitespace(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut last_was_space = false;
        for c in s.chars() {
            if c.is_whitespace() {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            } else {
                out.push(c);
                last_was_space = false;
            }
        }
        out.trim().to_string()
    }

    fn extract_timed_spans(
        p_node: &roxmltree::Node,
        _line_begin: u64,
        line_end: Option<u64>,
    ) -> Vec<LyricsWord> {
        let mut raw_spans: Vec<(u64, Option<u64>, String)> = Vec::new();

        for span in p_node
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("span"))
        {
            if let Some(begin) = Self::extract_time_attr(&span, "begin") {
                let has_timed_children = span.descendants().any(|child| {
                    child.is_element()
                        && child != span
                        && Self::extract_time_attr(&child, "begin").is_some()
                });
                if !has_timed_children {
                    let end = Self::extract_time_attr(&span, "end").or_else(|| {
                        let dur = Self::extract_time_attr(&span, "dur")?;
                        Some(begin + dur)
                    });
                    let mut text = String::new();
                    for d in span.descendants() {
                        if d.is_text() {
                            if let Some(t) = d.text() {
                                text.push_str(t);
                            }
                        }
                    }
                    let clean = Self::normalize_whitespace(&text);
                    if !clean.is_empty() {
                        raw_spans.push((begin, end, clean));
                    }
                }
            }
        }

        if raw_spans.is_empty() {
            return Vec::new();
        }

        let mut words = Vec::new();
        let total = raw_spans.len();
        for i in 0..total {
            let (begin, explicit_end, text) = &raw_spans[i];
            let end = match explicit_end {
                Some(e) => *e,
                None => {
                    if i + 1 < total && raw_spans[i + 1].0 >= *begin {
                        raw_spans[i + 1].0
                    } else if let Some(le) = line_end {
                        le.max(*begin)
                    } else {
                        *begin
                    }
                }
            };
            words.push(LyricsWord {
                start_ms: *begin,
                end_ms: end.max(*begin),
                text: text.clone(),
            });
        }

        words
    }

    fn has_subword_segments(segments: &[LyricsWord], full_text: &str) -> bool {
        let words: Vec<&str> = full_text.split_whitespace().collect();
        if segments.len() > words.len() {
            return true;
        }
        for seg in segments {
            let st = seg.text.trim();
            if !st.is_empty() && !words.contains(&st) {
                return true;
            }
        }
        false
    }

    pub fn parse_ttml_time(s: &str) -> Option<u64> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        // Offset-time format
        if let Some(ms_str) = s.strip_suffix("ms") {
            return ms_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|ms| ms.round().max(0.0) as u64);
        }
        if let Some(s_str) = s.strip_suffix('s') {
            return s_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|secs| (secs * 1000.0).round().max(0.0) as u64);
        }
        if let Some(m_str) = s.strip_suffix('m') {
            return m_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|mins| (mins * 60.0 * 1000.0).round().max(0.0) as u64);
        }
        if let Some(h_str) = s.strip_suffix('h') {
            return h_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|hrs| (hrs * 3600.0 * 1000.0).round().max(0.0) as u64);
        }

        // Clock-time format: [hh:]mm:ss[.frac]
        if s.contains(':') {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() == 2 {
                let mins: f64 = parts[0].trim().parse().ok()?;
                let secs: f64 = parts[1].trim().parse().ok()?;
                return Some(((mins * 60.0 + secs) * 1000.0).round().max(0.0) as u64);
            } else if parts.len() == 3 {
                let hrs: f64 = parts[0].trim().parse().ok()?;
                let mins: f64 = parts[1].trim().parse().ok()?;
                let secs: f64 = parts[2].trim().parse().ok()?;
                return Some(
                    (((hrs * 3600.0) + (mins * 60.0) + secs) * 1000.0)
                        .round()
                        .max(0.0) as u64,
                );
            } else {
                return None;
            }
        }

        // Bare float/int seconds
        if let Ok(secs) = s.parse::<f64>() {
            return Some((secs * 1000.0).round().max(0.0) as u64);
        }

        None
    }
}

// ============================================================================
// 4. LrcMux Adapter (https://lrcmux.dev/docs & api.lrcmux.dev)
// ============================================================================

pub struct LrcMuxAdapter;

impl LrcMuxAdapter {
    pub async fn fetch(
        client: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        let mut url = format!(
            "https://api.lrcmux.dev/get?artist={}&title={}&level=word&format=json",
            urlencoding::encode(artist),
            urlencoding::encode(title),
        );
        if let Some(alb) = album {
            if !alb.trim().is_empty() {
                url.push_str(&format!("&album={}", urlencoding::encode(alb.trim())));
            }
        }
        if let Some(d) = duration_secs {
            if d > 0 {
                url.push_str(&format!("&duration={}", d));
            }
        }

        let resp = client
            .get(&url)
            .header(
                "User-Agent",
                "Cibitube/0.2.9 (https://github.com/Cibitka/Cibitube)",
            )
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let json: Value = resp.json().await.ok()?;
        Self::parse_json(&json)
    }

    fn parse_json(json: &Value) -> Option<Lyrics> {
        let lines_val = json.get("lines")?.as_array()?;
        let meta = json.get("meta");
        let provider = meta
            .and_then(|m| m.get("source"))
            .and_then(|s| s.get("name").or_else(|| s.get("id")))
            .and_then(|v| v.as_str())
            .map(String::from);

        let level = meta
            .and_then(|m| m.get("level"))
            .and_then(|l| l.as_str())
            .unwrap_or("line");

        let sync_type = match level {
            "word" => LyricsSyncType::Word,
            "line" => LyricsSyncType::Line,
            _ => LyricsSyncType::None,
        };

        let mut lines = Vec::new();
        let mut plain_lines = Vec::new();

        for l in lines_val {
            let text = l
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            plain_lines.push(text.clone());
            let start_ms = l.get("start").and_then(|s| s.as_u64()).unwrap_or(0);
            let end_ms = l.get("end").and_then(|e| e.as_u64());

            let words = l.get("words").and_then(|w| w.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|w_obj| {
                        let w_text = w_obj.get("text")?.as_str()?.to_string();
                        let w_start = w_obj.get("start")?.as_u64()?;
                        let w_end = w_obj.get("end")?.as_u64()?;
                        Some(LyricsWord {
                            start_ms: w_start,
                            end_ms: w_end,
                            text: w_text,
                        })
                    })
                    .collect::<Vec<_>>()
            });

            lines.push(LyricsLine {
                seconds: start_ms as f64 / 1000.0,
                start_ms,
                end_ms,
                text,
                words: words.filter(|w| !w.is_empty()),
                translation: None,
                romanization: None,
            });
        }

        if lines.is_empty() {
            return None;
        }

        Some(Lyrics {
            source_name: format!("LrcMux ({})", provider.as_deref().unwrap_or("auto")),
            provider,
            title: json
                .get("track")
                .and_then(|t| t.get("title"))
                .and_then(|v| v.as_str())
                .map(String::from),
            artist: json
                .get("track")
                .and_then(|t| t.get("artist"))
                .and_then(|v| v.as_str())
                .map(String::from),
            album: json
                .get("track")
                .and_then(|t| t.get("album"))
                .and_then(|v| v.as_str())
                .map(String::from),
            duration_ms: json
                .get("track")
                .and_then(|t| t.get("duration"))
                .and_then(|v| v.as_u64())
                .map(|d| d * 1000),
            sync_type,
            is_synced: sync_type != LyricsSyncType::None,
            plain: plain_lines.join("\n"),
            lines,
            offset_ms: 0,
        })
    }
}

// ============================================================================
// 5. Better Lyrics Adapter (QQ, KuGou, TTML)
// ============================================================================

pub struct BetterLyricsAdapter;

impl BetterLyricsAdapter {
    pub async fn fetch_qq(
        client: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        Self::fetch_endpoint(client, "qq", title, artist, album, duration_secs).await
    }

    pub async fn fetch_kugou(
        client: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        Self::fetch_endpoint(client, "kugou", title, artist, album, duration_secs).await
    }

    pub async fn fetch_ttml(
        client: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        Self::fetch_endpoint(client, "ttml", title, artist, album, duration_secs).await
    }

    async fn fetch_endpoint(
        client: &reqwest::Client,
        endpoint: &str,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        let dur = duration_secs.unwrap_or(0);
        let alb = album.unwrap_or("");
        let url = format!(
            "https://api.betterlyrics.org/{}/getLyrics?s={}&a={}&al={}&d={}",
            endpoint,
            urlencoding::encode(title),
            urlencoding::encode(artist),
            urlencoding::encode(alb),
            dur
        );

        let resp = client
            .get(&url)
            .header(
                "User-Agent",
                "Cibitube/0.2.9 (https://github.com/Cibitka/Cibitube)",
            )
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;

        // If 401 on TTML or other endpoints: silently skip without asking for payment
        if resp.status().as_u16() == 401 {
            return None;
        }

        if !resp.status().is_success() {
            return None;
        }

        let json: Value = resp.json().await.ok()?;
        let lyrics_str = json.get("lyrics")?.as_str()?;
        let provider = json.get("provider").and_then(|p| p.as_str());

        match endpoint {
            "qq" => QrcParser::parse(lyrics_str, provider),
            "ttml" => TtmlParser::parse(lyrics_str, provider),
            "kugou" => {
                let mut res = LrcParser::parse(lyrics_str, "Better Lyrics (KuGou)")?;
                res.provider = provider.map(String::from).or(Some("kugou".to_string()));
                Some(res)
            }
            _ => None,
        }
    }
}

// ============================================================================
// 6. LRCLIB Adapter (https://lrclib.net)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackVersionMeta {
    pub is_remix: bool,
    pub is_live: bool,
    pub is_cover: bool,
    pub is_acoustic: bool,
    pub is_instrumental: bool,
}

impl TrackVersionMeta {
    pub fn from_title(title: &str) -> Self {
        let lower = title.to_lowercase();
        Self {
            is_remix: lower.contains("remix")
                || lower.contains(" club mix")
                || lower.contains(" dance mix"),
            is_live: lower.contains("live"),
            is_cover: lower.contains("cover"),
            is_acoustic: lower.contains("acoustic") || lower.contains("unplugged"),
            is_instrumental: lower.contains("instrumental") || lower.contains("karaoke version"),
        }
    }

    pub fn matches(&self, other: &Self) -> bool {
        self.is_remix == other.is_remix
            && self.is_live == other.is_live
            && self.is_cover == other.is_cover
            && self.is_acoustic == other.is_acoustic
            && self.is_instrumental == other.is_instrumental
    }
}

pub struct LrclibAdapter;

impl LrclibAdapter {
    pub async fn fetch(
        client: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        let clean_t = Self::clean_title(title);
        let clean_a = Self::clean_artist(artist);
        let search_title = if clean_t.is_empty() { title } else { &clean_t };
        let search_artist = if clean_a.is_empty() { artist } else { &clean_a };

        // 1. Direct GET /api/get
        let mut get_url = format!(
            "https://lrclib.net/api/get?track_name={}&artist_name={}",
            urlencoding::encode(search_title),
            urlencoding::encode(search_artist),
        );
        if let Some(alb) = album {
            if !alb.trim().is_empty() {
                get_url.push_str(&format!("&album_name={}", urlencoding::encode(alb.trim())));
            }
        }
        if let Some(d) = duration_secs {
            if d > 0 {
                get_url.push_str(&format!("&duration={}", d));
            }
        }

        if let Ok(res) = client
            .get(&get_url)
            .header(
                "User-Agent",
                "Cibitube/0.2.9 (https://github.com/Cibitka/Cibitube)",
            )
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(json) = res.json::<Value>().await {
                    if Self::is_match_eligible(&json, title, artist, duration_secs) {
                        if let Some(lyrics) = Self::parse_lrclib_json(&json) {
                            return Some(lyrics);
                        }
                    }
                }
            }
        }

        // 2. Fallback /api/search
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(&format!("{} {}", search_title, search_artist))
        );

        if let Ok(res) = client
            .get(&search_url)
            .header(
                "User-Agent",
                "Cibitube/0.2.9 (https://github.com/Cibitka/Cibitube)",
            )
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(items) = res.json::<Vec<Value>>().await {
                    let best = Self::select_best_match(items, title, artist, album, duration_secs);
                    if let Some(item) = best {
                        if let Some(lyrics) = Self::parse_lrclib_json(&item) {
                            return Some(lyrics);
                        }
                    }
                }
            }
        }

        None
    }

    pub fn clean_title(s: &str) -> String {
        let text = s.trim();
        let junk_patterns = [
            "official music video",
            "official video",
            "music video",
            "official audio",
            "lyric video",
            "official lyric video",
            "visualizer",
            "official visualizer",
            "video",
            "audio",
            "4k",
            "hd",
            "mv",
            "m/v",
            "clip officiel",
            "video clip",
        ];

        let mut cleaned = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '(' || c == '[' {
                let closing = if c == '(' { ')' } else { ']' };
                let mut inner = String::new();
                let mut found_closing = false;
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch == closing {
                        found_closing = true;
                        break;
                    }
                    inner.push(ch);
                }
                let inner_lower = inner.to_lowercase();
                let is_junk = junk_patterns
                    .iter()
                    .any(|&p| inner_lower == p || inner_lower.starts_with(p));
                let is_feat = inner_lower.starts_with("feat")
                    || inner_lower.starts_with("ft.")
                    || inner_lower.starts_with("prod.");
                if is_junk || is_feat {
                    continue;
                }
                cleaned.push(c);
                cleaned.push_str(&inner);
                if found_closing {
                    cleaned.push(closing);
                }
            } else {
                cleaned.push(c);
            }
        }

        let lower = cleaned.to_lowercase();
        for marker in &["feat.", "ft.", "prod."] {
            if let Some(pos) = lower.find(marker) {
                if pos > 0 {
                    cleaned = cleaned[..pos].to_string();
                }
            }
        }

        cleaned.trim().trim_end_matches('-').trim().to_string()
    }

    pub fn clean_artist(s: &str) -> String {
        let mut artist = s.trim().to_string();
        let lower = artist.to_lowercase();
        for marker in &["feat.", "ft.", "feat", "prod.", " - Topic"] {
            if let Some(pos) = lower.find(marker) {
                if pos > 0 {
                    artist = artist[..pos].to_string();
                }
            }
        }
        artist.trim().trim_end_matches('-').trim().to_string()
    }

    pub fn normalize_for_comparison(s: &str) -> String {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn artist_matches(norm_target: &str, norm_cand: &str) -> bool {
        if norm_target.is_empty() || norm_cand.is_empty() {
            return false;
        }
        if norm_target == norm_cand {
            return true;
        }
        if norm_target.contains(norm_cand) || norm_cand.contains(norm_target) {
            return true;
        }
        let target_tokens: Vec<&str> = norm_target.split_whitespace().collect();
        let cand_tokens: Vec<&str> = norm_cand.split_whitespace().collect();
        for tt in &target_tokens {
            if tt.len() >= 3 && cand_tokens.contains(tt) {
                return true;
            }
        }
        false
    }

    pub fn title_matches(norm_target: &str, norm_cand: &str) -> bool {
        if norm_target.is_empty() || norm_cand.is_empty() {
            return false;
        }
        if norm_target == norm_cand {
            return true;
        }
        if norm_target.contains(norm_cand) || norm_cand.contains(norm_target) {
            return true;
        }
        let target_words: std::collections::HashSet<&str> =
            norm_target.split_whitespace().collect();
        let cand_words: std::collections::HashSet<&str> = norm_cand.split_whitespace().collect();
        let common = target_words.intersection(&cand_words).count();
        let total = target_words.len().min(cand_words.len());
        if total > 0 && (common as f64 / total as f64) >= 0.6 {
            return true;
        }
        false
    }

    pub fn is_match_eligible(
        item: &Value,
        target_title: &str,
        target_artist: &str,
        target_duration: Option<u64>,
    ) -> bool {
        let item_title = item.get("trackName").and_then(|v| v.as_str()).unwrap_or("");
        let item_artist = item
            .get("artistName")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let target_ver = TrackVersionMeta::from_title(target_title);
        let item_ver = TrackVersionMeta::from_title(item_title);
        if !target_ver.matches(&item_ver) {
            return false;
        }

        let clean_t = Self::clean_title(target_title);
        let clean_a = Self::clean_artist(target_artist);
        let norm_t = Self::normalize_for_comparison(&clean_t);
        let norm_a = Self::normalize_for_comparison(&clean_a);

        let clean_cand_a = Self::clean_artist(item_artist);
        let norm_cand_a = Self::normalize_for_comparison(&clean_cand_a);
        if !Self::artist_matches(&norm_a, &norm_cand_a) {
            return false;
        }

        let clean_cand_t = Self::clean_title(item_title);
        let norm_cand_t = Self::normalize_for_comparison(&clean_cand_t);
        if !Self::title_matches(&norm_t, &norm_cand_t) {
            return false;
        }

        if let (Some(td), Some(cd)) = (
            target_duration,
            item.get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d.round() as u64),
        ) {
            if (td as i64 - cd as i64).abs() > 15 {
                return false;
            }
        }

        let has_synced = item
            .get("syncedLyrics")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_plain = item
            .get("plainLyrics")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        has_synced || has_plain
    }

    pub fn select_best_match(
        items: Vec<Value>,
        target_title: &str,
        target_artist: &str,
        target_album: Option<&str>,
        target_duration: Option<u64>,
    ) -> Option<Value> {
        let clean_t = Self::clean_title(target_title);
        let clean_a = Self::clean_artist(target_artist);
        let norm_t = Self::normalize_for_comparison(&clean_t);
        let norm_a = Self::normalize_for_comparison(&clean_a);
        let target_ver = TrackVersionMeta::from_title(target_title);

        let mut eligible_items = Vec::new();

        // Stage 1: Filter out ineligible candidates
        for item in items {
            let item_title = item.get("trackName").and_then(|v| v.as_str()).unwrap_or("");
            let item_artist = item
                .get("artistName")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // 1. Version check: do not mix original with remix, live, cover, acoustic
            let item_ver = TrackVersionMeta::from_title(item_title);
            if !target_ver.matches(&item_ver) {
                continue;
            }

            // 2. Artist check: artist must match
            let clean_cand_a = Self::clean_artist(item_artist);
            let norm_cand_a = Self::normalize_for_comparison(&clean_cand_a);
            if !Self::artist_matches(&norm_a, &norm_cand_a) {
                continue;
            }

            // 3. Title check: title must match
            let clean_cand_t = Self::clean_title(item_title);
            let norm_cand_t = Self::normalize_for_comparison(&clean_cand_t);
            if !Self::title_matches(&norm_t, &norm_cand_t) {
                continue;
            }

            // 4. Duration check: candidate must be within 15 seconds if target duration is known
            let item_duration = item
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d.round() as u64);
            let mut dur_diff = None;
            if let (Some(td), Some(cd)) = (target_duration, item_duration) {
                let diff = (td as i64 - cd as i64).abs();
                if diff > 15 {
                    continue; // Incompatible duration!
                }
                dur_diff = Some(diff);
            }

            // 5. Must have lyrics
            let has_synced = item
                .get("syncedLyrics")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let has_plain = item
                .get("plainLyrics")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_synced && !has_plain {
                continue;
            }

            eligible_items.push((item, has_synced, dur_diff, norm_cand_t, norm_cand_a));
        }

        if eligible_items.is_empty() {
            return None;
        }

        // Stage 2: Score eligible candidates
        let mut scored_items = Vec::new();
        for (item, has_synced, dur_diff, cand_t, cand_a) in eligible_items {
            let mut score = 0i32;

            // Prefer detailed sync among verified matching candidates
            if has_synced {
                score += 500;
            } else {
                score += 100;
            }

            // Duration closeness
            if let Some(diff) = dur_diff {
                if diff <= 2 {
                    score += 150;
                } else if diff <= 5 {
                    score += 80;
                } else if diff <= 10 {
                    score += 30;
                }
            }

            // Exact title match bonus
            if norm_t == cand_t {
                score += 100;
            }

            // Exact artist match bonus
            if norm_a == cand_a {
                score += 100;
            }

            // Album bonus
            if let Some(target_alb) = target_album {
                if let Some(cand_alb) = item.get("albumName").and_then(|v| v.as_str()) {
                    if Self::normalize_for_comparison(target_alb)
                        == Self::normalize_for_comparison(cand_alb)
                    {
                        score += 50;
                    }
                }
            }

            scored_items.push((score, item));
        }

        scored_items.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored_items.into_iter().next().map(|(_, item)| item)
    }

    fn parse_lrclib_json(json: &Value) -> Option<Lyrics> {
        if let Some(synced) = json.get("syncedLyrics").and_then(|v| v.as_str()) {
            if let Some(mut lyrics) = LrcParser::parse(synced, "LRCLIB") {
                lyrics.title = json
                    .get("trackName")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                lyrics.artist = json
                    .get("artistName")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                lyrics.album = json
                    .get("albumName")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                return Some(lyrics);
            }
        }

        if let Some(plain) = json.get("plainLyrics").and_then(|v| v.as_str()) {
            return Some(Lyrics {
                source_name: "LRCLIB".to_string(),
                provider: None,
                title: json
                    .get("trackName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                artist: json
                    .get("artistName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                album: json
                    .get("albumName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                duration_ms: None,
                sync_type: LyricsSyncType::None,
                is_synced: false,
                lines: Vec::new(),
                plain: plain.to_string(),
                offset_ms: 0,
            });
        }

        None
    }
}

// ============================================================================
// 7. Local File Adapter (.lrc & .ttml)
// ============================================================================

pub struct LocalFileAdapter;

impl LocalFileAdapter {
    pub fn load_file(path: &Path) -> Option<Lyrics> {
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "ttml" || content.trim_start().starts_with('<') {
            if let Some(lyrics) = TtmlParser::parse(&content, Some("local")) {
                let mut l = lyrics;
                l.source_name = "Локальный файл (TTML)".to_string();
                return Some(l);
            }
        }

        if let Some(lyrics) = LrcParser::parse(&content, "Локальный файл (LRC)") {
            return Some(lyrics);
        }

        // Fallback: read plain text file
        Some(Lyrics {
            source_name: "Локальный файл (Текст)".to_string(),
            provider: Some("local".to_string()),
            title: None,
            artist: None,
            album: None,
            duration_ms: None,
            sync_type: LyricsSyncType::None,
            is_synced: false,
            lines: Vec::new(),
            plain: content,
            offset_ms: 0,
        })
    }
}

// ============================================================================
// 8. Disk and Memory Cache
// ============================================================================

pub struct LyricsCache {
    memory: RwLock<HashMap<String, Lyrics>>,
    cache_dir: PathBuf,
}

impl LyricsCache {
    pub fn new() -> Self {
        let dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cibitube")
            .join("lyrics");
        Self::new_with_dir(dir)
    }

    pub fn new_with_dir(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            memory: RwLock::new(HashMap::new()),
            cache_dir,
        }
    }

    fn make_key(
        title: &str,
        artist: &str,
        duration_secs: Option<u64>,
        source_mode: usize,
    ) -> String {
        let clean = format!(
            "{}_{}_{}_mode{}",
            title.trim().to_lowercase(),
            artist.trim().to_lowercase(),
            duration_secs.unwrap_or(0),
            source_mode
        );
        let mut hasher = sha1::Sha1::new();
        hasher.update(clean.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn legacy_key(title: &str, artist: &str, duration_secs: Option<u64>) -> String {
        let clean = format!(
            "{}_{}_{}",
            title.trim().to_lowercase(),
            artist.trim().to_lowercase(),
            duration_secs.unwrap_or(0)
        );
        let mut hasher = sha1::Sha1::new();
        hasher.update(clean.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn matches_source_mode(source_mode: usize, lyrics: &Lyrics) -> bool {
        match source_mode {
            0 => true,
            1 => lyrics.source_name.contains("LRCLIB"),
            2 => lyrics.source_name.contains("QQ") || lyrics.provider.as_deref() == Some("qq"),
            3 => lyrics.source_name.contains("TTML") || lyrics.provider.as_deref() == Some("ttml"),
            4 => {
                lyrics.source_name.contains("KuGou") || lyrics.provider.as_deref() == Some("kugou")
            }
            5 => lyrics.source_name.contains("LrcMux"),
            _ => false,
        }
    }

    pub async fn get(
        &self,
        source_mode: usize,
        title: &str,
        artist: &str,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        let key = Self::make_key(title, artist, duration_secs, source_mode);
        {
            let mem = self.memory.read().await;
            if let Some(l) = mem.get(&key) {
                return Some(l.clone());
            }
        }

        let file_path = self.cache_dir.join(format!("{}.json", key));
        if file_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&file_path) {
                if let Ok(lyrics) = serde_json::from_str::<Lyrics>(&content) {
                    let mut mem = self.memory.write().await;
                    mem.insert(key, lyrics.clone());
                    return Some(lyrics);
                }
            }
        }

        // Backward compatibility: inspect unkeyed legacy file
        let leg_key = Self::legacy_key(title, artist, duration_secs);
        let leg_path = self.cache_dir.join(format!("{}.json", leg_key));
        if leg_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&leg_path) {
                if let Ok(lyrics) = serde_json::from_str::<Lyrics>(&content) {
                    if Self::matches_source_mode(source_mode, &lyrics) {
                        let mut mem = self.memory.write().await;
                        mem.insert(key, lyrics.clone());
                        return Some(lyrics);
                    }
                }
            }
        }

        None
    }

    pub async fn put(
        &self,
        source_mode: usize,
        title: &str,
        artist: &str,
        duration_secs: Option<u64>,
        lyrics: &Lyrics,
    ) {
        let key = Self::make_key(title, artist, duration_secs, source_mode);
        {
            let mut mem = self.memory.write().await;
            mem.insert(key.clone(), lyrics.clone());
        }

        let file_path = self.cache_dir.join(format!("{}.json", key));
        if let Ok(serialized) = serde_json::to_string(lyrics) {
            let _ = std::fs::write(file_path, serialized);
        }
    }
}

// ============================================================================
// 9. Central LyricsClient Orchestrator
// ============================================================================

pub struct LyricsClient {
    client: reqwest::Client,
    cache: LyricsCache,
}

impl LyricsClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            cache: LyricsCache::new(),
        }
    }

    #[allow(dead_code)]
    pub fn parse_lrc_string(synced: &str) -> Option<Lyrics> {
        LrcParser::parse(synced, "LRC")
    }

    pub async fn get_cached(
        &self,
        source_mode: usize,
        title: &str,
        artist: &str,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        self.cache
            .get(source_mode, title, artist, duration_secs)
            .await
    }

    pub async fn save_to_cache(
        &self,
        source_mode: usize,
        title: &str,
        artist: &str,
        duration_secs: Option<u64>,
        lyrics: &Lyrics,
    ) {
        self.cache
            .put(source_mode, title, artist, duration_secs, lyrics)
            .await;
    }

    pub async fn fetch_direct_source(
        &self,
        source_mode: usize,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        match source_mode {
            1 => LrclibAdapter::fetch(&self.client, title, artist, album, duration_secs).await,
            2 => {
                BetterLyricsAdapter::fetch_qq(&self.client, title, artist, album, duration_secs)
                    .await
            }
            3 => {
                BetterLyricsAdapter::fetch_ttml(&self.client, title, artist, album, duration_secs)
                    .await
            }
            4 => {
                BetterLyricsAdapter::fetch_kugou(&self.client, title, artist, album, duration_secs)
                    .await
            }
            5 => LrcMuxAdapter::fetch(&self.client, title, artist, album, duration_secs).await,
            _ => None,
        }
    }

    pub async fn fetch_richer_sync(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        // Priority 1: TTML (syllable sync)
        if let Some(l) =
            BetterLyricsAdapter::fetch_ttml(&self.client, title, artist, album, duration_secs).await
        {
            if l.is_word_synced() {
                return Some(l);
            }
        }

        // Priority 2: Better Lyrics QQ (word sync QRC)
        if let Some(l) =
            BetterLyricsAdapter::fetch_qq(&self.client, title, artist, album, duration_secs).await
        {
            if l.is_word_synced() {
                return Some(l);
            }
        }

        // Priority 3: LrcMux (word sync)
        if let Some(l) =
            LrcMuxAdapter::fetch(&self.client, title, artist, album, duration_secs).await
        {
            if l.is_word_synced() {
                return Some(l);
            }
        }

        None
    }

    pub async fn fetch(&self, title: &str, artist: &str) -> Option<Lyrics> {
        self.fetch_with_meta(title, artist, None, None).await
    }

    pub async fn fetch_with_meta(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_secs: Option<u64>,
    ) -> Option<Lyrics> {
        LrclibAdapter::fetch(&self.client, title, artist, album, duration_secs).await
    }
}

pub fn is_lyrics_response_valid(
    is_local: bool,
    current_req_id: u64,
    incoming_req_id: u64,
    current_track_id: Option<&str>,
    incoming_track_id: &str,
) -> bool {
    !is_local && current_req_id == incoming_req_id && current_track_id == Some(incoming_track_id)
}

// ============================================================================
// 10. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_lrc() {
        let sample = "\
[ti:Test Track]
[ar:Test Artist]
[00:09.55]The club isn't the best place
[00:11.99]So the bar is where I go";

        let parsed = LrcParser::parse(sample, "LRCLIB").expect("Failed to parse LRC");
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].start_ms, 9550);
        assert_eq!(parsed.lines[0].text, "The club isn't the best place");
        assert_eq!(parsed.lines[0].end_ms, Some(11990));
        assert_eq!(parsed.sync_type, LyricsSyncType::Line);
        assert!(!parsed.is_word_synced());
    }

    #[test]
    fn test_parse_qrc() {
        let sample = "\
[ti:Shape of You]
[0,2380]Shape(0,476) of(476,285) You(761,381)";

        let parsed = QrcParser::parse(sample, Some("qq")).expect("Failed to parse QRC");
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].start_ms, 0);
        assert_eq!(parsed.lines[0].end_ms, Some(2380));
        assert!(parsed.lines[0].words.is_some());
        let words = parsed.lines[0].words.as_ref().unwrap();
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "Shape");
        assert_eq!(words[0].start_ms, 0);
        assert_eq!(words[0].end_ms, 476);
        assert_eq!(parsed.sync_type, LyricsSyncType::Word);
        assert!(parsed.is_word_synced());
    }

    #[test]
    fn test_parse_ttml() {
        let sample = "\
<tt><body><div>
<p begin=\"9.731\" end=\"12.105\">
<span begin=\"9.731\" end=\"9.927\">The</span>
<span begin=\"9.927\" end=\"10.284\">club</span>
</p>
</div></body></tt>";

        let parsed = TtmlParser::parse(sample, Some("ttml")).expect("Failed to parse TTML");
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].start_ms, 9731);
        assert_eq!(parsed.lines[0].end_ms, Some(12105));
        assert_eq!(parsed.lines[0].text, "The club");
        let words = parsed.lines[0].words.as_ref().unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].text, "club");
        assert_eq!(words[1].start_ms, 9927);
        assert_eq!(words[1].end_ms, 10284);
        assert_eq!(parsed.sync_type, LyricsSyncType::Word);
        assert!(parsed.is_word_synced());
    }

    #[test]
    fn test_parse_ttml_syllables_punctuation_cjk() {
        // Syllables in single words + punctuation + CJK
        let sample = "\
<tt xmlns:itunes=\"http://music.apple.com/lyric-ttml-extensions\">
  <body>
    <div>
      <p begin=\"00:01.000\" end=\"00:03.500\">
        <span begin=\"00:01.000\" end=\"00:01.500\">Hel</span><span begin=\"00:01.500\" end=\"00:02.000\">lo</span><span begin=\"00:02.000\" end=\"00:03.000\">, world!</span>
      </p>
      <p begin=\"00:04.000\" end=\"00:06.000\">
        <span begin=\"00:04.000\" end=\"00:04.800\">你好</span><span begin=\"00:04.800\" end=\"00:05.500\">世界</span>
      </p>
    </div>
  </body>
</tt>";
        let parsed =
            TtmlParser::parse(sample, Some("ttml")).expect("Failed to parse TTML syllables");
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].text, "Hello, world!");
        assert_eq!(parsed.sync_type, LyricsSyncType::Syllable);
        assert_eq!(parsed.lines[1].text, "你好世界");
    }

    #[test]
    fn test_parse_ttml_line_only() {
        let sample = "\
<tt><body><div>
<p begin=\"00:01.200\" end=\"00:04.500\">Построчный текст без слогов</p>
<p begin=\"00:05.000\" end=\"00:08.000\">Вторая строка</p>
</div></body></tt>";
        let parsed =
            TtmlParser::parse(sample, Some("ttml")).expect("Failed to parse line-only TTML");
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.sync_type, LyricsSyncType::Line);
        assert!(parsed.lines[0].words.is_none());
        assert_eq!(parsed.lines[0].text, "Построчный текст без слогов");
        assert_eq!(parsed.lines[0].start_ms, 1200);
        assert_eq!(parsed.lines[0].end_ms, Some(4500));
    }

    #[test]
    fn test_parse_ttml_namespaces_entities_and_corrupted() {
        // XML entities and namespaces
        let sample_entities = "\
<tt:tt xmlns:tt=\"http://www.w3.org/ns/ttml\">
  <tt:body>
    <tt:div>
      <tt:p begin=\"1s\" end=\"3s\">
        <tt:span begin=\"1s\" end=\"2s\">Rock &amp; &quot;Roll&quot;</tt:span>
      </tt:p>
    </tt:div>
  </tt:body>
</tt:tt>";
        let parsed = TtmlParser::parse(sample_entities, Some("ttml")).expect("Entities TTML");
        assert_eq!(parsed.lines[0].text, "Rock & \"Roll\"");

        // Corrupted XML does not panic
        assert!(TtmlParser::parse("<tt><p begin=\"1\">Unclosed", None).is_none());
        assert!(TtmlParser::parse("plain non-xml text", None).is_none());
        assert!(TtmlParser::parse("", None).is_none());
    }

    #[test]
    fn test_parse_lrc_multi_timestamp_offset_and_parentheses() {
        let sample = "\
[ti:Test Track]
[ar:Test Artist]
[offset:500]
[00:10.00][00:30.00]Chorus line with (ooh yeah) non-timing brackets
[00:50.00]End line";

        let parsed = LrcParser::parse(sample, "LRCLIB").expect("Multi-timestamp LRC");
        assert_eq!(parsed.offset_ms, 500);
        assert_eq!(parsed.lines.len(), 3);
        assert_eq!(parsed.lines[0].start_ms, 10000);
        assert_eq!(
            parsed.lines[0].text,
            "Chorus line with (ooh yeah) non-timing brackets"
        );
        assert_eq!(parsed.lines[1].start_ms, 30000);
        assert_eq!(
            parsed.lines[1].text,
            "Chorus line with (ooh yeah) non-timing brackets"
        );
        assert_eq!(parsed.lines[2].start_ms, 50000);
    }

    #[test]
    fn test_parse_qrc_cyrillic_cjk_parentheses() {
        let sample = "\
[1000,3000]Я(1000,500) юзаю(1500,500) (хор) 練(2000,1000)";
        let parsed = QrcParser::parse(sample, Some("qq")).expect("QRC parse");
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].start_ms, 1000);
        assert_eq!(parsed.lines[0].end_ms, Some(4000));
        assert!(parsed.lines[0].text.contains("Я"));
        assert!(parsed.lines[0].text.contains("(хор)"));
        assert!(parsed.lines[0].text.contains("練"));
    }

    #[test]
    fn test_cjk_unicode_lyrics() {
        let sample = "\
[00:01.00]Я юзаю — 纏, 練, 発, бум!
[00:03.50]日本語の歌詞テスト";

        let parsed = LrcParser::parse(sample, "LRCLIB").expect("Failed to parse CJK LRC");
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].text, "Я юзаю — 纏, 練, 発, бум!");
        assert_eq!(parsed.lines[1].text, "日本語の歌詞テスト");
    }

    #[test]
    fn test_active_line_lookup_and_offset() {
        let mut lyrics = Lyrics {
            source_name: "Test".to_string(),
            provider: None,
            title: None,
            artist: None,
            album: None,
            duration_ms: Some(20000),
            sync_type: LyricsSyncType::Line,
            is_synced: true,
            plain: String::new(),
            lines: vec![
                LyricsLine {
                    seconds: 1.0,
                    start_ms: 1000,
                    end_ms: Some(3000),
                    text: "Line 1".to_string(),
                    words: None,
                    translation: None,
                    romanization: None,
                },
                LyricsLine {
                    seconds: 3.0,
                    start_ms: 3000,
                    end_ms: Some(5000),
                    text: "Line 2".to_string(),
                    words: None,
                    translation: None,
                    romanization: None,
                },
            ],
            offset_ms: 0,
        };

        // At 2.0s -> Line 0
        assert_eq!(lyrics.find_active_line_idx(2.0), Some(0));
        // At 3.5s -> Line 1
        assert_eq!(lyrics.find_active_line_idx(3.5), Some(1));

        // Apply offset -1000ms: at 2.0s effective time is 1.0s -> Line 0
        lyrics.offset_ms = -1500;
        // At 2.0s -> effective time 500ms (before line 1 at 1000ms) -> None
        assert_eq!(lyrics.find_active_line_idx(2.0), None);
    }

    #[test]
    fn test_clean_title_preserves_version() {
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (Official Video)"),
            "Song Name"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name [Official Music Video]"),
            "Song Name"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (feat. Drake)"),
            "Song Name"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (Remix)"),
            "Song Name (Remix)"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name [Live in Tokyo]"),
            "Song Name [Live in Tokyo]"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (Acoustic Version)"),
            "Song Name (Acoustic Version)"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (Cover)"),
            "Song Name (Cover)"
        );
        assert_eq!(
            LrclibAdapter::clean_title("Song Name (Instrumental)"),
            "Song Name (Instrumental)"
        );
    }

    #[test]
    fn test_select_best_match_different_artists() {
        // Target: "Hello" by "Adele", 295s
        let lionel = serde_json::json!({
            "trackName": "Hello",
            "artistName": "Lionel Richie",
            "duration": 295,
            "syncedLyrics": "[00:10.00]Hello, is it me you're looking for?",
            "plainLyrics": "Hello, is it me you're looking for?"
        });
        let adele = serde_json::json!({
            "trackName": "Hello",
            "artistName": "Adele",
            "duration": 295,
            "plainLyrics": "Hello from the other side"
        });

        let items = vec![lionel, adele];
        let best = LrclibAdapter::select_best_match(items, "Hello", "Adele", None, Some(295));
        assert!(best.is_some());
        let selected = best.unwrap();
        assert_eq!(selected["artistName"], "Adele");
        // Must choose Adele's plain lyrics over Lionel Richie's synced lyrics!
        assert_eq!(selected["plainLyrics"], "Hello from the other side");
    }

    #[test]
    fn test_select_best_match_version_mismatch() {
        // Target: "Levitating" by "Dua Lipa" (Original studio version, 203s)
        let remix = serde_json::json!({
            "trackName": "Levitating (The Blessed Madonna Remix)",
            "artistName": "Dua Lipa",
            "duration": 203,
            "syncedLyrics": "[00:05.00]Levitating remix beat"
        });
        let studio = serde_json::json!({
            "trackName": "Levitating",
            "artistName": "Dua Lipa",
            "duration": 203,
            "plainLyrics": "If you wanna run away with me, I know a galaxy"
        });

        let items = vec![remix, studio];
        let best =
            LrclibAdapter::select_best_match(items, "Levitating", "Dua Lipa", None, Some(203));
        assert!(best.is_some());
        let selected = best.unwrap();
        // Remix should be rejected because original is requested
        assert_eq!(selected["trackName"], "Levitating");
        assert_eq!(
            selected["plainLyrics"],
            "If you wanna run away with me, I know a galaxy"
        );

        // Now reverse: target is explicitly a remix
        let items2 = vec![
            serde_json::json!({
                "trackName": "Levitating",
                "artistName": "Dua Lipa",
                "duration": 203,
                "syncedLyrics": "[00:05.00]Original synced"
            }),
            serde_json::json!({
                "trackName": "Levitating (Remix)",
                "artistName": "Dua Lipa",
                "duration": 203,
                "plainLyrics": "Remix plain"
            }),
        ];
        let best_remix = LrclibAdapter::select_best_match(
            items2,
            "Levitating (Remix)",
            "Dua Lipa",
            None,
            Some(203),
        );
        assert!(best_remix.is_some());
        assert_eq!(best_remix.unwrap()["trackName"], "Levitating (Remix)");
    }

    #[test]
    fn test_select_best_match_incompatible_duration() {
        // Target: 240s
        let shortened = serde_json::json!({
            "trackName": "My Song",
            "artistName": "My Artist",
            "duration": 180, // diff = 60s > 15s max threshold
            "syncedLyrics": "[00:10.00]Short version"
        });
        let items = vec![shortened];
        let best = LrclibAdapter::select_best_match(items, "My Song", "My Artist", None, Some(240));
        assert!(
            best.is_none(),
            "Should reject candidate with incompatible duration"
        );
    }

    #[test]
    fn test_select_best_match_correct_plain_over_wrong_synced() {
        let wrong_artist_synced = serde_json::json!({
            "trackName": "Yesterday",
            "artistName": "Boyz II Men",
            "duration": 125,
            "syncedLyrics": "[00:05.00]Yesterday..."
        });
        let correct_plain = serde_json::json!({
            "trackName": "Yesterday",
            "artistName": "The Beatles",
            "duration": 125,
            "plainLyrics": "Yesterday, all my troubles seemed so far away"
        });

        let items = vec![wrong_artist_synced, correct_plain];
        let best =
            LrclibAdapter::select_best_match(items, "Yesterday", "The Beatles", None, Some(125));
        assert!(best.is_some());
        let selected = best.unwrap();
        assert_eq!(selected["artistName"], "The Beatles");
    }

    #[tokio::test]
    async fn test_cache_provider_isolation() {
        let temp_dir = std::env::temp_dir().join(format!(
            "cibitube_cache_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = LyricsCache::new_with_dir(temp_dir.clone());

        let lrc_lib_lyrics = Lyrics {
            source_name: "LRCLIB".to_string(),
            provider: Some("lrclib".to_string()),
            title: Some("Song".to_string()),
            artist: Some("Artist".to_string()),
            album: None,
            duration_ms: Some(180000),
            sync_type: LyricsSyncType::Line,
            is_synced: true,
            plain: "Lyrics from LRCLIB".to_string(),
            lines: vec![],
            offset_ms: 0,
        };

        // Put into cache under mode 1 (LRCLIB)
        cache
            .put(1, "Song", "Artist", Some(180), &lrc_lib_lyrics)
            .await;

        // Query under mode 2 (QQ) - must be None
        let qq_cached = cache.get(2, "Song", "Artist", Some(180)).await;
        assert!(
            qq_cached.is_none(),
            "Cache for QQ must not return LRCLIB lyrics"
        );

        // Query under mode 1 (LRCLIB) - must be Some
        let lrclib_cached = cache.get(1, "Song", "Artist", Some(180)).await;
        assert!(lrclib_cached.is_some());
        assert_eq!(lrclib_cached.unwrap().plain, "Lyrics from LRCLIB");

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_stale_request_handling_and_local_protection() {
        // Normal matching response: valid
        assert!(is_lyrics_response_valid(
            false,
            5,
            5,
            Some("track_123"),
            "track_123"
        ));

        // Stale response from previous request ID (e.g. song was changed or search refreshed)
        assert!(!is_lyrics_response_valid(
            false,
            6,
            5,
            Some("track_123"),
            "track_123"
        ));

        // Stale response for previous track
        assert!(!is_lyrics_response_valid(
            false,
            5,
            5,
            Some("track_456"),
            "track_123"
        ));

        // Local lyrics file selected by user: must protect against any incoming network responses!
        assert!(!is_lyrics_response_valid(
            true,
            5,
            5,
            Some("track_123"),
            "track_123"
        ));
    }
}
