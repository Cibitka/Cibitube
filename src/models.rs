use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_text: String,
    pub thumbnail_url: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct LyricsLine {
    pub seconds: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Lyrics {
    pub is_synced: bool,
    pub plain: String,
    pub lines: Vec<LyricsLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTab {
    Search,
    Liked,
    Playlists,
    Queue,
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
