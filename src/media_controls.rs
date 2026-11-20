use crate::models::{Lyrics, PlaybackInfo, Track};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use tokio::sync::watch;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub(crate) enum Command {
    Raise,
    Toggle,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek(i64),
    SetPosition(String, i64),
    Volume(f64),
    Rate(f64),
}

#[derive(Clone, Default, PartialEq)]
struct State {
    track: Option<Track>,
    playing: bool,
    loaded: bool,
    position: i64,
    duration: i64,
    volume: f64,
    rate: f64,
    next: bool,
    previous: bool,
    lyrics_offset: i64,
}

impl State {
    fn track_path(&self) -> String {
        self.track.as_ref().map_or_else(
            || "/org/mpris/MediaPlayer2/TrackList/NoTrack".into(),
            |track| format!("{PATH}/track/t{}", hex::encode(track.id.as_bytes())),
        )
    }

    fn status(&self) -> &str {
        if !self.loaded {
            "Stopped"
        } else if self.playing {
            "Playing"
        } else {
            "Paused"
        }
    }
}

#[derive(Clone)]
struct Service {
    state: Arc<Mutex<State>>,
    commands: mpsc::Sender<Command>,
    ctx: egui::Context,
}

impl Service {
    fn command(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            self.ctx.request_repaint();
        }
    }

    fn snapshot(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

struct Root(Service);

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        self.0.command(Command::Raise);
    }
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Cibitube"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "cibitube"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Service {
    fn play_pause(&self) {
        self.command(Command::Toggle);
    }
    fn play(&self) {
        self.command(Command::Play);
    }
    fn pause(&self) {
        self.command(Command::Pause);
    }
    fn stop(&self) {
        self.command(Command::Stop);
    }
    fn next(&self) {
        self.command(Command::Next);
    }
    fn previous(&self) {
        self.command(Command::Previous);
    }
    fn seek(&self, offset: i64) {
        self.command(Command::Seek(offset));
    }
    fn set_position(&self, track_id: OwnedObjectPath, position: i64) {
        self.command(Command::SetPosition(track_id.to_string(), position));
    }
    fn open_uri(&self, _uri: &str) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "OpenUri is unavailable".into(),
        ))
    }
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.snapshot().status().into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.snapshot().rate.max(0.5)
    }
    #[zbus(property)]
    fn set_rate(&mut self, value: f64) -> zbus::fdo::Result<()> {
        if !value.is_finite() || !(0.5..=2.0).contains(&value) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Speed must be between 0.5 and 2".into(),
            ));
        }
        self.command(Command::Rate(value));
        Ok(())
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        0.5
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        2.0
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.snapshot().volume
    }
    #[zbus(property)]
    fn set_volume(&mut self, value: f64) -> zbus::fdo::Result<()> {
        if !value.is_finite() {
            return Err(zbus::fdo::Error::InvalidArgs("Invalid volume".into()));
        }
        self.command(Command::Volume(value.clamp(0.0, 1.0)));
        Ok(())
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.snapshot().position
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.snapshot().next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.snapshot().previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.snapshot().track.is_some()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.snapshot().loaded
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.snapshot().loaded
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let state = self.snapshot();
        let mut metadata = HashMap::new();
        if let Some(track) = &state.track {
            metadata.insert(
                "mpris:trackid".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from(
                    OwnedObjectPath::try_from(state.track_path()).unwrap(),
                ))
                .unwrap(),
            );
            metadata.insert("mpris:length".into(), state.duration.into());
            metadata.insert("xesam:title".into(), Str::from(track.title.as_str()).into());
            metadata.insert(
                "xesam:artist".into(),
                OwnedValue::try_from(zbus::zvariant::Value::from(vec![track.artist.as_str()]))
                    .unwrap(),
            );
            if let Some(cover) = &track.thumbnail_url {
                metadata.insert("mpris:artUrl".into(), Str::from(cover.as_str()).into());
            }
        }
        metadata
    }
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct LyricsSnapshot {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub english: bool,
    pub revision: u64,
    pub synced: bool,
    pub available: bool,
    pub active: Option<usize>,
    pub lines: Option<Vec<String>>,
}

#[derive(Default)]
struct LyricContent {
    track_id: Option<String>,
    revision: u64,
    lyrics: Option<Arc<Lyrics>>,
}

struct TerminalService {
    service: Service,
    content: Arc<Mutex<LyricContent>>,
}

#[zbus::interface(name = "io.github.Cibitka.Cibitube.Lyrics1")]
impl TerminalService {
    fn snapshot(&self, known_track: &str, known_revision: u64) -> zbus::fdo::Result<String> {
        let state = self.service.snapshot();
        let (revision, owner, lyrics) = {
            let content = self
                .content
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            (
                content.revision,
                content.track_id.clone(),
                content.lyrics.clone(),
            )
        };
        let track_id = state.track.as_ref().map_or("", |track| track.id.as_str());
        let lyrics = lyrics.filter(|_| owner.as_deref() == Some(track_id));
        let synced = lyrics.as_ref().is_some_and(|lyrics| lyrics.is_synced);
        let position_ms = (state.position / 1000)
            .saturating_add(state.lyrics_offset)
            .max(0) as u64;
        let active = lyrics.as_ref().filter(|_| synced).and_then(|lyrics| {
            lyrics
                .lines
                .iter()
                .rposition(|line| line.start_ms <= position_ms)
        });
        let changed = known_track != track_id || known_revision != revision;
        let lines = changed.then(|| {
            lyrics.as_ref().map_or_else(Vec::new, |lyrics| {
                if lyrics.lines.is_empty() {
                    lyrics.plain.lines().map(str::to_owned).collect()
                } else {
                    lyrics.lines.iter().map(|line| line.text.clone()).collect()
                }
            })
        });
        let snapshot = LyricsSnapshot {
            track_id: track_id.to_owned(),
            title: state
                .track
                .as_ref()
                .map_or_else(String::new, |track| track.title.clone()),
            artist: state
                .track
                .as_ref()
                .map_or_else(String::new, |track| track.artist.clone()),
            playing: state.loaded && state.playing,
            position_secs: state.position as f64 / 1_000_000.0,
            duration_secs: state.duration as f64 / 1_000_000.0,
            english: crate::i18n::is_english(),
            revision,
            synced,
            available: lyrics
                .as_ref()
                .is_some_and(|lyrics| !lyrics.lines.is_empty() || !lyrics.plain.trim().is_empty()),
            active,
            lines,
        };
        serde_json::to_string(&snapshot)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

pub(crate) struct MediaControls {
    commands: mpsc::Receiver<Command>,
    updates: watch::Sender<State>,
    lyrics: Arc<Mutex<LyricContent>>,
}

impl MediaControls {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let (tx, commands) = mpsc::channel();
        let (updates, mut rx) = watch::channel(State::default());
        let state = Arc::new(Mutex::new(State::default()));
        let service = Service {
            state: state.clone(),
            commands: tx,
            ctx: ctx.clone(),
        };
        let lyrics = Arc::new(Mutex::new(LyricContent::default()));
        let terminal_lyrics = lyrics.clone();
        tokio::spawn(async move {
            loop {
                let connect = async {
                    zbus::connection::Builder::session()?
                        .name(format!(
                            "org.mpris.MediaPlayer2.cibitube.instance{}",
                            std::process::id()
                        ))?
                        .serve_at(PATH, Root(service.clone()))?
                        .serve_at(PATH, service.clone())?
                        .serve_at(
                            PATH,
                            TerminalService {
                                service: service.clone(),
                                content: terminal_lyrics.clone(),
                            },
                        )?
                        .build()
                        .await
                };
                let connection = match connect.await {
                    Ok(connection) => connection,
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        if rx.has_changed().is_err() {
                            return;
                        }
                        continue;
                    }
                };
                let mut last = State::default();
                loop {
                    if rx.changed().await.is_err() {
                        return;
                    }
                    let current = rx.borrow_and_update().clone();
                    *state.lock().unwrap_or_else(|error| error.into_inner()) = current.clone();
                    let mut changed = Vec::new();
                    if current.track != last.track || current.duration != last.duration {
                        changed.push("Metadata");
                    }
                    if current.status() != last.status() {
                        changed.push("PlaybackStatus");
                    }
                    if current.volume != last.volume {
                        changed.push("Volume");
                    }
                    if current.rate != last.rate {
                        changed.push("Rate");
                    }
                    if current.next != last.next {
                        changed.push("CanGoNext");
                    }
                    if current.previous != last.previous {
                        changed.push("CanGoPrevious");
                    }
                    if current.track.is_some() != last.track.is_some() {
                        changed.push("CanPlay");
                    }
                    if current.loaded != last.loaded {
                        changed.extend(["CanPause", "CanSeek"]);
                    }
                    if !changed.is_empty()
                        && connection
                            .emit_signal(
                                None::<&str>,
                                PATH,
                                "org.freedesktop.DBus.Properties",
                                "PropertiesChanged",
                                &(PLAYER, HashMap::<String, OwnedValue>::new(), changed),
                            )
                            .await
                            .is_err()
                    {
                        break;
                    }
                    if current.track == last.track
                        && (current.position - last.position).abs() > 1_500_000
                    {
                        let _ = connection
                            .emit_signal(None::<&str>, PATH, PLAYER, "Seeked", &(current.position,))
                            .await;
                    }
                    last = current;
                }
            }
        });
        Self {
            commands,
            updates,
            lyrics,
        }
    }

    pub(crate) fn set_lyrics(&self, track_id: Option<&str>, lyrics: Option<&Lyrics>) {
        let mut content = self
            .lyrics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        content.track_id = track_id.map(str::to_owned);
        content.lyrics = lyrics.map(|lyrics| Arc::new(lyrics.clone()));
        content.revision = content.revision.wrapping_add(1);
    }

    pub(crate) fn command(&self) -> Option<Command> {
        self.commands.try_recv().ok()
    }

    pub(crate) fn matches_track(&self, path: &str, track: Option<&Track>) -> bool {
        track.is_some_and(|track| {
            path == format!("{PATH}/track/t{}", hex::encode(track.id.as_bytes()))
        })
    }

    pub(crate) fn sync(
        &self,
        track: Option<&Track>,
        info: &PlaybackInfo,
        loaded: bool,
        navigation: (bool, bool),
        rate: f64,
        lyrics_offset: i64,
    ) {
        let state = State {
            track: track.cloned(),
            playing: info.is_playing,
            loaded,
            position: (info.position_secs.max(0.0) * 1_000_000.0) as i64,
            duration: (info.duration_secs.max(0.0) * 1_000_000.0) as i64,
            volume: info.volume / 100.0,
            next: navigation.0,
            previous: navigation.1,
            lyrics_offset,
            rate,
        };
        self.updates.send_if_modified(|previous| {
            if *previous == state {
                return false;
            }
            *previous = state;
            true
        });
    }
}
