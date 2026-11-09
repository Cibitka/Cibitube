use crate::browser_auth::BrowserAuth;
use crate::discord::{ActivityDetails, DiscordRpc};
use crate::downloader::Downloader;
use crate::innertube::InnertubeClient;
use crate::library::Library;
use crate::lyrics::LyricsClient;
use crate::models::{AccountProfile, ActiveTab, Lyrics, PlaybackInfo, Playlist, Track};
use crate::player::AudioPlayer;
use crate::settings::AppSettings;
use eframe::egui::{self, Color32, Pos2, RichText, Rounding, Stroke, Vec2};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;

enum AppEvent {
    SearchResults(Result<Vec<Track>, String>),
    LikedMusicLoaded(Result<Vec<Track>, String>),
    PlaylistsLoaded(Result<Vec<Playlist>, String>),
    PlaylistTracksLoaded {
        playlist_id: String,
        result: Result<Vec<Track>, String>,
    },
    LyricsLoaded(Option<Lyrics>),
    DownloadFinished {
        track: Track,
        result: Result<String, String>,
    },
    AccountsDiscovered(Vec<AccountProfile>),
    CookiesExtracted(Result<String, String>),
    UpdateCheckResult(Result<Option<crate::updater::UpdateInfo>, String>),
    UpdateApplied(Result<(), String>),
}

pub struct MetrolistApp {
    innertube: Arc<InnertubeClient>,
    lyrics_client: Arc<LyricsClient>,
    player: AudioPlayer,
    discord: Arc<DiscordRpc>,
    downloader: Downloader,
    library: Library,
    settings: AppSettings,

    // UI State
    active_tab: ActiveTab,
    previous_tab: ActiveTab,
    repeat_mode: usize,
    shuffle_state: bool,
    settings_menu_tab: usize,
    show_lyrics_in_player: bool,
    search_query: String,
    search_results: Vec<Track>,
    is_searching: bool,
    status_message: Option<String>,
    download_status: Option<String>,

    // Updater State
    update_info: Option<crate::updater::UpdateInfo>,
    is_checking_update: bool,
    is_applying_update: bool,
    update_status: Option<String>,
    show_update_confirm: bool,

    // Playlists state
    playlists: Vec<Playlist>,
    selected_playlist: Option<Playlist>,
    playlist_tracks: Vec<Track>,
    is_loading_playlists: bool,
    is_loading_playlist_tracks: bool,

    // Queue & Playback
    queue: Vec<Track>,
    current_index: Option<usize>,
    lyrics: Option<Lyrics>,

    // Channels
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    sync_status_message: Option<String>,
    last_auto_sync: std::time::Instant,
}

impl MetrolistApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let (event_tx, event_rx) = mpsc::channel();
        let (downloader, dl_rx) = Downloader::new();
        let dl_tx = event_tx.clone();
        std::thread::spawn(move || {
            while let Ok((t, res)) = dl_rx.recv() {
                let _ = dl_tx.send(AppEvent::DownloadFinished {
                    track: t,
                    result: res,
                });
            }
        });

        let innertube = Arc::new(InnertubeClient::new());
        let lyrics_client = Arc::new(LyricsClient::new());
        let settings = AppSettings::load();
        let player = AudioPlayer::with_volume(settings.volume)
            .unwrap_or_else(|_| AudioPlayer::new().expect("Failed to initialize MPV player"));
        player.set_speed(settings.playback_speed, settings.preserve_pitch);
        let discord = Arc::new(DiscordRpc::new());
        let library = Library::load();

        let default_tab = match settings.default_nav_tab {
            1 => ActiveTab::Playlists,
            2 => ActiveTab::Liked,
            3 => ActiveTab::Search,
            _ => ActiveTab::Playlists,
        };

        let app = Self {
            innertube,
            lyrics_client,
            player,
            discord,
            downloader,
            library,
            settings,
            active_tab: default_tab,
            previous_tab: default_tab,
            repeat_mode: 0,
            shuffle_state: false,
            settings_menu_tab: 0,
            show_lyrics_in_player: false,
            search_query: String::new(),
            search_results: Vec::new(),
            is_searching: false,
            status_message: None,
            download_status: None,
            update_info: None,
            is_checking_update: false,
            is_applying_update: false,
            update_status: None,
            show_update_confirm: false,
            playlists: Vec::new(),
            selected_playlist: None,
            playlist_tracks: Vec::new(),
            is_loading_playlists: false,
            is_loading_playlist_tracks: false,
            queue: Vec::new(),
            current_index: None,
            lyrics: None,
            event_tx,
            event_rx,
            sync_status_message: None,
            last_auto_sync: std::time::Instant::now(),
        };

        app.apply_theme(&cc.egui_ctx);
        app.player.set_volume(app.settings.volume);

        // Continuous heartbeat thread: wakes up Wayland/Hyprland winit event loop every 50ms,
        // completely preventing compositor ANR / "window not responding" timeouts on inactive workspaces
        let heartbeat_ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            heartbeat_ctx.request_repaint();
        });

        // Check for updates in background on startup
        let tx_up = app.event_tx.clone();
        tokio::spawn(async move {
            let res = crate::updater::Updater::check_for_update().await;
            let _ = tx_up.send(AppEvent::UpdateCheckResult(res));
        });

        // Non-blocking async cookie extraction on startup
        let tx = app.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let res = BrowserAuth::extract_firefox_cookies();
            let _ = tx.send(AppEvent::CookiesExtracted(res));
        });

        // Auto-discover / refresh accounts and playlists if cookie exists
        if !app.settings.google_cookie.is_empty() {
            let client1 = Arc::clone(&app.innertube);
            let client2 = Arc::clone(&app.innertube);
            let cookie1 = app.settings.google_cookie.clone();
            let cookie2 = app.settings.google_cookie.clone();
            let auth_user = app.settings.auth_user_index;
            let tx1 = app.event_tx.clone();
            let tx2 = app.event_tx.clone();

            tokio::spawn(async move {
                let accounts = client1.discover_accounts(&cookie1).await;
                let _ = tx1.send(AppEvent::AccountsDiscovered(accounts));
            });

            tokio::spawn(async move {
                let pls = client2.fetch_user_playlists(&cookie2, auth_user).await;
                let _ = tx2.send(AppEvent::PlaylistsLoaded(pls));
            });
        }

        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let accent = self.accent_color();
        let mut visuals = match self.settings.theme_mode {
            0 => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = Color32::from_rgb(0, 0, 0);
                v.window_fill = Color32::from_rgb(10, 10, 10);
                v.extreme_bg_color = Color32::from_rgb(0, 0, 0);
                v.widgets.noninteractive.bg_fill = Color32::from_rgb(18, 18, 20);
                v.widgets.inactive.bg_fill = Color32::from_rgb(26, 26, 30);
                v.widgets.hovered.bg_fill = Color32::from_rgb(40, 40, 48);
                v
            }
            1 => {
                let mut v = egui::Visuals::dark();
                v.panel_fill = Color32::from_rgb(18, 18, 22);
                v.window_fill = Color32::from_rgb(24, 24, 30);
                v.extreme_bg_color = Color32::from_rgb(12, 12, 16);
                v.widgets.noninteractive.bg_fill = Color32::from_rgb(28, 28, 36);
                v.widgets.inactive.bg_fill = Color32::from_rgb(32, 32, 42);
                v.widgets.hovered.bg_fill = Color32::from_rgb(45, 45, 58);
                v
            }
            _ => egui::Visuals::light(),
        };

        visuals.widgets.active.bg_fill = accent;
        visuals.selection.bg_fill = accent;
        ctx.set_visuals(visuals);
    }

    fn accent_colors() -> [Color32; 5] {
        [
            Color32::from_rgb(16, 185, 129), // Emerald (Metrolist Default)
            Color32::from_rgb(244, 63, 94),  // Rose / Ruby
            Color32::from_rgb(168, 85, 247), // Violet / Amethyst
            Color32::from_rgb(14, 165, 233), // Sky / Cyberpunk
            Color32::from_rgb(245, 158, 11), // Amber Gold
        ]
    }

    fn accent_color(&self) -> Color32 {
        if self.settings.use_custom_accent {
            let [r, g, b] = self.settings.custom_accent_color;
            Color32::from_rgb(r, g, b)
        } else {
            Self::accent_colors()[self
                .settings
                .accent_color_idx
                .min(Self::accent_colors().len() - 1)]
        }
    }

    fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.queue.get(idx))
    }

    fn play_track(&mut self, track: Track) {
        let url = format!("https://www.youtube.com/watch?v={}", track.id);
        self.player.play_url(&url);

        // Update library history
        self.library.add_history(track.clone());

        // Update Discord Rich Presence if enabled
        if self.settings.discord_rpc_enabled {
            self.discord.set_activity(ActivityDetails {
                title: track.title.clone(),
                artist: track.artist.clone(),
                song_id: track.id.clone(),
                elapsed_secs: 0.0,
                duration_secs: 0.0,
                thumbnail: track.thumbnail_url.clone(),
                show_buttons: self.settings.discord_show_buttons,
                activity_type: self.settings.discord_activity_type,
            });
        }

        // Fetch lyrics in background
        let client = Arc::clone(&self.lyrics_client);
        let title = track.title.clone();
        let artist = track.artist.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch(&title, &artist).await;
            let _ = tx.send(AppEvent::LyricsLoaded(res));
        });

        // Add to queue if not present
        if let Some(pos) = self.queue.iter().position(|t| t.id == track.id) {
            self.current_index = Some(pos);
        } else {
            self.queue.push(track);
            self.current_index = Some(self.queue.len() - 1);
        }
        self.lyrics = None;
        self.last_auto_sync = std::time::Instant::now();
    }

    fn shuffle_slice(slice: &mut [Track]) {
        if slice.len() <= 1 {
            return;
        }
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        for i in (1..slice.len()).rev() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (seed >> 32) as usize % (i + 1);
            slice.swap(i, j);
        }
    }

    fn shuffle_queue(&mut self) {
        if self.queue.len() <= 1 {
            return;
        }

        if let Some(curr_idx) = self.current_index {
            if curr_idx < self.queue.len() {
                // Сохраняем текущий играющий трек, чтобы обложка, звук и позиция не сбивались
                let current_track = self.queue.remove(curr_idx);
                // Перемешиваем остальные треки в очереди
                Self::shuffle_slice(&mut self.queue);
                // Возвращаем текущий трек на первое место (индекс 0)
                self.queue.insert(0, current_track);
                self.current_index = Some(0);
                return;
            }
        }

        Self::shuffle_slice(&mut self.queue);
    }

    fn play_next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let next_idx = match self.current_index {
            Some(i) => {
                if i + 1 < self.queue.len() {
                    i + 1
                } else if self.repeat_mode == 1 {
                    0
                } else {
                    return;
                }
            }
            None => 0,
        };
        self.current_index = Some(next_idx);
        let track = self.queue[next_idx].clone();
        self.play_track(track);
    }

    fn play_prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let prev_idx = match self.current_index {
            Some(i) => {
                if i > 0 {
                    i - 1
                } else if self.repeat_mode == 1 {
                    self.queue.len() - 1
                } else {
                    0
                }
            }
            None => 0,
        };
        self.current_index = Some(prev_idx);
        let track = self.queue[prev_idx].clone();
        self.play_track(track);
    }

    fn download_playlist(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            self.download_status = Some("В плейлисте нет треков для скачивания".to_string());
            return;
        }
        let total = tracks.len();
        self.download_status = Some(format!("Добавлено в очередь скачивания: {} треков", total));
        for track in tracks {
            self.download_track(track);
        }
    }

    fn execute_search(&mut self) {
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.is_searching = true;
        self.status_message = Some("Поиск...".to_string());
        let client = Arc::clone(&self.innertube);
        let country = self.settings.content_country.clone();
        let cookie = self.settings.google_cookie.clone();
        let auth_user = self.settings.auth_user_index;
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.search(&q, &country, &cookie, auth_user).await;
            let _ = tx.send(AppEvent::SearchResults(res));
        });
    }

    fn sync_playlists(&mut self) {
        if self.settings.google_cookie.is_empty() {
            self.status_message =
                Some("Войдите в аккаунт для синхронизации плейлистов".to_string());
            return;
        }
        self.is_loading_playlists = true;
        self.status_message = Some("Загрузка плейлистов из YouTube Music...".to_string());
        let client = Arc::clone(&self.innertube);
        let cookie = self.settings.google_cookie.clone();
        let auth_user = self.settings.auth_user_index;
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch_user_playlists(&cookie, auth_user).await;
            let _ = tx.send(AppEvent::PlaylistsLoaded(res));
        });
    }

    fn open_playlist(&mut self, pl: Playlist) {
        self.selected_playlist = Some(pl.clone());
        self.is_loading_playlist_tracks = true;
        self.playlist_tracks.clear();

        let client = Arc::clone(&self.innertube);
        let cookie = self.settings.google_cookie.clone();
        let auth_user = self.settings.auth_user_index;
        let pid = pl.id.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch_playlist_tracks(&pid, &cookie, auth_user).await;
            let _ = tx.send(AppEvent::PlaylistTracksLoaded {
                playlist_id: pid,
                result: res,
            });
        });
    }

    fn download_track(&mut self, track: Track) {
        self.download_status = Some(format!("Скачивание: {}", track.title));
        let opts = crate::downloader::DownloadOptions::from_settings(&self.settings);
        self.downloader.download_with(track, opts);
    }

    fn format_time(seconds: f64) -> String {
        if seconds.is_nan() || seconds < 0.0 {
            return "00:00".to_string();
        }
        let total_secs = seconds as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    fn trigger_auto_sync(&self, playback_info: &PlaybackInfo) {
        if !self.settings.cross_device_sync_enabled {
            return;
        }
        let Some(track) = self.current_track().cloned() else {
            return;
        };
        let folder = self.settings.google_drive_sync_folder.clone();
        let pos = playback_info.position_secs;
        let dur = playback_info.duration_secs;
        let is_playing = playback_info.is_playing;
        let speed = self.settings.playback_speed;
        let queue = self.queue.clone();
        let current_idx = self.current_index.unwrap_or(0);
        let repeat_str = if self.repeat_mode == 1 {
            "one".to_string()
        } else if self.repeat_mode == 2 {
            "all".to_string()
        } else {
            "off".to_string()
        };
        let shuffle = self.shuffle_state;

        std::thread::spawn(move || {
            let _ = crate::sync::SyncManager::save_state_with_queue(
                &folder,
                &track,
                pos,
                dur,
                is_playing,
                speed,
                &queue,
                current_idx,
                &repeat_str,
                shuffle,
            );
        });
    }

    fn render_playback_slider(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        height: f32,
        playback_info: &PlaybackInfo,
    ) {
        let max_d = if playback_info.duration_secs > 0.0 {
            playback_info.duration_secs
        } else {
            100.0
        };
        let pos = playback_info.position_secs.clamp(0.0, max_d);
        let fraction = if max_d > 0.0 {
            (pos / max_d).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };

        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click_and_drag());

        // Seeking interaction
        if resp.clicked() || resp.dragged() {
            if let Some(mouse_pos) = resp.interact_pointer_pos() {
                let frac = ((mouse_pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let seek_target = frac as f64 * max_d;
                self.player.seek(seek_target);
                self.last_auto_sync = std::time::Instant::now();
                self.trigger_auto_sync(playback_info);
            }
        }

        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let accent = self.accent_color();
        let unplayed_color = Color32::from_rgb(50, 50, 60);
        let center_y = rect.center().y;
        let played_x = rect.left() + fraction * rect.width();

        let painter = ui.painter();

        match self.settings.slider_style {
            // 1 = Волнистый (Змейка) / Squiggly seekbar
            1 => {
                // If song is playing, animate the wave phase!
                if playback_info.is_playing {
                    ui.ctx().request_repaint();
                }

                let time_sec = ui.input(|i| i.time) as f32;
                let phase = if playback_info.is_playing {
                    time_sec * 5.0
                } else {
                    0.0
                };
                let wave_len = 22.0_f32; // wave period length
                let amplitude = if playback_info.is_playing {
                    3.5_f32
                } else {
                    2.5_f32
                };

                // Draw unplayed straight line on the right
                if played_x < rect.right() {
                    painter.line_segment(
                        [
                            Pos2::new(played_x, center_y),
                            Pos2::new(rect.right(), center_y),
                        ],
                        Stroke::new(3.5_f32, unplayed_color),
                    );
                }

                // Draw animated sine wave for played portion
                if played_x > rect.left() {
                    let step = 2.0_f32;
                    let mut prev_pt = Pos2::new(rect.left(), center_y);

                    let mut x = rect.left() + step;
                    while x <= played_x {
                        // Smoothly taper amplitude at the very start and near the thumb
                        let dist_from_start = x - rect.left();
                        let dist_from_end = played_x - x;
                        let taper = (dist_from_start / 10.0)
                            .min(dist_from_end / 10.0)
                            .clamp(0.0, 1.0);

                        let current_amp = amplitude * taper;
                        let y = center_y
                            + ((dist_from_start / wave_len) * std::f32::consts::TAU - phase).sin()
                                * current_amp;
                        let curr_pt = Pos2::new(x, y);

                        painter.line_segment([prev_pt, curr_pt], Stroke::new(3.5_f32, accent));
                        prev_pt = curr_pt;
                        x += step;
                    }

                    if prev_pt.x < played_x {
                        painter.line_segment(
                            [prev_pt, Pos2::new(played_x, center_y)],
                            Stroke::new(3.5_f32, accent),
                        );
                    }
                }

                // Thumb circle
                let thumb_radius = if resp.hovered() || resp.dragged() {
                    7.5
                } else {
                    6.0
                };
                painter.circle_filled(Pos2::new(played_x, center_y), thumb_radius, Color32::WHITE);
                painter.circle_stroke(
                    Pos2::new(played_x, center_y),
                    thumb_radius,
                    Stroke::new(2.0_f32, accent),
                );
            }

            // 2 = Тонкий (Minimal thin line)
            2 => {
                // Background track
                painter.line_segment(
                    [
                        Pos2::new(rect.left(), center_y),
                        Pos2::new(rect.right(), center_y),
                    ],
                    Stroke::new(2.0_f32, unplayed_color),
                );
                // Played track
                if played_x > rect.left() {
                    painter.line_segment(
                        [
                            Pos2::new(rect.left(), center_y),
                            Pos2::new(played_x, center_y),
                        ],
                        Stroke::new(2.5_f32, accent),
                    );
                }
                // Small thumb
                let thumb_radius = if resp.hovered() || resp.dragged() {
                    5.0
                } else {
                    3.5
                };
                painter.circle_filled(Pos2::new(played_x, center_y), thumb_radius, accent);
                painter.circle_stroke(
                    Pos2::new(played_x, center_y),
                    thumb_radius,
                    Stroke::new(1.0_f32, Color32::WHITE),
                );
            }

            // 0 = Прямой (Classic standard bar)
            _ => {
                // Background unplayed track
                painter.line_segment(
                    [
                        Pos2::new(rect.left(), center_y),
                        Pos2::new(rect.right(), center_y),
                    ],
                    Stroke::new(4.0_f32, unplayed_color),
                );
                // Played track
                if played_x > rect.left() {
                    painter.line_segment(
                        [
                            Pos2::new(rect.left(), center_y),
                            Pos2::new(played_x, center_y),
                        ],
                        Stroke::new(4.0_f32, accent),
                    );
                }
                // Thumb circle
                let thumb_radius = if resp.hovered() || resp.dragged() {
                    7.0
                } else {
                    5.5
                };
                painter.circle_filled(Pos2::new(played_x, center_y), thumb_radius, Color32::WHITE);
                painter.circle_stroke(
                    Pos2::new(played_x, center_y),
                    thumb_radius,
                    Stroke::new(1.5_f32, accent),
                );
            }
        }
    }

    fn draw_share_icon(ui: &mut egui::Ui, hover_color: Color32) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), egui::Sense::click());
        let color = if resp.hovered() {
            hover_color
        } else {
            Color32::from_rgb(200, 200, 215)
        };
        let painter = ui.painter();
        let center = rect.center();

        // Standard sleek vector share icon: 3 circular nodes connected by 2 lines
        let p_left = Pos2::new(center.x - 6.5, center.y);
        let p_top_right = Pos2::new(center.x + 6.5, center.y - 6.5);
        let p_bot_right = Pos2::new(center.x + 6.5, center.y + 6.5);

        let stroke = Stroke::new(1.8_f32, color);
        painter.line_segment([p_left, p_top_right], stroke);
        painter.line_segment([p_left, p_bot_right], stroke);

        let node_r = 3.0_f32;
        painter.circle_filled(p_left, node_r, color);
        painter.circle_filled(p_top_right, node_r, color);
        painter.circle_filled(p_bot_right, node_r, color);

        resp
    }
}

impl eframe::App for MetrolistApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let playback_info = self
            .player
            .info
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        // Repaint request for smooth progress animation
        ctx.request_repaint_after(std::time::Duration::from_millis(20));

        // Periodic auto-sync to Google Drive (every ~4 seconds during playback)
        if self.settings.cross_device_sync_enabled
            && playback_info.is_playing
            && self.last_auto_sync.elapsed() >= std::time::Duration::from_secs(4)
        {
            self.last_auto_sync = std::time::Instant::now();
            self.trigger_auto_sync(&playback_info);
        }

        // Poll events
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::SearchResults(res) => {
                    self.is_searching = false;
                    match res {
                        Ok(tracks) => {
                            self.status_message = Some(format!("Найдено треков: {}", tracks.len()));
                            self.search_results = tracks;
                        }
                        Err(err) => {
                            self.status_message = Some(format!("Ошибка: {}", err));
                        }
                    }
                }
                AppEvent::LikedMusicLoaded(res) => match res {
                    Ok(tracks) => {
                        let count = tracks.len();
                        for t in tracks {
                            if !self.library.is_liked(&t.id) {
                                self.library.liked_tracks.push(t);
                            }
                        }
                        self.library.save();
                        self.status_message =
                            Some(format!("Синхронизировано любимых треков: {}", count));
                    }
                    Err(err) => {
                        self.status_message = Some(err);
                    }
                },
                AppEvent::PlaylistsLoaded(res) => {
                    self.is_loading_playlists = false;
                    match res {
                        Ok(pls) => {
                            let count = pls.len();
                            self.playlists = pls;
                            self.status_message =
                                Some(format!("Синхронизировано плейлистов: {}", count));
                        }
                        Err(err) => {
                            self.status_message = Some(format!("Ошибка плейлистов: {}", err));
                        }
                    }
                }
                AppEvent::PlaylistTracksLoaded {
                    playlist_id,
                    result,
                } => {
                    self.is_loading_playlist_tracks = false;
                    if self.selected_playlist.as_ref().map(|p| p.id.as_str()) == Some(&playlist_id)
                    {
                        match result {
                            Ok(tracks) => {
                                self.playlist_tracks = tracks;
                            }
                            Err(err) => {
                                self.status_message =
                                    Some(format!("Ошибка треков плейлиста: {}", err));
                            }
                        }
                    }
                }
                AppEvent::LyricsLoaded(lyrics) => {
                    self.lyrics = lyrics;
                }
                AppEvent::DownloadFinished { track, result } => match result {
                    Ok(msg) => {
                        self.download_status = Some(format!("✓ {} ({})", track.title, msg));
                        self.library.add_downloaded(track);
                    }
                    Err(err) => {
                        self.download_status = Some(format!("Ошибка скачивания: {}", err));
                    }
                },
                AppEvent::AccountsDiscovered(accounts) => {
                    let total = accounts.len();
                    self.settings.available_accounts = accounts;

                    // Ensure selected index is valid
                    let current_user = self.settings.auth_user_index;
                    if let Some(active) = self
                        .settings
                        .available_accounts
                        .iter()
                        .find(|a| a.auth_user == current_user)
                    {
                        self.settings.account_name = Some(active.name.clone());
                    } else if let Some(first) = self.settings.available_accounts.first() {
                        self.settings.auth_user_index = first.auth_user;
                        self.settings.account_name = Some(first.name.clone());
                    }

                    self.settings.save();
                    if let Some(name) = &self.settings.account_name {
                        self.status_message =
                            Some(format!("Аккаунтов Google: {}. Активен: {}", total, name));
                    }
                    if self.playlists.is_empty() {
                        self.sync_playlists();
                    }
                }
                AppEvent::CookiesExtracted(res) => match res {
                    Ok(fresh_cookie) => {
                        if !fresh_cookie.is_empty() {
                            let was_empty = self.settings.google_cookie.is_empty();
                            self.settings.google_cookie = fresh_cookie.clone();
                            self.settings.save();
                            if was_empty {
                                self.status_message =
                                    Some("Куки из Firefox успешно получены!".to_string());
                            }
                            let client1 = Arc::clone(&self.innertube);
                            let client2 = Arc::clone(&self.innertube);
                            let auth_user = self.settings.auth_user_index;
                            let tx1 = self.event_tx.clone();
                            let tx2 = self.event_tx.clone();
                            let cookie_for_acc = fresh_cookie.clone();
                            let cookie_for_pls = fresh_cookie;
                            tokio::spawn(async move {
                                let accounts = client1.discover_accounts(&cookie_for_acc).await;
                                let _ = tx1.send(AppEvent::AccountsDiscovered(accounts));
                            });
                            tokio::spawn(async move {
                                let pls = client2
                                    .fetch_user_playlists(&cookie_for_pls, auth_user)
                                    .await;
                                let _ = tx2.send(AppEvent::PlaylistsLoaded(pls));
                            });
                        }
                    }
                    Err(err) => {
                        if self.settings.google_cookie.is_empty() {
                            self.status_message = Some(format!("Firefox: {}", err));
                        }
                    }
                },
                AppEvent::UpdateCheckResult(res) => {
                    self.is_checking_update = false;
                    match res {
                        Ok(Some(info)) => {
                            self.status_message =
                                Some(format!("Доступна новая версия: {}!", info.version));
                            self.update_info = Some(info);
                        }
                        Ok(None) => {
                            self.update_status =
                                Some("У вас установлена самая последняя версия".to_string());
                        }
                        Err(e) => {
                            self.update_status = Some(format!("Ошибка проверки: {}", e));
                        }
                    }
                }
                AppEvent::UpdateApplied(res) => {
                    self.is_applying_update = false;
                    match res {
                        Ok(()) => {
                            self.update_status = Some(
                                "✓ Обновление успешно установлено! Старая версия удалена."
                                    .to_string(),
                            );
                        }
                        Err(e) => {
                            self.update_status = Some(format!("Ошибка установки: {}", e));
                        }
                    }
                }
            }
        }

        // Auto next track with repeat mode support
        if self.player.track_finished.swap(false, Ordering::SeqCst) {
            if self.repeat_mode == 2 {
                if let Some(t) = self.current_track() {
                    self.play_track(t.clone());
                }
            } else {
                self.play_next();
            }
        }

        let accent = self.accent_color();

        // 1. Sidebar (Left panel - hidden in fullscreen NowPlaying mode)
        let show_sidebar = self.active_tab != ActiveTab::NowPlaying;
        if show_sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(false)
                .exact_width(if self.settings.slim_nav_bar {
                    180.0
                } else {
                    220.0
                })
                .show(ctx, |ui| {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("♫").size(24.0).color(accent));
                        ui.vertical(|ui| {
                            ui.label(RichText::new("CIBITUBE").size(18.0).strong().color(accent));
                            ui.label(RichText::new("Взято за идею Metrolist").size(10.0).weak());
                        });
                    });

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Navigation buttons with reliable Unicode icons
                    let strings = crate::i18n::tr(&self.settings.ui_language);
                    let tabs = [
                        (ActiveTab::Search, format!("🔍  {}", strings.nav_search)),
                        (
                            ActiveTab::Liked,
                            if self.settings.ui_language == "en" {
                                "♥  Liked Songs".to_string()
                            } else {
                                "♥  Любимые треки".to_string()
                            },
                        ),
                        (ActiveTab::Playlists, format!("📑  {}", strings.nav_library)),
                        (
                            ActiveTab::Queue,
                            if self.settings.ui_language == "en" {
                                "☰  Queue".to_string()
                            } else {
                                "☰  Очередь".to_string()
                            },
                        ),
                        (
                            ActiveTab::Lyrics,
                            format!("♫  {}", strings.player_lyrics_tab),
                        ),
                        (
                            ActiveTab::Downloads,
                            format!("⬇  {}", strings.nav_downloads),
                        ),
                        (ActiveTab::Settings, format!("⚙  {}", strings.nav_settings)),
                    ];

                    for (tab, label) in tabs {
                        let is_active = self.active_tab == tab;
                        let text = RichText::new(label).size(13.0);
                        let button = if is_active {
                            egui::Button::new(text.color(Color32::WHITE).strong())
                                .fill(accent)
                                .rounding(Rounding::same(8.0))
                        } else {
                            egui::Button::new(text.color(Color32::from_rgb(200, 200, 210)))
                                .fill(Color32::TRANSPARENT)
                                .rounding(Rounding::same(8.0))
                        };

                        if ui.add_sized([ui.available_width(), 34.0], button).clicked() {
                            self.active_tab = tab;
                        }
                        ui.add_space(3.0);
                    }

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Account quick info badge in sidebar
                    if let Some(acc) = &self.settings.account_name {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("●")
                                    .size(10.0)
                                    .color(Color32::from_rgb(16, 185, 129)),
                            );
                            ui.label(RichText::new(acc).size(11.0).strong().color(Color32::WHITE));
                        });
                        ui.add_space(6.0);
                    }

                    if let Some(msg) = &self.status_message {
                        ui.label(RichText::new(msg).size(11.0).color(accent));
                    }

                    if let Some(dl) = &self.download_status {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(dl)
                                .size(10.0)
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                    }

                    if let Some(info) = &self.update_info {
                        ui.add_space(8.0);
                        let up_btn = egui::Button::new(
                            RichText::new(format!("● Обновление {}", info.version))
                                .size(11.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(16, 185, 129))
                        .rounding(Rounding::same(6.0));
                        if ui
                            .add(up_btn)
                            .on_hover_text("Нажмите для перехода в меню обновлений")
                            .clicked()
                        {
                            self.active_tab = ActiveTab::Settings;
                            self.settings_menu_tab = 10;
                        }
                    }

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Cibitube • Взято за идею Metrolist")
                                .size(10.0)
                                .weak(),
                        );
                    });
                });
        }

        // 2. Bottom Player Bar (only visible when not in full NowPlaying tab)
        if self.active_tab != ActiveTab::NowPlaying {
            let bar_bg = if self.settings.mini_player_bg_style == 1
                || self.settings.mini_player_pure_black
            {
                Color32::from_rgb(0, 0, 0)
            } else if self.settings.mini_player_bg_style == 0 {
                Color32::from_rgb(18, 18, 24)
            } else {
                Color32::from_rgb(24, 24, 30)
            };

            let current_track_clone = self.current_track().cloned();
            let mut open_now_playing = false;
            let mut track_to_toggle_like = None;

            egui::TopBottomPanel::bottom("player_bar")
                .exact_height(82.0)
                .frame(egui::Frame::none().fill(bar_bg).inner_margin(egui::Margin {
                    left: 0.0,
                    right: 0.0,
                    top: 0.0,
                    bottom: 6.0,
                }))
                .show(ctx, |ui| {
                    if !self.player.is_alive() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("⚠️ MPV плеер завершился")
                                    .color(Color32::from_rgb(239, 68, 68))
                                    .strong(),
                            );
                            if ui.button("🔄 Перезапустить плеер").clicked() {
                                if let Ok(new_player) =
                                    AudioPlayer::with_volume(self.settings.volume)
                                {
                                    self.player = new_player;
                                }
                            }
                        });
                    }

                    // 1. Линия длины трека по всей ширине окна (полная длина)
                    let total_bar_w = ui.available_width();
                    self.render_playback_slider(ui, total_bar_w, 10.0, &playback_info);

                    ui.add_space(2.0);

                    // 2. Строка управления под прогресс-баром
                    let row_w = ui.available_width();
                    let left_w = (row_w * 0.34).clamp(180.0, 360.0);
                    let right_w = 180.0;
                    let center_w = (row_w - left_w - right_w - 20.0).max(160.0);

                    ui.horizontal(|ui| {
                        ui.add_space(14.0);

                        // Слева: Обложка + Название и Автор
                        let rounding = Rounding::same(self.settings.album_art_rounding);
                        ui.allocate_ui_with_layout(
                            Vec2::new(left_w, 60.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if let Some(track) = &current_track_clone {
                                    let img_resp = if let Some(thumb) = &track.thumbnail_url {
                                        ui.add(
                                            egui::Image::new(thumb)
                                                .fit_to_exact_size(Vec2::new(48.0, 48.0))
                                                .rounding(rounding),
                                        )
                                    } else {
                                        let (rect, resp) = ui.allocate_exact_size(
                                            Vec2::new(48.0, 48.0),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().rect_filled(
                                            rect,
                                            rounding,
                                            Color32::from_rgb(40, 40, 50),
                                        );
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "♫",
                                            egui::FontId::proportional(20.0),
                                            Color32::GRAY,
                                        );
                                        resp
                                    };

                                    if img_resp.clicked() {
                                        open_now_playing = true;
                                    }

                                    ui.add_space(8.0);

                                    let text_resp = ui
                                        .vertical(|ui| {
                                            ui.label(
                                                RichText::new(&track.title)
                                                    .size(13.0)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.label(
                                                RichText::new(&track.artist).size(11.0).weak(),
                                            );
                                        })
                                        .response;

                                    if text_resp.clicked() {
                                        open_now_playing = true;
                                    }
                                } else {
                                    ui.label(RichText::new("Ничего не воспроизводится").weak());
                                }
                            },
                        );

                        // По центру: Кнопки управления (⏮ ▶/⏸ ⏭) и тайминг под ними
                        ui.allocate_ui_with_layout(
                            Vec2::new(center_w, 60.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.horizontal(|ui| {
                                    let prev_btn = egui::Button::new(
                                        RichText::new("⏮")
                                            .size(18.0)
                                            .strong()
                                            .color(Color32::from_rgb(210, 210, 220)),
                                    )
                                    .frame(false);
                                    if ui.add_sized([32.0, 32.0], prev_btn).clicked() {
                                        self.play_prev();
                                    }
                                    ui.add_space(14.0);

                                    let play_icon = if playback_info.is_playing {
                                        "⏸"
                                    } else {
                                        "▶"
                                    };
                                    let play_btn = egui::Button::new(
                                        RichText::new(play_icon)
                                            .size(18.0)
                                            .color(Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(Color32::from_rgb(29, 185, 84)) // Emerald Accent
                                    .rounding(Rounding::same(20.0));
                                    if ui.add_sized([40.0, 40.0], play_btn).clicked() {
                                        self.player.toggle_pause();
                                        self.last_auto_sync = std::time::Instant::now();
                                        self.trigger_auto_sync(&playback_info);
                                    }
                                    ui.add_space(14.0);

                                    let next_btn = egui::Button::new(
                                        RichText::new("⏭")
                                            .size(18.0)
                                            .strong()
                                            .color(Color32::from_rgb(210, 210, 220)),
                                    )
                                    .frame(false);
                                    if ui.add_sized([32.0, 32.0], next_btn).clicked() {
                                        self.play_next();
                                    }
                                });

                                ui.add_space(2.0);

                                let curr_str = Self::format_time(playback_info.position_secs);
                                let dur_str = if self.settings.show_remaining_time
                                    && playback_info.duration_secs > playback_info.position_secs
                                {
                                    format!(
                                        "-{}",
                                        Self::format_time(
                                            playback_info.duration_secs
                                                - playback_info.position_secs
                                        )
                                    )
                                } else {
                                    Self::format_time(playback_info.duration_secs)
                                };
                                let time_text = format!("{} / {}", curr_str, dur_str);
                                ui.label(RichText::new(time_text).size(11.0).weak());
                            },
                        );

                        // Справа: Кнопка «Сердечко» [ ♥ ], Громкость и Разворачивание
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(14.0);

                            // Кнопка разворачивания экрана воспроизведения
                            let expand_btn = egui::Button::new(
                                RichText::new("⛶")
                                    .size(16.0)
                                    .color(Color32::from_rgb(200, 200, 210)),
                            )
                            .frame(false);
                            if ui
                                .add(expand_btn)
                                .on_hover_text("Развернуть плеер")
                                .clicked()
                            {
                                open_now_playing = true;
                            }

                            ui.add_space(8.0);

                            let mut vol = playback_info.volume;
                            let vol_slider =
                                egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false);
                            if ui.add_sized([60.0, 12.0], vol_slider).changed() {
                                self.player.set_volume(vol);
                                self.settings.volume = vol;
                                self.settings.save();
                            }
                            ui.label(RichText::new("🔊").size(13.0));

                            ui.add_space(10.0);

                            // Перемещенная вправо кнопка «Сердечко» [ ♥ ]
                            if let Some(track) = &current_track_clone {
                                let is_liked = self.library.is_liked(&track.id);
                                let heart_color = if is_liked {
                                    Color32::from_rgb(244, 63, 94)
                                } else {
                                    Color32::from_rgb(160, 160, 175)
                                };
                                let heart_symbol = if is_liked { "♥" } else { "♡" };
                                let heart_btn = egui::Button::new(
                                    RichText::new(heart_symbol).size(15.0).color(heart_color),
                                )
                                .fill(Color32::from_rgb(44, 44, 52))
                                .rounding(Rounding::same(6.0));
                                if ui
                                    .add_sized([30.0, 26.0], heart_btn)
                                    .on_hover_text("Понравилось")
                                    .clicked()
                                {
                                    track_to_toggle_like = Some(track.clone());
                                }
                            }
                        });
                    });
                });

            if let Some(track) = track_to_toggle_like {
                self.library.toggle_like(track);
            }
            if open_now_playing {
                self.previous_tab = self.active_tab;
                self.active_tab = ActiveTab::NowPlaying;
            }
        }

        // 3. Central Content Panel
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            match self.active_tab {
                ActiveTab::Search => self.render_search_tab(ui),
                ActiveTab::Liked => self.render_liked_tab(ui),
                ActiveTab::Playlists => self.render_playlists_tab(ui),
                ActiveTab::Queue => self.render_queue_tab(ui),
                ActiveTab::Lyrics => self.render_lyrics_tab(ui, playback_info.position_secs),
                ActiveTab::Downloads => self.render_downloads_tab(ui),
                ActiveTab::Settings => self.render_settings_tab(ui, ctx),
                ActiveTab::NowPlaying => self.render_now_playing_tab(ui),
            }
        });
    }
}

impl MetrolistApp {
    fn render_search_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        // Search bar
        ui.horizontal(|ui| {
            let input = egui::TextEdit::singleline(&mut self.search_query)
                .hint_text("Поиск песен, альбомов или исполнителей...")
                .desired_width(ui.available_width() - 100.0);
            let resp = ui.add_sized([ui.available_width() - 100.0, 36.0], input);

            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.execute_search();
            }

            let search_btn = egui::Button::new(RichText::new("Найти").strong())
                .fill(accent)
                .rounding(Rounding::same(8.0));
            if ui.add_sized([90.0, 36.0], search_btn).clicked() {
                self.execute_search();
            }
        });

        ui.add_space(16.0);

        if self.is_searching {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label(RichText::new("Поиск треков на YouTube Music...").weak());
            });
            return;
        }

        if self.search_results.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    RichText::new("♫ Введите название трека или исполнителя для поиска")
                        .size(15.0)
                        .weak(),
                );
            });
            return;
        }

        // List results
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut track_idx_to_play = None;
                let mut track_to_queue = None;
                let mut track_to_download = None;

                for (idx, track) in self.search_results.iter().enumerate() {
                    let card_bg = Color32::from_rgb(26, 26, 34);
                    let rounding = Rounding::same(self.settings.album_art_rounding);
                    let frame = egui::Frame::none()
                        .fill(card_bg)
                        .rounding(Rounding::same(8.0))
                        .inner_margin(8.0)
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)));

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Thumbnail
                            if let Some(thumb) = &track.thumbnail_url {
                                ui.add(
                                    egui::Image::new(thumb)
                                        .fit_to_exact_size(Vec2::new(44.0, 44.0))
                                        .rounding(rounding),
                                );
                            } else {
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(44.0, 44.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    rounding,
                                    Color32::from_rgb(45, 45, 55),
                                );
                            }
                            ui.add_space(10.0);

                            // Title & Artist
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&track.title)
                                        .size(14.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} • {}",
                                        track.artist, track.duration_text
                                    ))
                                    .size(12.0)
                                    .weak(),
                                );
                            });

                            // Right action buttons
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let play_btn =
                                        egui::Button::new(RichText::new("▶ Играть").strong())
                                            .fill(accent)
                                            .rounding(Rounding::same(6.0));
                                    if ui.add(play_btn).clicked() {
                                        track_idx_to_play = Some(idx);
                                    }

                                    // Like button
                                    let is_liked = self.library.is_liked(&track.id);
                                    let heart_col = if is_liked {
                                        Color32::from_rgb(244, 63, 94)
                                    } else {
                                        Color32::GRAY
                                    };
                                    let heart_symbol = if is_liked { "♥" } else { "♡" };
                                    if ui
                                        .button(
                                            RichText::new(heart_symbol).size(15.0).color(heart_col),
                                        )
                                        .clicked()
                                    {
                                        self.library.toggle_like(track.clone());
                                    }

                                    // Add to queue
                                    if ui.button("+ Очередь").clicked() {
                                        track_to_queue = Some(track.clone());
                                    }

                                    // Download
                                    if ui.button("⬇").on_hover_text("Скачать MP3").clicked()
                                    {
                                        track_to_download = Some(track.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(6.0);
                }

                if let Some(idx) = track_idx_to_play {
                    self.queue = self.search_results.clone();
                    self.current_index = Some(idx);
                    let track = self.search_results[idx].clone();
                    self.play_track(track);
                }
                if let Some(t) = track_to_queue {
                    self.queue.push(t);
                    self.status_message = Some("Трек добавлен в очередь".to_string());
                }
                if let Some(t) = track_to_download {
                    self.download_track(t);
                }
            });
    }

    fn render_liked_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        ui.horizontal(|ui| {
            ui.label(RichText::new("♥ Любимые треки").size(18.0).strong());
            ui.add_space(16.0);

            // Sync from YouTube button
            if !self.settings.google_cookie.is_empty() {
                let user_name = self.settings.account_name.as_deref().unwrap_or("аккаунтом");
                let sync_label = format!("🔄 Синхронизировать с {}", user_name);
                if ui.button(RichText::new(sync_label).strong()).clicked() {
                    self.status_message = Some("Загрузка треков из аккаунта...".to_string());
                    let client = Arc::clone(&self.innertube);
                    let cookie = self.settings.google_cookie.clone();
                    let auth_user = self.settings.auth_user_index;
                    let tx = self.event_tx.clone();

                    tokio::spawn(async move {
                        let res = client.fetch_liked_music(&cookie, auth_user).await;
                        let _ = tx.send(AppEvent::LikedMusicLoaded(res));
                    });
                }
            }

            ui.add_space(8.0);
            if !self.library.liked_tracks.is_empty() {
                let shuffle_btn = egui::Button::new(RichText::new("🔀 Перемешать всё").strong())
                    .fill(Color32::from_rgb(40, 36, 52))
                    .rounding(Rounding::same(6.0));
                if ui
                    .add(shuffle_btn)
                    .on_hover_text("Перемешать любимые треки и начать воспроизведение")
                    .clicked()
                {
                    let mut tracks = self.library.liked_tracks.clone();
                    Self::shuffle_slice(&mut tracks);
                    self.queue = tracks.clone();
                    if let Some(first) = tracks.first().cloned() {
                        self.play_track(first);
                    }
                }

                ui.add_space(8.0);

                let dl_all_btn = egui::Button::new(RichText::new("⬇ Скачать все").strong())
                    .fill(Color32::from_rgb(32, 45, 58))
                    .rounding(Rounding::same(6.0));
                if ui
                    .add(dl_all_btn)
                    .on_hover_text("Скачать все любимые треки в папку Музыка")
                    .clicked()
                {
                    self.download_playlist(self.library.liked_tracks.clone());
                }
            }
        });

        ui.add_space(12.0);

        if self.library.liked_tracks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    RichText::new("У вас пока нет любимых треков.")
                        .size(15.0)
                        .weak(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Нажмите «Синхронизировать с YouTube» или ставьте ♥ в поиске.")
                        .size(12.0)
                        .weak(),
                );
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut track_idx_to_play = None;
                let mut track_to_download = None;
                let mut track_to_toggle_like = None;
                let rounding = Rounding::same(self.settings.album_art_rounding);

                for (idx, track) in self.library.liked_tracks.iter().enumerate() {
                    let frame = egui::Frame::none()
                        .fill(Color32::from_rgb(26, 26, 34))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(8.0)
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)));

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(thumb) = &track.thumbnail_url {
                                ui.add(
                                    egui::Image::new(thumb)
                                        .fit_to_exact_size(Vec2::new(40.0, 40.0))
                                        .rounding(rounding),
                                );
                            }
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&track.title)
                                        .size(13.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(RichText::new(&track.artist).size(11.0).weak());
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let play_btn =
                                        egui::Button::new(RichText::new("▶ Играть").strong())
                                            .fill(accent)
                                            .rounding(Rounding::same(6.0));
                                    if ui.add(play_btn).clicked() {
                                        track_idx_to_play = Some(idx);
                                    }

                                    if ui
                                        .button(
                                            RichText::new("♥")
                                                .size(15.0)
                                                .color(Color32::from_rgb(244, 63, 94)),
                                        )
                                        .clicked()
                                    {
                                        track_to_toggle_like = Some(track.clone());
                                    }

                                    if ui.button("⬇").on_hover_text("Скачать MP3").clicked()
                                    {
                                        track_to_download = Some(track.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }

                if let Some(t) = track_to_toggle_like {
                    self.library.toggle_like(t);
                }
                if let Some(idx) = track_idx_to_play {
                    self.queue = self.library.liked_tracks.clone();
                    self.current_index = Some(idx);
                    let track = self.library.liked_tracks[idx].clone();
                    self.play_track(track);
                }
                if let Some(t) = track_to_download {
                    self.download_track(t);
                }
            });
    }

    fn render_playlists_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        // 1. If viewing a specific playlist
        if let Some(pl) = self.selected_playlist.clone() {
            ui.horizontal(|ui| {
                if ui.button("← Ко всем плейлистам").clicked() {
                    self.selected_playlist = None;
                    self.playlist_tracks.clear();
                    return;
                }
                ui.add_space(10.0);
                ui.label(RichText::new(&pl.title).size(18.0).strong());
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if let Some(thumb) = &pl.thumbnail_url {
                    ui.add(
                        egui::Image::new(thumb)
                            .fit_to_exact_size(Vec2::new(64.0, 64.0))
                            .rounding(Rounding::same(self.settings.album_art_rounding)),
                    );
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&pl.subtitle).size(12.0).weak());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if !self.playlist_tracks.is_empty() {
                            let play_all_btn = egui::Button::new(
                                RichText::new("▶ Слушать всё")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(accent)
                            .rounding(Rounding::same(6.0));
                            if ui.add(play_all_btn).clicked() {
                                self.queue = self.playlist_tracks.clone();
                                if let Some(first) = self.playlist_tracks.first().cloned() {
                                    self.play_track(first);
                                }
                            }

                            let shuffle_pl_btn =
                                egui::Button::new(RichText::new("🔀 Перемешать плейлист").strong())
                                    .fill(Color32::from_rgb(40, 36, 52))
                                    .rounding(Rounding::same(6.0));
                            if ui
                                .add(shuffle_pl_btn)
                                .on_hover_text(
                                    "Перемешать треки плейлиста и начать воспроизведение",
                                )
                                .clicked()
                            {
                                let mut tracks = self.playlist_tracks.clone();
                                Self::shuffle_slice(&mut tracks);
                                self.queue = tracks.clone();
                                if let Some(first) = tracks.first().cloned() {
                                    self.play_track(first);
                                }
                            }

                            if ui.button("+ Вся в очередь").clicked() {
                                self.queue.extend(self.playlist_tracks.clone());
                                self.status_message =
                                    Some("Плейлист добавлен в очередь".to_string());
                            }

                            // Download whole playlist button
                            let dl_all_btn = egui::Button::new(
                                RichText::new("⬇ Скачать весь плейлист").strong(),
                            )
                            .fill(Color32::from_rgb(32, 45, 58))
                            .rounding(Rounding::same(6.0));
                            if ui
                                .add(dl_all_btn)
                                .on_hover_text(
                                    "Скачать все треки из этого плейлиста в папку Музыка",
                                )
                                .clicked()
                            {
                                self.download_playlist(self.playlist_tracks.clone());
                            }
                        }
                    });
                });
            });

            ui.add_space(14.0);

            if self.is_loading_playlist_tracks {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.spinner();
                    ui.add_space(8.0);
                    ui.label(RichText::new("Загрузка треков плейлиста...").weak());
                });
                return;
            }

            if self.playlist_tracks.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("В этом плейлисте пока нет треков").weak());
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut track_idx_to_play = None;
                let mut track_to_queue = None;
                let mut track_to_download = None;
                let rounding = Rounding::same(self.settings.album_art_rounding);

                for (idx, track) in self.playlist_tracks.iter().enumerate() {
                    let frame = egui::Frame::none()
                        .fill(Color32::from_rgb(26, 26, 34))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(8.0);

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}.", idx + 1)).weak());
                            ui.add_space(6.0);

                            if let Some(thumb) = &track.thumbnail_url {
                                ui.add(
                                    egui::Image::new(thumb)
                                        .fit_to_exact_size(Vec2::new(36.0, 36.0))
                                        .rounding(rounding),
                                );
                            }
                            ui.add_space(8.0);

                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&track.title)
                                        .size(13.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(RichText::new(&track.artist).size(11.0).weak());
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button(RichText::new("▶").strong()).clicked() {
                                        track_idx_to_play = Some(idx);
                                    }
                                    if ui.button("+").on_hover_text("Добавить в очередь").clicked()
                                    {
                                        track_to_queue = Some(track.clone());
                                    }
                                    if ui.button("⬇").on_hover_text("Скачать MP3").clicked()
                                    {
                                        track_to_download = Some(track.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }

                if let Some(idx) = track_idx_to_play {
                    self.queue = self.playlist_tracks.clone();
                    self.current_index = Some(idx);
                    let track = self.playlist_tracks[idx].clone();
                    self.play_track(track);
                }
                if let Some(t) = track_to_queue {
                    self.queue.push(t);
                    self.status_message = Some("Трек добавлен в очередь".to_string());
                }
                if let Some(t) = track_to_download {
                    self.download_track(t);
                }
            });

            return;
        }

        // 2. Auto-created playlists (photo_1)
        let any_auto = self.settings.show_liked_playlist
            || self.settings.show_downloaded_playlist
            || self.settings.show_top_playlist
            || self.settings.show_cached_playlist
            || self.settings.show_uploaded_playlist;
        if any_auto {
            ui.label(RichText::new("Автосозданные плейлисты").size(14.0).weak());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if self.settings.show_liked_playlist {
                    let btn = egui::Button::new(RichText::new("♥  Понравившиеся").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    if ui.add_sized([160.0, 36.0], btn).clicked() {
                        self.active_tab = ActiveTab::Liked;
                    }
                }
                if self.settings.show_downloaded_playlist {
                    let btn = egui::Button::new(RichText::new("⬇  Скачанные").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    if ui.add_sized([160.0, 36.0], btn).clicked() {
                        self.active_tab = ActiveTab::Downloads;
                    }
                }
                if self.settings.show_top_playlist {
                    let btn = egui::Button::new(RichText::new("🔥  Топ треки").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    if ui.add_sized([160.0, 36.0], btn).clicked()
                        && !self.library.liked_tracks.is_empty()
                    {
                        self.queue = self.library.liked_tracks.clone();
                        if let Some(first) = self.library.liked_tracks.first().cloned() {
                            self.play_track(first);
                        }
                    }
                }
                if self.settings.show_cached_playlist {
                    let btn = egui::Button::new(RichText::new("⚡  Кэшированные").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    let _ = ui.add_sized([160.0, 36.0], btn);
                }
                if self.settings.show_uploaded_playlist {
                    let btn = egui::Button::new(RichText::new("☁  Загруженные").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    let _ = ui.add_sized([160.0, 36.0], btn);
                }
            });
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(10.0);
        }

        // 3. User playlists from YouTube Music
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("📑 Плейлисты YouTube Music")
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(16.0);
            if ui
                .button(RichText::new("🔄 Синхронизировать плейлисты").strong())
                .clicked()
            {
                self.sync_playlists();
            }
        });

        ui.add_space(14.0);

        if self.is_loading_playlists {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label(RichText::new("Загрузка ваших плейлистов из YouTube Music...").weak());
            });
            return;
        }

        if self.playlists.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("Плейлисты ещё не загружены.").size(15.0).weak());
                ui.add_space(6.0);
                ui.label(RichText::new("Нажмите «Синхронизировать плейлисты», чтобы загрузить плейлисты из вашего аккаунта.").size(12.0).weak());
            });
            return;
        }

        let mut playlist_to_open = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let rounding = Rounding::same(self.settings.album_art_rounding);

            for pl in &self.playlists {
                let frame = egui::Frame::none()
                    .fill(Color32::from_rgb(26, 26, 34))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(10.0)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)));

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(thumb) = &pl.thumbnail_url {
                            ui.add(
                                egui::Image::new(thumb)
                                    .fit_to_exact_size(Vec2::new(56.0, 56.0))
                                    .rounding(rounding),
                            );
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(56.0, 56.0), egui::Sense::hover());
                            ui.painter()
                                .rect_filled(rect, rounding, Color32::from_rgb(45, 45, 55));
                        }
                        ui.add_space(10.0);

                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&pl.title)
                                    .size(15.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(2.0);
                            ui.label(RichText::new(&pl.subtitle).size(12.0).weak());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let open_btn = egui::Button::new(RichText::new("Открыть →").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(open_btn).clicked() {
                                playlist_to_open = Some(pl.clone());
                            }
                        });
                    });
                });
                ui.add_space(6.0);
            }
        });

        if let Some(pl) = playlist_to_open {
            self.open_playlist(pl);
        }
    }

    fn render_queue_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("☰ Очередь воспроизведения")
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(16.0);
            if ui.button("🔀 Перемешать очередь").clicked() {
                self.shuffle_queue();
                self.status_message = Some("Очередь перемешана".to_string());
            }
            ui.add_space(8.0);
            if ui.button("Очистить очередь").clicked() {
                self.queue.clear();
                self.current_index = None;
            }
        });

        ui.add_space(12.0);

        if self.queue.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("Очередь воспроизведения пуста").weak());
            });
            return;
        }

        let mut track_to_play = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (idx, track) in self.queue.iter().enumerate() {
                let is_current = Some(idx) == self.current_index;
                let bg = if is_current {
                    Color32::from_rgb(35, 45, 40)
                } else {
                    Color32::from_rgb(26, 26, 34)
                };

                let frame = egui::Frame::none()
                    .fill(bg)
                    .rounding(Rounding::same(6.0))
                    .inner_margin(8.0);

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let num_text = format!("{}.", idx + 1);
                        ui.label(RichText::new(num_text).weak());
                        ui.add_space(6.0);

                        ui.vertical(|ui| {
                            let title_color = if is_current { accent } else { Color32::WHITE };
                            ui.label(
                                RichText::new(&track.title)
                                    .size(13.0)
                                    .strong()
                                    .color(title_color),
                            );
                            ui.label(RichText::new(&track.artist).size(11.0).weak());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !is_current && ui.button("▶").clicked() {
                                track_to_play = Some(track.clone());
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });

        if let Some(t) = track_to_play {
            self.play_track(t.clone());
        }
    }

    fn render_lyrics_tab(&mut self, ui: &mut egui::Ui, current_time: f64) {
        let accent = self.accent_color();

        ui.label(RichText::new("♫ Текст песни").size(18.0).strong());
        ui.add_space(12.0);

        let Some(lyrics) = &self.lyrics else {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    RichText::new("Текст песни не найден или трек ещё не воспроизводится").weak(),
                );
            });
            return;
        };

        if !lyrics.is_synced {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(
                    RichText::new(&lyrics.plain)
                        .size(15.0)
                        .line_height(Some(24.0)),
                );
            });
            return;
        }

        // Synced lyrics with auto-scroll and highlight
        let current_line_idx = lyrics.lines.iter().rposition(|l| l.seconds <= current_time);
        let mut seek_to_sec: Option<f64> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (idx, line) in lyrics.lines.iter().enumerate() {
                let is_active = Some(idx) == current_line_idx;
                if is_active {
                    let active_frame = egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(29, 185, 84, 45))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0));
                    let resp = active_frame
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("▶").size(15.0).color(accent).strong());
                                ui.label(
                                    RichText::new(&line.text)
                                        .size(19.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                            });
                        })
                        .response;

                    if resp.clicked() {
                        seek_to_sec = Some(line.seconds);
                    }
                    if self.settings.lyrics_auto_scroll {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                } else {
                    let is_past = current_line_idx.is_some_and(|cur| idx < cur);
                    let text_color = if is_past {
                        Color32::from_rgb(130, 130, 140)
                    } else {
                        Color32::from_rgb(215, 215, 225)
                    };
                    let resp = ui.add(
                        egui::Label::new(RichText::new(&line.text).size(15.0).color(text_color))
                            .sense(egui::Sense::click()),
                    );
                    if resp.clicked() {
                        seek_to_sec = Some(line.seconds);
                    }
                }
                ui.add_space(6.0);
            }
        });

        if let Some(sec) = seek_to_sec {
            self.player.seek(sec);
            self.last_auto_sync = std::time::Instant::now();
            let playback_info = self
                .player
                .info
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            self.trigger_auto_sync(&playback_info);
        }
    }

    fn render_downloads_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        ui.horizontal(|ui| {
            ui.label(RichText::new("⬇ Локальные загрузки").size(18.0).strong());
            ui.add_space(16.0);
            if ui.button("Открыть папку загрузок").clicked() {
                let dir = self.settings.download_dir.clone();
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            }
        });

        ui.add_space(12.0);

        if self.library.downloaded_tracks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("Вы ещё не скачивали треки.").weak());
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Папка для сохранения: {}",
                        self.settings.download_dir
                    ))
                    .size(11.0)
                    .weak(),
                );
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut track_to_play = None;
            let rounding = Rounding::same(self.settings.album_art_rounding);

            for track in &self.library.downloaded_tracks {
                let frame = egui::Frame::none()
                    .fill(Color32::from_rgb(26, 26, 34))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(8.0);

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(thumb) = &track.thumbnail_url {
                            ui.add(
                                egui::Image::new(thumb)
                                    .fit_to_exact_size(Vec2::new(40.0, 40.0))
                                    .rounding(rounding),
                            );
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&track.title)
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.label(RichText::new(&track.artist).size(11.0).weak());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let play_btn = egui::Button::new(RichText::new("▶ Играть").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_btn).clicked() {
                                track_to_play = Some(track.clone());
                            }
                            ui.label(
                                RichText::new("✓ MP3")
                                    .size(11.0)
                                    .color(Color32::from_rgb(16, 185, 129)),
                            );
                        });
                    });
                });
                ui.add_space(4.0);
            }

            if let Some(t) = track_to_play {
                self.play_track(t.clone());
            }
        });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let accent = self.accent_color();
        let strings = crate::i18n::tr(&self.settings.ui_language);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("⚙ {}", strings.nav_settings))
                    .size(22.0)
                    .strong(),
            );
            ui.add_space(12.0);
            ui.label(RichText::new("Cibitube v0.2.9").size(12.0).weak());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let is_en = self.settings.ui_language == "en";
                if ui.selectable_label(is_en, "🇬🇧 English").clicked() {
                    self.settings.ui_language = "en".to_string();
                    self.settings.save();
                }
                let is_ru = self.settings.ui_language == "ru";
                if ui.selectable_label(is_ru, "🇷🇺 Русский").clicked() {
                    self.settings.ui_language = "ru".to_string();
                    self.settings.save();
                }
                ui.label(
                    RichText::new(strings.settings_language_label)
                        .size(12.0)
                        .weak(),
                );
            });
        });
        ui.add_space(10.0);

        // 1. Быстрые фильтры меню (разбивка на удобные категории)
        let categories = [
            (
                0,
                if self.settings.ui_language == "en" {
                    "📂 All"
                } else {
                    "📂 Все меню"
                },
            ),
            (
                1,
                if self.settings.ui_language == "en" {
                    "👤 Accounts"
                } else {
                    "👤 Аккаунты"
                },
            ),
            (
                2,
                if self.settings.ui_language == "en" {
                    "📑 Library"
                } else {
                    "📑 Библиотека"
                },
            ),
            (
                3,
                if self.settings.ui_language == "en" {
                    "🎨 Theme"
                } else {
                    "🎨 Оформление"
                },
            ),
            (
                4,
                if self.settings.ui_language == "en" {
                    "▶ Player"
                } else {
                    "▶ Плеер"
                },
            ),
            (
                5,
                if self.settings.ui_language == "en" {
                    "♫ Lyrics"
                } else {
                    "♫ Текст песни"
                },
            ),
            (
                6,
                if self.settings.ui_language == "en" {
                    "🖥 Window"
                } else {
                    "🖥 Окно и Вид"
                },
            ),
            (
                7,
                if self.settings.ui_language == "en" {
                    "🔊 Audio"
                } else {
                    "🔊 Звук"
                },
            ),
            (8, "🎮 Discord"),
            (
                9,
                if self.settings.ui_language == "en" {
                    "💾 Downloads"
                } else {
                    "💾 Хранилище"
                },
            ),
            (
                10,
                if self.settings.ui_language == "en" {
                    "🔄 Updates"
                } else {
                    "🔄 Обновления"
                },
            ),
            (
                11,
                if self.settings.ui_language == "en" {
                    "☁ Sync"
                } else {
                    "☁ Синхронизация"
                },
            ),
            (
                12,
                if self.settings.ui_language == "en" {
                    "💖 Support"
                } else {
                    "💖 Поддержать"
                },
            ),
        ];

        ui.horizontal_wrapped(|ui| {
            for (cat_id, title) in categories {
                let is_sel = self.settings_menu_tab == cat_id;
                let btn = egui::Button::new(RichText::new(title).size(12.0).color(if is_sel {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(205, 205, 215)
                }))
                .fill(if is_sel {
                    accent
                } else {
                    Color32::from_rgb(28, 28, 36)
                })
                .rounding(Rounding::same(8.0));
                if ui.add_sized([106.0, 32.0], btn).clicked() {
                    self.settings_menu_tab = cat_id;
                }
            }
        });

        ui.add_space(14.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // 1. АККАУНТ GOOGLE / YOUTUBE MUSIC
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 1 {
                egui::CollapsingHeader::new(RichText::new("👤 Аккаунты Google / YouTube Music").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let is_logged_in = !self.settings.google_cookie.is_empty();

                        if is_logged_in {
                            let user_title = self.settings.account_name.as_deref().unwrap_or("Пользователь YouTube");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(16, 185, 129)));
                                ui.label(RichText::new(format!("Текущий активный аккаунт: {}", user_title)).size(13.0).strong().color(Color32::WHITE));
                            });
                            ui.label(RichText::new("Синхронизация библиотеки, плейлистов и любимых треков с выбранным профилем активна.").size(11.0).weak());
                            ui.add_space(10.0);

                            // ACCOUNT SWITCHER
                            if !self.settings.available_accounts.is_empty() {
                                ui.label(RichText::new("Выберите, в какой аккаунт войти:").size(12.0).strong());
                                ui.add_space(4.0);

                                let mut account_to_switch = None;

                                for acc in &self.settings.available_accounts {
                                    let is_selected = acc.auth_user == self.settings.auth_user_index;
                                    let radio_text = if acc.email.is_empty() {
                                        acc.name.clone()
                                    } else {
                                        format!("{} ({})", acc.name, acc.email)
                                    };

                                    let frame = egui::Frame::none()
                                        .fill(if is_selected { Color32::from_rgb(30, 45, 38) } else { Color32::from_rgb(24, 24, 30) })
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(8.0)
                                        .stroke(Stroke::new(1.0_f32, if is_selected { Color32::from_rgb(16, 185, 129) } else { Color32::from_rgb(40, 40, 50) }));

                                    frame.show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let symbol = if is_selected { "●" } else { "○" };
                                            let symbol_col = if is_selected { Color32::from_rgb(16, 185, 129) } else { Color32::GRAY };
                                            ui.label(RichText::new(symbol).size(14.0).color(symbol_col));
                                            ui.label(RichText::new(&radio_text).size(13.0).strong());

                                            if is_selected {
                                                ui.label(RichText::new("[Активен]").size(11.0).color(Color32::from_rgb(16, 185, 129)));
                                            } else {
                                                if ui.button("Выбрать этот").clicked() {
                                                    account_to_switch = Some(acc.clone());
                                                }
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);
                                }

                                if let Some(acc) = account_to_switch {
                                    self.settings.auth_user_index = acc.auth_user;
                                    self.settings.account_name = Some(acc.name.clone());
                                    self.settings.save();
                                    self.status_message = Some(format!("Переключено на аккаунт: {}", acc.name));
                                    self.sync_playlists();
                                }

                                ui.add_space(8.0);
                            }

                            ui.horizontal(|ui| {
                                if ui.button("🔄 Обновить список аккаунтов").clicked() {
                                    let client = Arc::clone(&self.innertube);
                                    let cookie = self.settings.google_cookie.clone();
                                    let tx = self.event_tx.clone();
                                    tokio::spawn(async move {
                                        let accounts = client.discover_accounts(&cookie).await;
                                        let _ = tx.send(AppEvent::AccountsDiscovered(accounts));
                                    });
                                }

                                if ui.button(RichText::new("🚪 Выйти из аккаунта").color(Color32::from_rgb(244, 63, 94))).clicked() {
                                    self.settings.google_cookie.clear();
                                    self.settings.account_name = None;
                                    self.settings.available_accounts.clear();
                                    self.settings.save();
                                    self.status_message = Some("Вы вышли из аккаунта".to_string());
                                }
                            });
                        } else {
                            ui.label(RichText::new("Вход позволяет слушать ваши плейлисты, синхронизировать лайки и рекомендации с YouTube Music.").size(12.0).weak());
                            ui.add_space(8.0);

                            let ff_btn = egui::Button::new(RichText::new("🦊 Войти через Firefox в один клик").size(13.0).strong().color(Color32::WHITE))
                                .fill(Color32::from_rgb(234, 88, 12))
                                .rounding(Rounding::same(8.0));

                            if ui.add_sized([260.0, 36.0], ff_btn).clicked() {
                                self.status_message = Some("Чтение профилей Firefox в фоне...".to_string());
                                let tx = self.event_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let res = BrowserAuth::extract_firefox_cookies();
                                    let _ = tx.send(AppEvent::CookiesExtracted(res));
                                });
                            }
                            ui.add_space(4.0);
                            ui.label(RichText::new("Автоматически обнаруживает все ваши аккаунты Google в Firefox и даёт выбрать нужный.").size(11.0).weak());

                            ui.add_space(8.0);
                            ui.collapsing("Ввести Cookie вручную (для Chromium / Chrome / Edge)", |ui| {
                                let mut save_clicked = false;
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut self.settings.google_cookie)
                                        .hint_text("Вставьте строку Cookie...")
                                        .desired_width(ui.available_width() - 90.0));
                                    if ui.button("Сохранить").clicked() {
                                        save_clicked = true;
                                    }
                                });
                                if save_clicked {
                                    self.settings.save();
                                    let client = Arc::clone(&self.innertube);
                                    let cookie = self.settings.google_cookie.clone();
                                    let tx = self.event_tx.clone();
                                    tokio::spawn(async move {
                                        let accounts = client.discover_accounts(&cookie).await;
                                        let _ = tx.send(AppEvent::AccountsDiscovered(accounts));
                                    });
                                }
                            });
                        }
                    });
                ui.add_space(12.0);
            }

            // 2. АВТОСОЗДАННЫЕ ПЛЕЙЛИСТЫ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 2 {
                egui::CollapsingHeader::new(RichText::new("📑 Автосозданные плейлисты (Библиотека)").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Отображать автосозданные плейлисты в библиотеке:").size(12.0).weak());
                        ui.add_space(6.0);

                        if ui.checkbox(&mut self.settings.show_liked_playlist, "Понравившиеся").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_downloaded_playlist, "Скачанные").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_top_playlist, "Топ").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_cached_playlist, "Кэшированные").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_uploaded_playlist, "Загруженные").changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 3. ВНЕШНИЙ ВИД И ТЕМА
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 3 {
                egui::CollapsingHeader::new(RichText::new("🎨 Внешний вид и Тема").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Режим темы:");
                            let themes = ["Чисто чёрный (AMOLED)", "Тёмный сланец", "Светлая"];
                            for (i, t_name) in themes.iter().enumerate() {
                                let is_sel = self.settings.theme_mode == i;
                                if ui.selectable_label(is_sel, *t_name).clicked() {
                                    self.settings.theme_mode = i;
                                    self.settings.save();
                                    self.apply_theme(ctx);
                                }
                            }
                        });

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Акцентный цвет:");
                            let color_names = ["Изумруд (Metrolist)", "Рубин", "Аметист", "Кибер-синий", "Янтарь"];
                            for (i, name) in color_names.iter().enumerate() {
                                let color = Self::accent_colors()[i];
                                let is_sel = !self.settings.use_custom_accent && self.settings.accent_color_idx == i;
                                let btn = egui::Button::new(RichText::new(*name).color(Color32::WHITE))
                                    .fill(if is_sel { color } else { Color32::from_rgb(35, 35, 45) });
                                if ui.add(btn).clicked() {
                                    self.settings.use_custom_accent = false;
                                    self.settings.accent_color_idx = i;
                                    self.settings.save();
                                    self.apply_theme(ctx);
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Кастомный цвет:");
                            let mut custom_c = Color32::from_rgb(
                                self.settings.custom_accent_color[0],
                                self.settings.custom_accent_color[1],
                                self.settings.custom_accent_color[2],
                            );
                            let is_custom = self.settings.use_custom_accent;
                            let custom_btn = egui::Button::new(RichText::new("🎨 Свой цвет").color(Color32::WHITE))
                                .fill(if is_custom { custom_c } else { Color32::from_rgb(35, 35, 45) });
                            if ui.add(custom_btn).on_hover_text("Активировать свой цвет").clicked() {
                                self.settings.use_custom_accent = true;
                                self.settings.save();
                                self.apply_theme(ctx);
                            }

                            if egui::color_picker::color_edit_button_srgba(ui, &mut custom_c, egui::color_picker::Alpha::Opaque).changed() {
                                self.settings.use_custom_accent = true;
                                self.settings.custom_accent_color = [custom_c.r(), custom_c.g(), custom_c.b()];
                                self.settings.save();
                                self.apply_theme(ctx);
                            }

                            let hex_str = format!("#{:02X}{:02X}{:02X}", custom_c.r(), custom_c.g(), custom_c.b());
                            ui.label(RichText::new(hex_str).monospace().weak());
                        });

                        ui.add_space(6.0);
                        if ui.checkbox(&mut self.settings.dynamic_icon_colors, "Динамические цвета значка под обложку").changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 4. МИНИ-ПЛЕЕР И ПЛЕЕР
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 4 {
                egui::CollapsingHeader::new(RichText::new("▶ Мини-плеер и Плеер").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.new_mini_player_design, "Новый дизайн мини-плеера").changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label("Стиль фона мини-плеера:");
                            let bg_styles = ["Градиент", "Чистый черный", "Сплошной"];
                            for (i, name) in bg_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.mini_player_bg_style == i, *name).clicked() {
                                    self.settings.mini_player_bg_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.new_player_design, "Новый дизайн плеера").changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label("Стиль фона плеера:");
                            let bg_styles = ["Градиент", "Чистый черный", "Сплошной"];
                            for (i, name) in bg_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.player_bg_style == i, *name).clicked() {
                                    self.settings.player_bg_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.hide_player_thumbnail, "Скрыть миниатюру плеера").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.crop_album_art, "Обрезать обложку альбома (1:1)").changed() {
                            self.settings.save();
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Цветовой стиль кнопок плеера:");
                            let btn_styles = ["Акцентный", "Монохромный", "Адаптивный"];
                            for (i, name) in btn_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.player_buttons_color_style == i, *name).clicked() {
                                    self.settings.player_buttons_color_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("Стиль ползунка плеера:");
                            let slider_styles = ["Прямой", "Волнистый (Змейка)", "Тонкий"];
                            for (i, name) in slider_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.slider_style == i, *name).clicked() {
                                    self.settings.slider_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.experimental_lyrics, "Экспериментальный текст").changed() {
                            self.settings.save();
                        }

                        if ui.checkbox(&mut self.settings.show_remaining_time, "Отображать оставшееся время трека (-01:23)").changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label("Скругление обложек:");
                            let roundings = [(4.0, "4 px"), (8.0, "8 px (Metrolist)"), (16.0, "16 px"), (24.0, "Круглые")];
                            for (r, label) in roundings {
                                if ui.selectable_label((self.settings.album_art_rounding - r).abs() < 0.5, label).clicked() {
                                    self.settings.album_art_rounding = r;
                                    self.settings.save();
                                }
                            }
                        });
                    });
                ui.add_space(12.0);
            }

            // 5. ТЕКСТ ПЕСНИ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 5 {
                egui::CollapsingHeader::new(RichText::new("♫ Текст песни (Lyrics)").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Расположение текста песни:");
                            let pos_names = ["По центру", "По левому краю"];
                            for (i, name) in pos_names.iter().enumerate() {
                                if ui.selectable_label(self.settings.lyrics_position == i, *name).clicked() {
                                    self.settings.lyrics_position = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.lyrics_respect_artist_position, "Учитывать позиционирование исполнителя").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.lyrics_change_on_tap, "Перемотка трека по клику на строку текста").changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.lyrics_auto_scroll, "Автоматическая плавная прокрутка текста").changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 6. ОКНО И НАВИГАЦИЯ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 6 {
                egui::CollapsingHeader::new(RichText::new("🖥 Окно и Навигация").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Вкладка навигации по умолчанию:");
                            let nav_tabs = ["Главная", "Плейлисты", "Любимые треки", "Поиск"];
                            for (i, name) in nav_tabs.iter().enumerate() {
                                if ui.selectable_label(self.settings.default_nav_tab == i, *name).clicked() {
                                    self.settings.default_nav_tab = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.slim_nav_bar, "Компактная боковая панель навигации").changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label("Размер ячейки сетки библиотеки:");
                            let grid_sizes = ["Маленький", "Средний", "Большой"];
                            for (i, name) in grid_sizes.iter().enumerate() {
                                if ui.selectable_label(self.settings.grid_cell_size == i, *name).clicked() {
                                    self.settings.grid_cell_size = i;
                                    self.settings.save();
                                }
                            }
                        });
                    });
                ui.add_space(12.0);
            }

            // 7. ЗВУК И КАЧЕСТВО
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 7 {
                egui::CollapsingHeader::new(RichText::new("🔊 Звук и Воспроизведение").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Качество потока:");
                            egui::ComboBox::from_id_salt("quality_setting")
                                .selected_text(&self.settings.audio_quality)
                                .show_ui(ui, |ui| {
                                    for q in ["High (320 kbps Opus)", "Standard (192 kbps)", "Low (128 kbps)"] {
                                        if ui.selectable_value(&mut self.settings.audio_quality, q.to_string(), q).clicked() {
                                            self.settings.save();
                                        }
                                    }
                                });
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.audio_normalization, "Нормализация громкости (ReplayGain / Loudness Normalization)").changed() {
                            self.settings.save();
                        }

                        if ui.checkbox(&mut self.settings.skip_silence, "Пропуск тишины в начале и конце треков").changed() {
                            self.settings.save();
                        }

                        if ui.checkbox(&mut self.settings.sponsorblock_enabled, "SponsorBlock (автоматический пропуск немузыкальных фрагментов)").changed() {
                            self.settings.save();
                        }

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(strings.player_speed).strong());
                            for spd in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                                let is_sel = (self.settings.playback_speed - spd).abs() < 0.05;
                                if ui.selectable_label(is_sel, format!("{spd:.2}x")).clicked() {
                                    self.settings.playback_speed = spd;
                                    self.player.set_speed(spd, self.settings.preserve_pitch);
                                    self.settings.save();
                                }
                            }
                        });
                        if ui.checkbox(&mut self.settings.preserve_pitch, "Сохранять тональность звука при изменении скорости").changed() {
                            self.player.set_speed(self.settings.playback_speed, self.settings.preserve_pitch);
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 8. DISCORD RPC
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 8 {
                egui::CollapsingHeader::new(RichText::new("🎮 Discord Rich Presence").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let mut enabled = self.settings.discord_rpc_enabled;
                        if ui.checkbox(&mut enabled, "Отображать статус в Discord").changed() {
                            self.settings.discord_rpc_enabled = enabled;
                            self.settings.save();
                            if !enabled {
                                self.discord.clear();
                            } else if let Some(t) = self.current_track() {
                                self.discord.set_activity(ActivityDetails {
                                    title: t.title.clone(),
                                    artist: t.artist.clone(),
                                    song_id: t.id.clone(),
                                    elapsed_secs: 0.0,
                                    duration_secs: 0.0,
                                    thumbnail: t.thumbnail_url.clone(),
                                    show_buttons: self.settings.discord_show_buttons,
                                    activity_type: self.settings.discord_activity_type,
                                });
                            }
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Тип активности:");
                            let types = ["Слушает (Listening)", "Играет (Playing)"];
                            for (idx, name) in types.iter().enumerate() {
                                let is_sel = self.settings.discord_activity_type == idx;
                                if ui.selectable_label(is_sel, *name).clicked() {
                                    self.settings.discord_activity_type = idx;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.discord_show_buttons, "Отображать кнопку «Слушать в Cibitube» в профиле Discord").changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 9. ХРАНИЛИЩЕ И ЗАГРУЗКИ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 9 {
                egui::CollapsingHeader::new(RichText::new("💾 Загрузки и Хранилище").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Папка для сохранения треков:");
                            ui.add(egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width(ui.available_width() - 140.0));
                            if ui.button("Сохранить").clicked() {
                                self.settings.save();
                                self.status_message = Some("Папка загрузки обновлена".to_string());
                            }
                        });

                        ui.add_space(6.0);
                        let dir = self.settings.download_dir.clone();
                        if ui.button("📂 Открыть папку загрузок в файловом менеджере").clicked() {
                            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                        }

                        ui.add_space(8.0);
                        ui.label(RichText::new("Опции умного загрузчика:").strong());
                        if ui.checkbox(&mut self.settings.download_embed_cover, strings.downloads_embed_cover).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.download_save_cover_file, strings.downloads_save_cover_jpg).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.download_save_lyrics_lrc, strings.downloads_save_lyrics_lrc).changed() {
                            self.settings.save();
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label("Формат аудио:");
                            for fmt in ["mp3", "opus", "m4a", "flac"] {
                                let is_sel = self.settings.download_audio_format == fmt;
                                if ui.selectable_label(is_sel, fmt.to_uppercase()).clicked() {
                                    self.settings.download_audio_format = fmt.to_string();
                                    self.settings.save();
                                }
                            }
                        });
                    });
                ui.add_space(12.0);
            }

            // 10. ОБНОВЛЕНИЯ ПРОГРАММЫ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 10 {
                egui::CollapsingHeader::new(RichText::new("🔄 Обновления программы (GitHub)").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let cur_ver = crate::updater::Updater::CURRENT_VERSION;
                        ui.label(RichText::new(format!("Текущая версия Cibitube: v{}", cur_ver)).size(13.0).strong());
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let check_btn = egui::Button::new(RichText::new(format!("🔍 {}", strings.updater_check_btn)).strong())
                                .fill(Color32::from_rgb(35, 45, 55))
                                .rounding(Rounding::same(6.0));
                            if ui.add(check_btn).clicked() {
                                self.is_checking_update = true;
                                self.update_status = Some("Проверка новых релизов на GitHub...".to_string());
                                let tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    let res = crate::updater::Updater::check_for_update().await;
                                    let _ = tx.send(AppEvent::UpdateCheckResult(res));
                                });
                            }

                            if ui.button(RichText::new(strings.updater_open_releases).weak()).clicked() {
                                crate::updater::Updater::open_releases_page();
                            }

                            if self.is_checking_update {
                                ui.spinner();
                            }
                        });

                        ui.add_space(6.0);

                        if let Some(info) = self.update_info.clone() {
                            let frame = egui::Frame::none()
                                .fill(Color32::from_rgb(20, 35, 28))
                                .rounding(Rounding::same(8.0))
                                .inner_margin(12.0)
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 80, 50)));

                            frame.show(ui, |ui| {
                                ui.label(RichText::new(format!("★ Доступна новая версия: {}!", info.version)).size(15.0).strong().color(Color32::from_rgb(52, 211, 153)));
                                if !info.notes.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(&info.notes).size(12.0).weak());
                                }
                                ui.add_space(8.0);

                                if self.is_applying_update {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new("Скачивание и замена файла программы...").color(Color32::from_rgb(56, 189, 248)));
                                    });
                                } else if self.show_update_confirm {
                                    ui.label(RichText::new("Вы уверены, что хотите обновить приложение? Текущая версия будет автоматически заменена на новую.").size(12.0).color(Color32::YELLOW));
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        let confirm_btn = egui::Button::new(RichText::new("✓ Да, установить обновление").strong().color(Color32::WHITE))
                                            .fill(Color32::from_rgb(16, 185, 129))
                                            .rounding(Rounding::same(6.0));
                                        if ui.add(confirm_btn).clicked() {
                                            self.is_applying_update = true;
                                            self.show_update_confirm = false;
                                            self.update_status = Some("Загрузка обновления...".to_string());
                                            let dl = info.download_url.clone();
                                            let tx = self.event_tx.clone();
                                            tokio::spawn(async move {
                                                let res = crate::updater::Updater::apply_update(dl).await;
                                                let _ = tx.send(AppEvent::UpdateApplied(res));
                                            });
                                        }

                                        if ui.button("Отмена").clicked() {
                                            self.show_update_confirm = false;
                                        }
                                    });
                                } else {
                                    let update_btn = egui::Button::new(RichText::new(format!("⬇ Скачать и обновить до {}", info.version)).strong().color(Color32::WHITE))
                                        .fill(Color32::from_rgb(16, 185, 129))
                                        .rounding(Rounding::same(6.0));
                                    if ui.add(update_btn).clicked() {
                                        self.show_update_confirm = true;
                                    }
                                }
                            });
                        }

                        if let Some(status) = &self.update_status {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(status).size(12.0).color(Color32::from_rgb(147, 197, 253)));
                                if status.contains("✓") {
                                    let restart_btn = egui::Button::new(RichText::new("🚀 Перезапустить Cibitube").strong().color(Color32::WHITE))
                                        .fill(accent)
                                        .rounding(Rounding::same(6.0));
                                    if ui.add(restart_btn).clicked() {
                                        crate::updater::Updater::restart();
                                    }
                                }
                            });
                        }
                    });
                ui.add_space(12.0);
            }

            // 11. СИНХРОНИЗАЦИЯ ЧЕРЕЗ GOOGLE ДИСК
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 11 {
                egui::CollapsingHeader::new(RichText::new(format!("☁ {}", strings.sync_title)).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let folder_path = std::path::Path::new(&self.settings.google_drive_sync_folder);
                        let folder_exists = folder_path.exists();
                        let sync_file_exists = folder_path.join(crate::sync::SyncManager::SYNC_FILE_NAME).exists();

                        ui.horizontal(|ui| {
                            if folder_exists {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(16, 185, 129)));
                                ui.label(RichText::new("Папка синхронизации доступна").size(13.0).strong().color(Color32::WHITE));
                            } else {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(239, 68, 68)));
                                ui.label(RichText::new("Папка синхронизации ещё не создана на диске").size(13.0).weak());
                            }
                        });
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label("Папка синхронизации:");
                            ui.add(egui::TextEdit::singleline(&mut self.settings.google_drive_sync_folder).desired_width(ui.available_width() - 140.0));
                            if ui.button("Сохранить").clicked() {
                                self.settings.save();
                                self.sync_status_message = Some("Папка синхронизации сохранена".to_string());
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let push_btn = egui::Button::new(RichText::new(strings.sync_push_btn).strong().color(Color32::WHITE))
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(push_btn).clicked() {
                                if let Some(track) = self.current_track().cloned() {
                                    let info = self.player.info.lock().map(|i| (i.position_secs, i.duration_secs, i.is_playing)).unwrap_or((0.0, 0.0, false));
                                    let res = crate::sync::SyncManager::save_state_with_queue(
                                        &self.settings.google_drive_sync_folder,
                                        &track,
                                        info.0,
                                        info.1,
                                        info.2,
                                        self.settings.playback_speed,
                                        &self.queue,
                                        self.current_index.unwrap_or(0),
                                        if self.repeat_mode == 1 { "one" } else if self.repeat_mode == 2 { "all" } else { "off" },
                                        self.shuffle_state,
                                    );
                                    match res {
                                        Ok(_) => self.sync_status_message = Some("✓ Состояние и очередь успешно синхронизированы в файл!".to_string()),
                                        Err(e) => self.sync_status_message = Some(format!("Ошибка синхронизации: {e}")),
                                    }
                                } else {
                                    self.sync_status_message = Some("Сейчас ничего не играет для выгрузки".to_string());
                                }
                            }

                            let pull_btn = egui::Button::new(RichText::new(strings.sync_pull_btn).strong())
                                .fill(Color32::from_rgb(35, 45, 55))
                                .rounding(Rounding::same(6.0));
                            if ui.add(pull_btn).clicked() {
                                if let Some(state) = crate::sync::SyncManager::load_state(&self.settings.google_drive_sync_folder) {
                                    let current_t = state.to_track();
                                    let eff_pos = state.effective_position_secs();
                                    if !state.queue.is_empty() {
                                        self.queue = state.queue.iter().map(|e| e.to_track()).collect();
                                        self.current_index = Some(state.queue_index.min(self.queue.len().saturating_sub(1)));
                                    }
                                    self.play_track(current_t);
                                    self.player.seek(eff_pos);
                                    self.sync_status_message = Some(format!("✓ Загружен трек: {} ({:.0} сек)", state.title, eff_pos));
                                } else {
                                    self.sync_status_message = Some(format!("Файл {} не найден в указанной папке", crate::sync::SyncManager::SYNC_FILE_NAME));
                                }
                            }
                        });

                        if let Some(msg) = &self.sync_status_message {
                            ui.add_space(6.0);
                            ui.label(RichText::new(msg).size(12.0).color(Color32::from_rgb(147, 197, 253)));
                        }

                        ui.add_space(10.0);

                        // Инструкция по настройке синхронизации (CollapsingHeader)
                        egui::CollapsingHeader::new(RichText::new(strings.sync_guide_title).size(14.0).strong())
                            .default_open(!sync_file_exists)
                            .show(ui, |ui| {
                                ui.add_space(4.0);
                                ui.label(RichText::new(strings.sync_guide_body).size(12.0).weak());
                                ui.add_space(6.0);
                                if ui.button(RichText::new(strings.sync_guide_docs_btn).strong()).clicked() {
                                    let _ = std::process::Command::new("xdg-open")
                                        .arg("https://github.com/Cibitka/Cibitube/blob/main/docs/GOOGLE_DRIVE_SYNC_SETUP.md")
                                        .spawn();
                                }
                            });
                    });
                ui.add_space(12.0);
            }

            // 12. ПОДДЕРЖАТЬ РАЗРАБОТЧИКА (ДОНАТЫ)
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 12 {
                egui::CollapsingHeader::new(RichText::new(format!("💖 {}", strings.donate_title)).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(strings.donate_subtitle).size(12.0).weak());
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let boosty_btn = egui::Button::new(RichText::new(strings.donate_boosty_btn).strong().color(Color32::WHITE))
                                .fill(Color32::from_rgb(241, 95, 44))
                                .rounding(Rounding::same(6.0));
                            if ui.add(boosty_btn).clicked() {
                                crate::donate::open_boosty_donate();
                            }

                            let ton_btn = egui::Button::new(RichText::new(strings.donate_copy_ton_btn).strong().color(Color32::WHITE))
                                .fill(Color32::from_rgb(0, 152, 234))
                                .rounding(Rounding::same(6.0));
                            if ui.add(ton_btn).clicked() {
                                let conf = crate::donate::DonateConfig::default();
                                ui.output_mut(|o| o.copied_text = conf.ton_usdt_wallet.to_string());
                                self.status_message = Some("TON/USDT адрес скопирован в буфер обмена!".to_string());
                            }
                        });
                    });
                ui.add_space(12.0);
            }
        });
    }
    fn render_now_playing_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();
        let playback_info = self
            .player
            .info
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let current_track = self.current_track().cloned();

        let avail_w = ui.available_width();
        let avail_h = ui.available_height();

        // 1. Top Bar: ← Назад, "СЕЙЧАС ИГРАЕТ" / Source name, Share, Download
        ui.horizontal(|ui| {
            let back_btn = egui::Button::new(
                RichText::new("← Назад")
                    .size(14.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(Color32::from_rgb(32, 32, 40))
            .rounding(Rounding::same(8.0));
            if ui.add_sized([90.0, 34.0], back_btn).clicked() {
                self.active_tab = self.previous_tab;
                return;
            }

            let center_w = (ui.available_width() - 100.0).max(100.0);
            ui.allocate_ui_with_layout(
                Vec2::new(center_w, 36.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(RichText::new("СЕЙЧАС ИГРАЕТ").size(10.0).weak());
                    let source_title = self
                        .selected_playlist
                        .as_ref()
                        .map(|p| p.title.as_str())
                        .unwrap_or("Очередь воспроизведения");
                    ui.label(
                        RichText::new(source_title)
                            .size(13.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                },
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(track) = &current_track {
                    let track_clone = track.clone();
                    if ui
                        .button(RichText::new("⬇").size(16.0))
                        .on_hover_text("Скачать трек")
                        .clicked()
                    {
                        self.download_track(track_clone);
                    }
                }
            });
        });

        ui.add_space(8.0);

        // 2. Responsive dimensions: dynamically scaled with window size
        // Calculate dynamic album art size (up to 440px)
        let bottom_controls_h = 275.0;
        let top_bar_h = 48.0;
        let space_for_art = (avail_h - top_bar_h - bottom_controls_h).max(180.0);
        let max_w_allowed = (avail_w - 40.0).max(180.0);
        let art_size = space_for_art.min(max_w_allowed).clamp(220.0, 440.0);
        let flexible_gap = (avail_h - (top_bar_h + art_size + bottom_controls_h)).clamp(6.0, 36.0);

        // Center the entire player column
        let center_offset = ((avail_w - art_size) / 2.0).max(0.0);

        ui.horizontal(|ui| {
            ui.add_space(center_offset);
            ui.allocate_ui_with_layout(
                Vec2::new(art_size, ui.available_height()),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    let art_rounding = Rounding::same(20.0);

                    // Top: Centered Large Album Art OR Synced Lyrics in-place
                    if self.show_lyrics_in_player {
                        let frame = egui::Frame::none()
                            .fill(Color32::from_rgb(20, 20, 26))
                            .rounding(art_rounding)
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 45, 55)))
                            .inner_margin(14.0);
                        frame.show(ui, |ui| {
                            ui.set_width(art_size - 28.0);
                            ui.set_height(art_size - 28.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("♫ Текст песни")
                                        .size(13.0)
                                        .strong()
                                        .color(accent),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .button(RichText::new("✕ Обложка").size(11.0))
                                            .clicked()
                                        {
                                            self.show_lyrics_in_player = false;
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);

                            if let Some(lyrics) = &self.lyrics {
                                if !lyrics.is_synced {
                                    egui::ScrollArea::vertical()
                                        .max_height(art_size - 80.0)
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(&lyrics.plain)
                                                    .size(14.0)
                                                    .line_height(Some(22.0)),
                                            );
                                        });
                                } else {
                                    let current_time = playback_info.position_secs;
                                    let current_line_idx = lyrics
                                        .lines
                                        .iter()
                                        .rposition(|l| l.seconds <= current_time);
                                    let mut seek_to_sec: Option<f64> = None;

                                    egui::ScrollArea::vertical()
                                        .max_height(art_size - 80.0)
                                        .show(ui, |ui| {
                                            for (idx, line) in lyrics.lines.iter().enumerate() {
                                                let is_active = Some(idx) == current_line_idx;
                                                if is_active {
                                                    let active_frame = egui::Frame::none()
                                                        .fill(Color32::from_rgba_unmultiplied(
                                                            29, 185, 84, 45,
                                                        ))
                                                        .rounding(Rounding::same(6.0))
                                                        .inner_margin(egui::Margin::symmetric(
                                                            10.0, 6.0,
                                                        ));
                                                    let resp = active_frame
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                ui.label(
                                                                    RichText::new("▶")
                                                                        .size(13.0)
                                                                        .color(accent)
                                                                        .strong(),
                                                                );
                                                                ui.label(
                                                                    RichText::new(&line.text)
                                                                        .size(16.0)
                                                                        .strong()
                                                                        .color(Color32::WHITE),
                                                                );
                                                            });
                                                        })
                                                        .response;

                                                    if resp.clicked() {
                                                        seek_to_sec = Some(line.seconds);
                                                    }
                                                    if self.settings.lyrics_auto_scroll {
                                                        resp.scroll_to_me(Some(
                                                            egui::Align::Center,
                                                        ));
                                                    }
                                                } else {
                                                    let is_past = current_line_idx
                                                        .is_some_and(|cur| idx < cur);
                                                    let text_color = if is_past {
                                                        Color32::from_rgb(120, 120, 130)
                                                    } else {
                                                        Color32::from_rgb(205, 205, 215)
                                                    };
                                                    let resp = ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&line.text)
                                                                .size(13.0)
                                                                .color(text_color),
                                                        )
                                                        .sense(egui::Sense::click()),
                                                    );
                                                    if resp.clicked() {
                                                        seek_to_sec = Some(line.seconds);
                                                    }
                                                }
                                                ui.add_space(4.0);
                                            }
                                        });

                                    if let Some(sec) = seek_to_sec {
                                        self.player.seek(sec);
                                        self.last_auto_sync = std::time::Instant::now();
                                        self.trigger_auto_sync(&playback_info);
                                    }
                                }
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space((art_size / 2.0) - 50.0);
                                    ui.label(
                                        RichText::new("Текст песни загружается или недоступен")
                                            .weak(),
                                    );
                                });
                            }
                        });
                    } else if !self.settings.hide_player_thumbnail {
                        if let Some(track) = &current_track {
                            if let Some(thumb) = &track.thumbnail_url {
                                let hd_thumb = InnertubeClient::upgrade_thumbnail_quality(thumb);
                                ui.add(
                                    egui::Image::new(&hd_thumb)
                                        .fit_to_exact_size(Vec2::new(art_size, art_size))
                                        .rounding(art_rounding),
                                );
                            } else {
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(art_size, art_size),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    art_rounding,
                                    Color32::from_rgb(32, 32, 40),
                                );
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "♫",
                                    egui::FontId::proportional(72.0),
                                    Color32::GRAY,
                                );
                            }
                        } else {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(art_size, art_size),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                rect,
                                art_rounding,
                                Color32::from_rgb(32, 32, 40),
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "♫",
                                egui::FontId::proportional(72.0),
                                Color32::GRAY,
                            );
                        }
                    }

                    // Flexible gap pushes all controls comfortably to the BOTTOM
                    ui.add_space(flexible_gap);

                    // --- BOTTOM INTERFACE SECTION ---
                    // Track metadata row: Title & Artist + Share & Heart (width = art_size)
                    ui.allocate_ui_with_layout(
                        Vec2::new(art_size, 52.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(art_size - 80.0, 48.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    if let Some(track) = &current_track {
                                        ui.label(
                                            RichText::new(&track.title)
                                                .size(20.0)
                                                .strong()
                                                .color(Color32::WHITE),
                                        );
                                        ui.label(RichText::new(&track.artist).size(14.0).weak());
                                    } else {
                                        ui.label(
                                            RichText::new("Ничего не играет").size(16.0).weak(),
                                        );
                                    }
                                },
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(track) = &current_track {
                                        let is_liked = self.library.is_liked(&track.id);
                                        let heart_color = if is_liked {
                                            Color32::from_rgb(244, 63, 94)
                                        } else {
                                            Color32::GRAY
                                        };
                                        let heart_symbol = if is_liked { "♥" } else { "♡" };
                                        let heart_btn = egui::Button::new(
                                            RichText::new(heart_symbol)
                                                .size(22.0)
                                                .color(heart_color),
                                        )
                                        .frame(false);
                                        if ui.add(heart_btn).clicked() {
                                            self.library.toggle_like(track.clone());
                                        }

                                        ui.add_space(8.0);

                                        let share_resp = Self::draw_share_icon(ui, accent);
                                        if share_resp
                                            .on_hover_text("Поделиться: копировать ссылку на трек")
                                            .clicked()
                                        {
                                            let link = format!(
                                                "https://music.youtube.com/watch?v={}",
                                                track.id
                                            );
                                            ui.output_mut(|o| o.copied_text = link);
                                            self.status_message =
                                                Some("Ссылка скопирована!".to_string());
                                        }
                                    }
                                },
                            );
                        },
                    );

                    ui.add_space(6.0);

                    // Progress Slider across the ENTIRE artwork width
                    self.render_playback_slider(ui, art_size, 18.0, &playback_info);

                    ui.add_space(3.0);

                    // Time labels flush with slider ends
                    ui.allocate_ui_with_layout(
                        Vec2::new(art_size, 16.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let curr_str = Self::format_time(playback_info.position_secs);
                            let dur_str = if self.settings.show_remaining_time
                                && playback_info.duration_secs > playback_info.position_secs
                            {
                                format!(
                                    "-{}",
                                    Self::format_time(
                                        playback_info.duration_secs - playback_info.position_secs
                                    )
                                )
                            } else {
                                Self::format_time(playback_info.duration_secs)
                            };
                            ui.label(RichText::new(curr_str).size(12.0).weak());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new(dur_str).size(12.0).weak());
                                },
                            );
                        },
                    );

                    ui.add_space(14.0);

                    // Controls row: Centered directly under cover & slider
                    let playback_w = 56.0 + 16.0 + 160.0 + 16.0 + 56.0; // 304.0
                    let playback_pad = ((art_size - playback_w) / 2.0).max(0.0);

                    ui.horizontal(|ui| {
                        ui.add_space(playback_pad);

                        let prev_btn = egui::Button::new(
                            RichText::new("⏮").size(22.0).strong().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(34, 34, 44))
                        .rounding(Rounding::same(27.0));
                        if ui.add_sized([56.0, 54.0], prev_btn).clicked() {
                            self.play_prev();
                        }

                        ui.add_space(16.0);

                        let (icon, text) = if playback_info.is_playing {
                            ("⏸", "Пауза")
                        } else {
                            ("▶", "Играть")
                        };
                        let play_btn = egui::Button::new(
                            RichText::new(format!("{}  {}", icon, text))
                                .size(17.0)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(accent)
                        .rounding(Rounding::same(27.0));
                        if ui.add_sized([160.0, 54.0], play_btn).clicked() {
                            self.player.toggle_pause();
                        }

                        ui.add_space(16.0);

                        let next_btn = egui::Button::new(
                            RichText::new("⏭").size(22.0).strong().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(34, 34, 44))
                        .rounding(Rounding::same(27.0));
                        if ui.add_sized([56.0, 54.0], next_btn).clicked() {
                            self.play_next();
                        }
                    });

                    ui.add_space(16.0);

                    // Bottom Action Icons Row (Queue, Shuffle, Lyrics, Repeat): Centered
                    let action_w = 44.0 * 4.0 + 14.0 * 3.0; // 218.0
                    let action_pad = ((art_size - action_w) / 2.0).max(0.0);

                    ui.horizontal(|ui| {
                        ui.add_space(action_pad);

                        let queue_btn =
                            egui::Button::new(RichText::new("☰").size(18.0).color(Color32::WHITE))
                                .frame(false);
                        if ui
                            .add_sized([44.0, 36.0], queue_btn)
                            .on_hover_text("Очередь треков")
                            .clicked()
                        {
                            self.active_tab = ActiveTab::Queue;
                        }

                        ui.add_space(14.0);

                        let shuffle_col = if self.shuffle_state {
                            accent
                        } else {
                            Color32::GRAY
                        };
                        let shuffle_btn =
                            egui::Button::new(RichText::new("🔀").size(18.0).color(shuffle_col))
                                .frame(false);
                        if ui
                            .add_sized([44.0, 36.0], shuffle_btn)
                            .on_hover_text("Перемешать очередь")
                            .clicked()
                        {
                            self.shuffle_state = !self.shuffle_state;
                            if self.shuffle_state {
                                self.shuffle_queue();
                                self.status_message = Some("Очередь перемешана".to_string());
                            }
                        }

                        ui.add_space(14.0);

                        let lyrics_col = if self.show_lyrics_in_player {
                            accent
                        } else {
                            Color32::WHITE
                        };
                        let lyrics_btn =
                            egui::Button::new(RichText::new("♫").size(18.0).color(lyrics_col))
                                .frame(false);
                        if ui
                            .add_sized([44.0, 36.0], lyrics_btn)
                            .on_hover_text(if self.show_lyrics_in_player {
                                "Скрыть текст песни"
                            } else {
                                "Показать текст песни вместо обложки"
                            })
                            .clicked()
                        {
                            self.show_lyrics_in_player = !self.show_lyrics_in_player;
                        }

                        ui.add_space(14.0);

                        let (rep_icon, rep_col) = match self.repeat_mode {
                            1 => ("🔁", accent),
                            2 => ("🔂", accent),
                            _ => ("🔁", Color32::GRAY),
                        };
                        let rep_btn =
                            egui::Button::new(RichText::new(rep_icon).size(18.0).color(rep_col))
                                .frame(false);
                        if ui
                            .add_sized([44.0, 36.0], rep_btn)
                            .on_hover_text("Режим повтора")
                            .clicked()
                        {
                            self.repeat_mode = (self.repeat_mode + 1) % 3;
                        }
                    });

                    ui.add_space(14.0);

                    // Volume slider row: Centered with mute toggle & percentage
                    let vol_row_w = 210.0;
                    let vol_pad = ((art_size - vol_row_w) / 2.0).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(vol_pad);
                        let vol_icon = if self.settings.volume == 0.0 {
                            "🔇"
                        } else if self.settings.volume < 50.0 {
                            "🔉"
                        } else {
                            "🔊"
                        };
                        let mute_btn =
                            egui::Button::new(RichText::new(vol_icon).size(15.0)).frame(false);
                        if ui
                            .add(mute_btn)
                            .on_hover_text("Без звука / Включить звук")
                            .clicked()
                        {
                            if self.settings.volume > 0.0 {
                                self.settings.volume = 0.0;
                            } else {
                                self.settings.volume = 80.0;
                            }
                            self.player.set_volume(self.settings.volume);
                            self.settings.save();
                        }
                        let mut vol = self.settings.volume;
                        let vol_slider = egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false);
                        if ui.add_sized([130.0, 14.0], vol_slider).changed() {
                            self.settings.volume = vol;
                            self.player.set_volume(vol);
                            self.settings.save();
                        }
                        ui.label(RichText::new(format!("{}%", vol as u32)).size(11.0).weak());
                    });
                },
            );
        });
    }
}
