use crate::media_controls::LyricsSnapshot;
use std::io::{self, IsTerminal, Write};
use std::os::fd::AsRawFd;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const PATH: &str = "/org/mpris/MediaPlayer2";
const INTERFACE: &str = "io.github.Cibitka.Cibitube.Lyrics1";

struct TerminalScreen;

impl Drop for TerminalScreen {
    fn drop(&mut self) {
        let _ = write!(io::stdout(), "\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub(super) fn clean_line(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .filter(|character| !character.is_control())
        .take_while(|character| {
            used += character.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

fn terminal_size() -> (usize, usize) {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // ioctl writes a winsize into the supplied, initialized structure.
    let result = unsafe { libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    if result == 0 && size.ws_col > 0 && size.ws_row > 0 {
        (usize::from(size.ws_col), usize::from(size.ws_row))
    } else {
        (80, 24)
    }
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn render(
    snapshot: Option<&LyricsSnapshot>,
    lines: &[String],
    width: usize,
    height: usize,
) -> String {
    let width = width.saturating_sub(2);
    let mut rows = vec!["\x1b[1;32mCibitube\x1b[0m".to_owned(), String::new()];
    if let Some(snapshot) = snapshot.filter(|snapshot| !snapshot.track_id.is_empty()) {
        rows.push(clean_line(
            &format!("{} — {}", snapshot.title, snapshot.artist),
            width,
        ));
        rows.push(clean_line(
            &format!(
                "{} / {}  {}",
                clock(snapshot.position_secs),
                clock(snapshot.duration_secs),
                if snapshot.playing {
                    crate::i18n::text("Воспроизведение")
                } else {
                    crate::i18n::text("Пауза")
                }
            ),
            width,
        ));
        rows.push(String::new());
        if !snapshot.available {
            rows.push(clean_line(
                crate::i18n::text("Текст песни загружается или недоступен"),
                width,
            ));
        } else {
            if !snapshot.synced {
                rows.push(crate::i18n::text("Без синхронизации").into());
            }
            let count = height.saturating_sub(9).max(1);
            let start = snapshot
                .active
                .map_or(0, |active| active.saturating_sub(count / 2));
            for (index, line) in lines.iter().enumerate().skip(start).take(count) {
                let line = clean_line(line, width.saturating_sub(2));
                rows.push(if snapshot.active == Some(index) {
                    format!("\x1b[1;32m> {line}\x1b[0m")
                } else {
                    format!("\x1b[90m  {line}\x1b[0m")
                });
            }
        }
    } else {
        rows.push(clean_line(
            crate::i18n::text("Ожидание Cibitube. Откройте плеер и включите трек."),
            width,
        ));
    }
    rows.push(String::new());
    rows.push(clean_line(
        crate::i18n::text("Ctrl+C — закрыть текст, музыка продолжит играть"),
        width,
    ));
    let mut output = String::from("\x1b[H");
    for row in rows.into_iter().take(height.saturating_sub(1)) {
        output.push_str(&row);
        output.push_str("\x1b[K\r\n");
    }
    output.push_str("\x1b[J");
    output
}

async fn read_snapshot(
    connection: &zbus::Connection,
    selected: Option<&str>,
    known_track: &str,
    revision: u64,
) -> Result<(String, LyricsSnapshot), String> {
    let bus = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|error| error.to_string())?;
    let names = bus.list_names().await.map_err(|error| error.to_string())?;
    let mut names: Vec<_> = names
        .into_iter()
        .filter(|name| {
            name.as_str()
                .starts_with("org.mpris.MediaPlayer2.cibitube.")
        })
        .map(|name| name.to_string())
        .collect();
    names.sort_by_key(|name| selected != Some(name.as_str()));
    let mut paused = None;
    for name in names {
        let proxy = zbus::Proxy::new(connection, name.as_str(), PATH, INTERFACE)
            .await
            .map_err(|error| error.to_string())?;
        let (known, version) = if selected == Some(name.as_str()) {
            (known_track, revision)
        } else {
            ("", u64::MAX)
        };
        let reply: Result<String, _> = proxy.call("Snapshot", &(known, version)).await;
        if let Ok(reply) = reply {
            if let Ok(snapshot) = serde_json::from_str::<LyricsSnapshot>(&reply) {
                if snapshot.playing {
                    return Ok((name, snapshot));
                }
                if paused.is_none() {
                    paused = Some((name, snapshot));
                }
            }
        }
    }
    paused.ok_or_else(|| {
        crate::i18n::text("Ожидание Cibitube. Откройте плеер и включите трек.").into()
    })
}

pub(crate) async fn run(animated: bool) -> Result<(), String> {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    crate::i18n::set_language(if locale.starts_with("ru") { "ru" } else { "en" });
    let interactive = io::stdout().is_terminal();
    let _screen = if interactive {
        print!("\x1b[?1049h\x1b[?25l");
        io::stdout().flush().map_err(|error| error.to_string())?;
        Some(TerminalScreen)
    } else {
        None
    };
    let stop = tokio::signal::ctrl_c();
    tokio::pin!(stop);
    let mut connection = None;
    let mut selected = None;
    let mut known_track = String::new();
    let mut revision = u64::MAX;
    let mut lines = Vec::new();
    let mut previous = String::new();
    let mut animation = animated.then(crate::terminal_animation::Animation::new);
    loop {
        let read = async {
            if connection.is_none() {
                connection = Some(
                    zbus::Connection::session()
                        .await
                        .map_err(|error| error.to_string())?,
                );
            }
            read_snapshot(
                connection.as_ref().unwrap(),
                selected.as_deref(),
                &known_track,
                revision,
            )
            .await
        };
        let result = tokio::select! {
            _ = &mut stop => return Ok(()),
            result = tokio::time::timeout(Duration::from_secs(2), read) => result
                .map_err(|_| "Cibitube did not respond".to_owned()).and_then(|result| result),
        };
        let snapshot = match result {
            Ok((name, mut snapshot)) => {
                crate::i18n::set_language(if snapshot.english { "en" } else { "ru" });
                selected = Some(name);
                known_track.clone_from(&snapshot.track_id);
                revision = snapshot.revision;
                if let Some(updated) = snapshot.lines.take() {
                    lines = updated;
                }
                Some(snapshot)
            }
            Err(error) => {
                if !interactive {
                    return Err(error);
                }
                selected = None;
                known_track.clear();
                revision = u64::MAX;
                lines.clear();
                connection = None;
                None
            }
        };
        if !interactive {
            let snapshot = snapshot.unwrap();
            println!(
                "{} — {}\n",
                clean_line(&snapshot.title, usize::MAX),
                clean_line(&snapshot.artist, usize::MAX)
            );
            if !snapshot.available {
                println!(
                    "{}",
                    crate::i18n::text("Текст песни загружается или недоступен")
                );
            }
            for line in lines {
                println!("{}", clean_line(&line, usize::MAX));
            }
            return Ok(());
        }
        let next_poll = tokio::time::Instant::now() + Duration::from_millis(250);
        loop {
            let (width, height) = terminal_size();
            let output = animation
                .as_mut()
                .and_then(|animation| {
                    snapshot
                        .as_ref()
                        .and_then(|snapshot| animation.render(snapshot, &lines, width, height))
                })
                .unwrap_or_else(|| render(snapshot.as_ref(), &lines, width, height));
            if output != previous {
                io::stdout()
                    .write_all(output.as_bytes())
                    .map_err(|error| error.to_string())?;
                io::stdout().flush().map_err(|error| error.to_string())?;
                previous = output;
            }
            let frame = if animation
                .as_ref()
                .is_some_and(|animation| animation.is_transitioning())
            {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(250)
            };
            tokio::select! {
                _ = &mut stop => return Ok(()),
                _ = tokio::time::sleep_until(next_poll.min(tokio::time::Instant::now() + frame)) => {}
            }
            if tokio::time::Instant::now() >= next_poll {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_text_cannot_inject_control_sequences_and_respects_wide_characters() {
        assert_eq!(clean_line("歌詞abc", 5), "歌詞a");
        assert!(!clean_line("title\x1b[31m\r", 100).contains('\x1b'));
    }

    #[test]
    fn terminal_highlights_the_active_line_and_keeps_track_metadata() {
        let snapshot = LyricsSnapshot {
            track_id: "fixture".into(),
            title: "Song".into(),
            available: true,
            synced: true,
            active: Some(1),
            ..Default::default()
        };
        let output = render(
            Some(&snapshot),
            &["First".into(), "Second".into(), "Third".into()],
            80,
            24,
        );
        assert!(output.contains("Song"));
        assert!(output.contains("\x1b[1;32m> Second"));
        assert!(output.contains("  First"));
    }
}
