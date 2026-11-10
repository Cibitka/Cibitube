use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_text: String,
    pub thumbnail_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricsSyncType {
    #[default]
    None, // Plain text without timestamps
    Line,     // Line-by-line sync
    Word,     // Word-by-word sync
    Syllable, // Syllable-by-syllable sync
}

impl std::fmt::Display for LyricsSyncType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LyricsSyncType::None => write!(f, "Обычный текст"),
            LyricsSyncType::Line => write!(f, "Построчно"),
            LyricsSyncType::Word => write!(f, "Пословно"),
            LyricsSyncType::Syllable => write!(f, "По слогам"),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LyricsWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LyricsLine {
    pub seconds: f64,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
    pub words: Option<Vec<LyricsWord>>,
    pub translation: Option<String>,
    pub romanization: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Lyrics {
    pub source_name: String,
    pub provider: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub sync_type: LyricsSyncType,
    pub is_synced: bool,
    pub lines: Vec<LyricsLine>,
    pub plain: String,
    pub offset_ms: i64,
}

impl Lyrics {
    pub fn is_word_synced(&self) -> bool {
        matches!(
            self.sync_type,
            LyricsSyncType::Word | LyricsSyncType::Syllable
        )
    }

    #[allow(dead_code)]
    pub fn effective_ms(&self, position_secs: f64) -> i64 {
        (position_secs * 1000.0) as i64 + self.offset_ms
    }

    #[allow(dead_code)]
    pub fn find_active_line_idx(&self, position_secs: f64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let current_ms = self.effective_ms(position_secs).max(0) as u64;
        self.lines.iter().rposition(|l| l.start_ms <= current_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTab {
    Search,
    Liked,
    Playlists,
    Queue,
    Wave,
    Lyrics,
    Downloads,
    Settings,
    NowPlaying,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub thumbnail_url: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PlaybackInfo {
    pub is_playing: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountProfile {
    pub name: String,
    pub email: String,
    pub auth_user: usize,
}
