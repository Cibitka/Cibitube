use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct BrowserAuth;

#[derive(Clone, Copy, Debug)]
enum FirefoxReadError {
    SqliteUnavailable,
    Busy,
    Unreadable,
    Unsupported,
}

impl FirefoxReadError {
    fn message(self) -> &'static str {
        match self {
            Self::SqliteUnavailable => "Не удалось запустить sqlite3. Проверь, что он установлен.",
            Self::Busy => {
                "Firefox сейчас обновляет данные входа. Повтори подключение через несколько секунд."
            }
            Self::Unreadable => {
                "Нет доступа к базе Firefox. Можно подключиться через Cookie вручную."
            }
            Self::Unsupported => {
                "Не удалось распознать базу Firefox. Можно подключиться через Cookie вручную."
            }
        }
    }
}

impl BrowserAuth {
    /// Attempts to extract YouTube cookies automatically from the user's Firefox profile.
    pub fn firefox_sessions() -> Result<Vec<String>, String> {
        let mut search_dirs = Vec::new();
        if let Some(home) = dirs::home_dir() {
            search_dirs.push(home.join(".config/mozilla/firefox"));
            search_dirs.push(home.join(".mozilla/firefox"));
            search_dirs.push(home.join("Library/Application Support/Firefox/Profiles"));
        }
        if let Some(data_dir) = dirs::data_dir() {
            search_dirs.push(data_dir.join("Mozilla/Firefox/Profiles"));
        }

        let mut profiles: Vec<PathBuf> = search_dirs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("cookies.sqlite"))
            .filter(|path| path.is_file())
            .collect();
        profiles.sort_unstable();
        profiles.dedup();
        profiles.sort_by_cached_key(|path| {
            std::cmp::Reverse(
                path.with_file_name("cookies.sqlite-wal")
                    .metadata()
                    .and_then(|file| file.modified())
                    .ok()
                    .max(path.metadata().and_then(|file| file.modified()).ok()),
            )
        });
        if profiles.is_empty() {
            return Err("Профиль Firefox с файлом cookies.sqlite не найден.".into());
        }

        let mut read_profile = false;
        let mut first_error = None;
        let mut sessions = Vec::new();
        for path in profiles {
            match Self::read_profile(&path) {
                Ok(rows) => {
                    read_profile = true;
                    for session in Self::parse_sessions(&rows) {
                        if !sessions.contains(&session) {
                            sessions.push(session);
                        }
                    }
                }
                Err(FirefoxReadError::SqliteUnavailable) => {
                    return Err(FirefoxReadError::SqliteUnavailable.message().into());
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if !sessions.is_empty() {
            return Ok(sessions);
        }
        if !read_profile {
            return Err(first_error
                .unwrap_or(FirefoxReadError::Unreadable)
                .message()
                .into());
        }
        Err("В Firefox нет сохранённой сессии YouTube Music. Открой music.youtube.com в обычном окне Firefox, войди в аккаунт и повтори подключение.".into())
    }

    pub(crate) fn manual_session(input: &str) -> Result<(String, Option<usize>), String> {
        let input = input.trim();
        let mut cookies = Vec::new();
        let mut account = None;
        for line in input.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("cookie") {
                cookies.push(value.trim());
            } else if name.trim().eq_ignore_ascii_case("x-goog-authuser") {
                account = value.trim().parse().ok();
            }
        }
        let cookie = if cookies.is_empty() && input.lines().count() == 1 {
            input.to_owned()
        } else {
            cookies.join("; ")
        };
        if reqwest::header::HeaderValue::from_str(&cookie).is_err()
            || !crate::innertube::InnertubeClient::has_browser_auth(&cookie)
        {
            return Err(
                "Вставь Cookie из заголовков запроса к music.youtube.com после входа в аккаунт."
                    .into(),
            );
        }
        Ok((cookie, account))
    }

    fn read_profile(path: &Path) -> Result<String, FirefoxReadError> {
        match Self::query_profile(path) {
            Err(FirefoxReadError::Busy | FirefoxReadError::Unreadable) => {
                Self::query_snapshot(path)
            }
            result => result,
        }
    }

    fn query_snapshot(path: &Path) -> Result<String, FirefoxReadError> {
        fn stamp(path: &Path) -> io::Result<Option<(u64, std::time::SystemTime)>> {
            match fs::metadata(path) {
                Ok(metadata) => Ok(Some((metadata.len(), metadata.modified()?))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        let wal = path.with_file_name("cookies.sqlite-wal");
        let version = || Ok::<_, io::Error>((stamp(path)?, stamp(&wal)?));
        for _ in 0..3 {
            let before = version().map_err(|_| FirefoxReadError::Unreadable)?;
            if before.0.is_none() {
                return Err(FirefoxReadError::Unreadable);
            }
            let mut temp = tempfile::Builder::new();
            #[cfg(unix)]
            temp.permissions(fs::Permissions::from_mode(0o700));
            let temp = temp.tempdir().map_err(|_| FirefoxReadError::Unreadable)?;
            let copy = temp.path().join("cookies.sqlite");
            let copied = (|| {
                io::copy(&mut File::open(path)?, &mut File::create(&copy)?)?;
                if before.1.is_some() {
                    io::copy(
                        &mut File::open(&wal)?,
                        &mut File::create(temp.path().join("cookies.sqlite-wal"))?,
                    )?;
                }
                Ok::<_, io::Error>(())
            })();
            // A checkpoint can replace the WAL while Firefox is still running.
            if version().map_err(|_| FirefoxReadError::Unreadable)? != before {
                continue;
            }
            copied.map_err(|_| FirefoxReadError::Unreadable)?;
            return Self::query_profile(&copy);
        }
        Err(FirefoxReadError::Busy)
    }

    fn query_profile(path: &Path) -> Result<String, FirefoxReadError> {
        // Firefox schema 16 switched expiry from Unix seconds to milliseconds.
        let query = "SELECT originAttributes, name, value FROM moz_cookies
            WHERE ltrim(host, '.') IN ('youtube.com', 'music.youtube.com')
            AND expiry > CAST(strftime('%s', 'now') AS INTEGER) *
                (SELECT CASE WHEN user_version >= 16 THEN 1000 ELSE 1 END FROM pragma_user_version)
            AND path = '/'
            ORDER BY (ltrim(host, '.') = 'music.youtube.com') DESC, lastAccessed DESC;";
        let output = Command::new("sqlite3")
            .args([
                "-readonly",
                "-batch",
                "-noheader",
                "-list",
                "-separator",
                "\t",
                "-cmd",
                ".timeout 200",
            ])
            .arg(path)
            .arg(query)
            .output()
            .map_err(|_| FirefoxReadError::SqliteUnavailable)?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }

        // Classify diagnostics without exposing the browser profile's path or contents.
        let error = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if error.contains("locked") || error.contains("busy") {
            Err(FirefoxReadError::Busy)
        } else if error.contains("unable to open")
            || error.contains("readonly")
            || error.contains("read-only")
            || error.contains("disk i/o")
        {
            Err(FirefoxReadError::Unreadable)
        } else {
            Err(FirefoxReadError::Unsupported)
        }
    }

    fn parse_sessions(rows: &str) -> Vec<String> {
        let mut sessions = BTreeMap::<_, BTreeMap<_, _>>::new();
        for line in rows.lines() {
            let mut columns = line.splitn(3, '\t');
            if let (Some(context), Some(name), Some(value)) =
                (columns.next(), columns.next(), columns.next())
            {
                sessions
                    .entry(context)
                    .or_default()
                    .entry(name)
                    .or_insert(value);
            }
        }
        sessions
            .into_values()
            .filter(|cookies| {
                ["__Secure-3PAPISID", "SAPISID"]
                    .iter()
                    .any(|name| cookies.get(name).is_some_and(|value| !value.is_empty()))
            })
            .map(|cookies| {
                cookies
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::process::Stdio;

    #[test]
    fn manual_login_accepts_a_cookie_or_firefox_request_headers() {
        let cookie = "__Secure-3PAPISID=example; SID=fixture";
        assert_eq!(
            BrowserAuth::manual_session(cookie),
            Ok((cookie.to_owned(), None))
        );
        let headers = format!("POST /youtubei/v1/browse HTTP/2\r\nauthorization: ignored\r\nx-goog-authuser: 7\r\nCookie: {cookie}\r\n");
        assert_eq!(
            BrowserAuth::manual_session(&headers),
            Ok((cookie.to_owned(), Some(7)))
        );
        assert_eq!(
            BrowserAuth::manual_session(&format!("cookie: {cookie}")),
            Ok((cookie.to_owned(), None))
        );
        assert!(BrowserAuth::manual_session(
            "set-cookie: SID=fixture\norigin: https://example.com"
        )
        .is_err());
    }

    #[test]
    fn firefox_sessions_do_not_mix_containers() {
        let profile = "\tCONSENT\tdefault\n^userContextId=2\tSAPISID\tfixture\n^userContextId=2\tSID\tcontainer-fixture";
        let sessions = BrowserAuth::parse_sessions(profile);
        assert_eq!(sessions, ["SAPISID=fixture; SID=container-fixture"]);
        assert!(BrowserAuth::parse_sessions("\tLOGIN_INFO\tfixture").is_empty());
    }

    #[test]
    fn reads_firefox_expiry_in_seconds_and_milliseconds() {
        if Command::new("sqlite3").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        for (version, units) in [(15, 1), (16, 1000)] {
            let database = dir.path().join(format!("profile-{version}.sqlite"));
            let setup = format!("PRAGMA user_version={version};
                CREATE TABLE moz_cookies (originAttributes TEXT, name TEXT, value TEXT,
                    host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER);
                INSERT INTO moz_cookies VALUES
                    ('', 'SAPISID', 'stale', 'music.youtube.com', '/', (unixepoch()-3600)*{units}, 2),
                    ('', 'SAPISID', 'fresh', '.youtube.com', '/', (unixepoch()+3600)*{units}, 1),
                    ('', 'SAPISID', 'foreign', 'notyoutube.com', '/', (unixepoch()+3600)*{units}, 3),
                    ('^userContextId=2', '__Secure-3PAPISID', 'other', '.youtube.com', '/', (unixepoch()+3600)*{units}, 1);");
            assert!(Command::new("sqlite3")
                .arg(&database)
                .arg(setup)
                .status()
                .unwrap()
                .success());
            let rows = BrowserAuth::read_profile(&database).unwrap();
            assert_eq!(
                BrowserAuth::parse_sessions(&rows),
                ["SAPISID=fresh", "__Secure-3PAPISID=other"]
            );
        }
    }

    #[test]
    fn reads_committed_wal_rows_while_firefox_holds_an_exclusive_lock() {
        if Command::new("sqlite3").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cookies.sqlite");
        let setup = "PRAGMA locking_mode=EXCLUSIVE; PRAGMA journal_mode=WAL;
            PRAGMA wal_autocheckpoint=0; PRAGMA user_version=16;
            CREATE TABLE moz_cookies (originAttributes TEXT, name TEXT, value TEXT,
                host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER);
            INSERT INTO moz_cookies VALUES
                ('', 'SAPISID', 'wal', '.youtube.com', '/',
                (unixepoch()+3600)*1000, 1);
            SELECT 'READY';";
        let mut writer = Command::new("sqlite3")
            .args(["-batch", "-bail"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(writer.stdin.as_mut().unwrap(), "{setup}").unwrap();
        let mut output = io::BufReader::new(writer.stdout.take().unwrap());
        let mut line = String::new();
        while output.read_line(&mut line).unwrap() != 0 && line.trim() != "READY" {
            line.clear();
        }
        let direct_read = BrowserAuth::query_profile(&path);
        let imported = BrowserAuth::read_profile(&path);
        let status = writer.wait().unwrap();

        assert!(status.success());
        assert_eq!(line.trim(), "READY");
        assert!(matches!(direct_read, Err(FirefoxReadError::Busy)));
        assert_eq!(
            BrowserAuth::parse_sessions(&imported.unwrap()),
            ["SAPISID=wal"]
        );
    }
}
