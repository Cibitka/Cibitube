use crate::models::{PlaybackInfo, Track};
use serde_json::{json, Value};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
type RpcStream = UnixStream;

#[cfg(windows)]
type RpcStream = std::fs::File;

#[cfg(not(any(unix, windows)))]
type RpcStream = std::io::Cursor<Vec<u8>>;

const CIBITUBE_ICON: &str =
    "https://raw.githubusercontent.com/Cibitka/Cibitube/main/assets/icon.png";

const CIBITUBE_APPLICATION_ID: &str = "1571534617847275590";

pub struct DiscordRpc {
    tx: mpsc::Sender<Option<ActivityDetails>>,
    last_activity: Option<ActivityDetails>,
    status: Arc<Mutex<DiscordStatus>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DiscordStatus {
    #[default]
    Idle,
    Connecting,
    Unavailable,
    Active,
    Rejected(i64),
}

#[derive(Clone, Debug)]
struct ActivityDetails {
    title: String,
    artist: String,
    song_id: String,
    start_timestamp: u64,
    end_timestamp: Option<u64>,
    thumbnail: Option<String>,
    show_buttons: bool,
    activity_type: usize,
}

impl DiscordRpc {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Option<ActivityDetails>>();
        let status = Arc::new(Mutex::new(DiscordStatus::Idle));
        let worker_status = Arc::clone(&status);

        thread::spawn(move || {
            let mut stream = None;
            let mut activity = None;
            loop {
                let wait = match (&activity, &stream) {
                    (Some(_), Some(_)) => Duration::from_secs(20),
                    (Some(_), None) => Duration::from_secs(2),
                    (None, _) => Duration::from_secs(60),
                };
                match rx.recv_timeout(wait) {
                    Ok(next) => activity = rx.try_iter().last().unwrap_or(next),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                let result = if let Some(active) = &activity {
                    if stream.is_none() {
                        if let Ok(mut status) = worker_status.lock() {
                            *status = DiscordStatus::Connecting;
                        }
                        stream = connect_discord();
                    }
                    match stream.as_mut().map(|stream| send_activity(stream, active)) {
                        Some(Ok(None)) => DiscordStatus::Active,
                        Some(Ok(Some(code))) => DiscordStatus::Rejected(code),
                        Some(Err(_)) | None => {
                            stream = None;
                            DiscordStatus::Unavailable
                        }
                    }
                } else {
                    if let Some(mut stream) = stream.take() {
                        let _ = send_set_activity(&mut stream, Value::Null);
                    }
                    DiscordStatus::Idle
                };
                if let Ok(mut status) = worker_status.lock() {
                    *status = result;
                }
            }
        });

        Self {
            tx,
            status,
            last_activity: None,
        }
    }

    pub(crate) fn status(&self) -> DiscordStatus {
        self.status
            .lock()
            .map_or(DiscordStatus::Idle, |status| *status)
    }

    pub(crate) fn sync_activity(
        &mut self,
        track: Option<&Track>,
        playback: &PlaybackInfo,
        show_buttons: bool,
        activity_type: usize,
        speed: f64,
    ) {
        let Some(track) = track.filter(|_| playback.is_playing) else {
            if self.last_activity.take().is_some() {
                let _ = self.tx.send(None);
            }
            return;
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let speed = speed.clamp(0.5, 2.0);
        let start_timestamp = now.saturating_sub((playback.position_secs / speed) as u64);
        let end_timestamp = (playback.duration_secs > 0.0)
            .then(|| start_timestamp.saturating_add((playback.duration_secs / speed) as u64));
        let unchanged = self.last_activity.as_ref().is_some_and(|last| {
            last.song_id == track.id
                && last.title == track.title
                && last.artist == track.artist
                && last.thumbnail == track.thumbnail_url
                && last.show_buttons == show_buttons
                && last.activity_type == activity_type
                && last.start_timestamp.abs_diff(start_timestamp) <= 3
                && match (last.end_timestamp, end_timestamp) {
                    (Some(previous), Some(current)) => previous.abs_diff(current) <= 3,
                    (None, None) => true,
                    _ => false,
                }
        });
        if !unchanged {
            let activity = ActivityDetails {
                title: track.title.clone(),
                artist: track.artist.clone(),
                song_id: track.id.clone(),
                start_timestamp,
                end_timestamp,
                thumbnail: track.thumbnail_url.clone(),
                show_buttons,
                activity_type,
            };
            let _ = self.tx.send(Some(activity.clone()));
            self.last_activity = Some(activity);
        }
    }
}

fn connect_discord() -> Option<RpcStream> {
    let handshake = json!({"v": 1, "client_id": CIBITUBE_APPLICATION_ID}).to_string();
    for path in socket_paths() {
        let Ok(mut stream) = connect_rpc_stream(&path) else {
            continue;
        };
        if send_packet(&mut stream, 0, &handshake).is_ok()
            && read_frame(&mut stream).is_ok_and(|frame| frame["evt"] == "READY")
        {
            return Some(stream);
        }
    }
    None
}

fn send_activity(stream: &mut RpcStream, act: &ActivityDetails) -> std::io::Result<Option<i64>> {
    let mut timestamps = json!({ "start": act.start_timestamp });
    if let Some(end) = act.end_timestamp {
        timestamps["end"] = json!(end);
    }

    let image_url = act
        .thumbnail
        .clone()
        .unwrap_or_else(|| CIBITUBE_ICON.to_string());

    let mut activity_obj = json!({
        "name": "Cibitube",
        "details": act.title,
        "state": format!("by {}", act.artist),
        "timestamps": timestamps,
        "assets": {
            "large_image": image_url,
            "large_text": format!("{} — {}", act.title, act.artist),
            "small_image": CIBITUBE_ICON,
            "small_text": "Cibitube"
        },
        "type": if act.activity_type == 1 { 0 } else { 2 } // 2 = Listening, 0 = Playing
    });

    if act.show_buttons {
        let mut buttons = Vec::new();
        if !act.song_id.is_empty() && !act.song_id.starts_with("local:") {
            buttons.push(json!({
                "label": "Слушать в YouTube Music",
                "url": format!("https://music.youtube.com/watch?v={}", act.song_id)
            }));
        }
        buttons.push(json!({
            "label": "Cibitube",
            "url": "https://github.com/Cibitka/Cibitube"
        }));
        activity_obj["buttons"] = json!(buttons);
    }

    send_set_activity(stream, activity_obj)
}

fn send_set_activity(
    stream: &mut (impl Read + Write),
    activity: Value,
) -> std::io::Result<Option<i64>> {
    let nonce = "cibitube-activity";
    let payload = json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity },
        "nonce": nonce
    });
    send_packet(stream, 1, &payload.to_string())?;
    for _ in 0..16 {
        let reply = read_frame(stream)?;
        if reply["nonce"] == nonce {
            return Ok(
                (reply["evt"] == "ERROR").then(|| reply["data"]["code"].as_i64().unwrap_or(0))
            );
        }
    }
    Err(std::io::Error::other(
        "Discord did not confirm SET_ACTIVITY",
    ))
}

#[cfg(unix)]
fn connect_rpc_stream(path: &Path) -> std::io::Result<RpcStream> {
    let stream = UnixStream::connect(path)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    Ok(stream)
}

#[cfg(windows)]
fn connect_rpc_stream(path: &Path) -> std::io::Result<RpcStream> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn connect_rpc_stream(_path: &Path) -> std::io::Result<RpcStream> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Unsupported OS",
    ))
}

#[cfg(unix)]
fn socket_paths() -> Vec<PathBuf> {
    let mut roots: Vec<_> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    roots.push(std::env::temp_dir());
    let mut paths = Vec::new();
    for root in roots {
        for location in ["", "app/com.discordapp.Discord", "app/dev.vencord.Vesktop"] {
            for i in 0..10 {
                let path = root.join(location).join(format!("discord-ipc-{i}"));
                if path.exists() && !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

#[cfg(windows)]
fn socket_paths() -> Vec<PathBuf> {
    (0..10)
        .map(|i| PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}")))
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn socket_paths() -> Vec<PathBuf> {
    Vec::new()
}

fn send_packet(stream: &mut impl Write, opcode: u32, payload: &str) -> std::io::Result<()> {
    let bytes = payload.as_bytes();
    let length = bytes.len() as u32;

    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..8].copy_from_slice(&length.to_le_bytes());

    stream.write_all(&header)?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_frame(stream: &mut (impl Read + Write)) -> std::io::Result<Value> {
    for _ in 0..16 {
        let mut header = [0u8; 8];
        stream.read_exact(&mut header)?;
        let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
        let length = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if length > 65536 {
            return Err(std::io::Error::other("Discord IPC frame too large"));
        }
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body)?;
        match opcode {
            1 => return Ok(serde_json::from_slice(&body)?),
            3 => {
                let payload = std::str::from_utf8(&body).map_err(std::io::Error::other)?;
                send_packet(stream, 4, payload)?;
            }
            _ => return Err(std::io::Error::other("Discord IPC connection closed")),
        }
    }
    Err(std::io::Error::other("Discord IPC reply missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_activity_when_a_restored_track_resumes_and_clears_on_pause() {
        let (tx, rx) = mpsc::channel();
        let mut discord = DiscordRpc {
            tx,
            last_activity: None,
            status: Arc::new(Mutex::new(DiscordStatus::Idle)),
        };
        let track = Track {
            id: "restored".into(),
            title: "Restored track".into(),
            artist: "Test".into(),
            duration_text: "3:00".into(),
            thumbnail_url: None,
        };
        let mut playback = PlaybackInfo {
            position_secs: 55.0,
            duration_secs: 180.0,
            ..Default::default()
        };
        discord.sync_activity(Some(&track), &playback, true, 0, 1.0);
        assert!(rx.try_recv().is_err());

        playback.is_playing = true;
        discord.sync_activity(Some(&track), &playback, true, 0, 1.0);
        assert_eq!(rx.try_recv().unwrap().unwrap().song_id, track.id);
        discord.sync_activity(Some(&track), &playback, true, 0, 1.0);
        assert!(rx.try_recv().is_err());

        playback.is_playing = false;
        discord.sync_activity(Some(&track), &playback, true, 0, 1.0);
        assert!(rx.try_recv().unwrap().is_none());
    }

    #[test]
    fn matches_the_reply_and_reports_a_rejected_status() {
        #[derive(Default)]
        struct MockStream {
            received: std::io::Cursor<Vec<u8>>,
            sent: std::io::Cursor<Vec<u8>>,
        }
        impl Read for MockStream {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                self.received.read(bytes)
            }
        }
        impl Write for MockStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.sent.write(bytes)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut incoming = Vec::new();
        send_packet(&mut incoming, 3, "{}").unwrap();
        send_packet(&mut incoming, 1, r#"{"evt":"DISPATCH"}"#).unwrap();
        send_packet(
            &mut incoming,
            1,
            r#"{"evt":"ERROR","data":{"code":4000},"nonce":"cibitube-activity"}"#,
        )
        .unwrap();
        let mut socket = MockStream {
            received: std::io::Cursor::new(incoming),
            ..Default::default()
        };
        assert_eq!(
            send_set_activity(&mut socket, json!({"details":"Test"})).unwrap(),
            Some(4000)
        );
        let mut output = MockStream {
            received: socket.sent,
            ..Default::default()
        };
        output.received.set_position(0);
        assert_eq!(read_frame(&mut output).unwrap()["cmd"], "SET_ACTIVITY");
        let mut pong = [0; 4];
        output.read_exact(&mut pong).unwrap();
        assert_eq!(u32::from_le_bytes(pong), 4);
    }
}
