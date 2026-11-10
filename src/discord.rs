use serde_json::json;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
type RpcStream = UnixStream;

#[cfg(windows)]
type RpcStream = std::fs::File;

#[cfg(not(any(unix, windows)))]
type RpcStream = std::io::Cursor<Vec<u8>>;

const METROLIST_CLIENT_ID: &str = "1447278780795064401";

pub struct DiscordRpc {
    tx: mpsc::Sender<RpcMessage>,
}

#[derive(Clone, Debug)]
pub struct ActivityDetails {
    pub title: String,
    pub artist: String,
    pub song_id: String,
    pub elapsed_secs: f64,
    pub duration_secs: f64,
    pub thumbnail: Option<String>,
    pub show_buttons: bool,
    pub activity_type: usize, // 0 = Listening (type 2), 1 = Playing (type 0)
}

enum RpcMessage {
    Update(ActivityDetails),
    Clear,
}

impl DiscordRpc {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<RpcMessage>();

        thread::spawn(move || {
            let mut current_stream: Option<RpcStream> = None;
            let mut last_activity: Option<ActivityDetails> = None;

            loop {
                // If not connected, try to connect
                if current_stream.is_none() {
                    if let Some(socket_path) = get_socket_path() {
                        if let Ok(mut stream) = connect_rpc_stream(&socket_path) {
                            // Send handshake
                            let handshake = json!({
                                "v": 1,
                                "client_id": METROLIST_CLIENT_ID
                            })
                            .to_string();

                            if send_packet(&mut stream, 0, &handshake).is_ok() {
                                let _ = read_response(&mut stream);
                                current_stream = Some(stream);

                                // Resend last activity if any
                                if let (Some(s), Some(act)) =
                                    (current_stream.as_mut(), &last_activity)
                                {
                                    send_activity(s, act);
                                }
                            }
                        }
                    }
                }

                // Check messages with timeout
                match rx.recv_timeout(Duration::from_millis(2000)) {
                    Ok(RpcMessage::Update(act)) => {
                        last_activity = Some(act.clone());
                        if let Some(stream) = current_stream.as_mut() {
                            if !send_activity(stream, &act) {
                                current_stream = None;
                            }
                        }
                    }
                    Ok(RpcMessage::Clear) => {
                        last_activity = None;
                        if let Some(stream) = current_stream.as_mut() {
                            let payload = json!({
                                "cmd": "SET_ACTIVITY",
                                "args": {
                                    "pid": std::process::id()
                                },
                                "nonce": "clear"
                            })
                            .to_string();
                            let _ = send_packet(stream, 1, &payload);
                            let _ = read_response(stream);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Periodic heartbeat / check connection
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
        });

        Self { tx }
    }

    pub fn set_activity(&self, details: ActivityDetails) {
        let _ = self.tx.send(RpcMessage::Update(details));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(RpcMessage::Clear);
    }
}

fn send_activity(stream: &mut RpcStream, act: &ActivityDetails) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let start_timestamp = if act.elapsed_secs > 0.0 {
        now.saturating_sub(act.elapsed_secs as u64)
    } else {
        now
    };

    let end_timestamp = if act.duration_secs > 0.0 {
        Some(start_timestamp + act.duration_secs as u64)
    } else {
        None
    };

    let mut timestamps = json!({ "start": start_timestamp });
    if let Some(end) = end_timestamp {
        timestamps["end"] = json!(end);
    }

    let image_url = act.thumbnail.clone().unwrap_or_else(|| {
        "https://raw.githubusercontent.com/MetrolistGroup/Metrolist/main/assets/ic_launcher-playstore.png".to_string()
    });

    let mut activity_obj = json!({
        "name": act.title,
        "details": act.title,
        "state": format!("by {}", act.artist),
        "timestamps": timestamps,
        "assets": {
            "large_image": image_url,
            "large_text": format!("{} — {}", act.title, act.artist),
            "small_image": "https://raw.githubusercontent.com/MetrolistGroup/Metrolist/main/assets/ic_launcher-playstore.png",
            "small_text": "Cibitube"
        },
        "type": if act.activity_type == 1 { 0 } else { 2 } // 2 = Listening, 0 = Playing
    });

    if act.show_buttons {
        let mut buttons = Vec::new();
        if !act.song_id.is_empty() {
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

    let payload = json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": std::process::id(),
            "activity": activity_obj
        },
        "nonce": "metrolist-update"
    })
    .to_string();

    if send_packet(stream, 1, &payload).is_err() {
        return false;
    }
    let _ = read_response(stream);
    true
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
fn get_socket_path() -> Option<PathBuf> {
    let runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string());
    for i in 0..10 {
        let path = PathBuf::from(format!("{}/discord-ipc-{}", runtime_dir, i));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn get_socket_path() -> Option<PathBuf> {
    for i in 0..10 {
        let path = PathBuf::from(format!(r"\\.\pipe\discord-ipc-{}", i));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[cfg(not(any(unix, windows)))]
fn get_socket_path() -> Option<PathBuf> {
    None
}

fn send_packet(stream: &mut RpcStream, opcode: u32, payload: &str) -> std::io::Result<()> {
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

fn read_response(stream: &mut RpcStream) -> std::io::Result<String> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header)?;
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length > 65536 {
        return Ok(String::new());
    }
    let mut buf = vec![0u8; length];
    stream.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}
