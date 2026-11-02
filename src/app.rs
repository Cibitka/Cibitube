use crate::browser_auth::BrowserAuth;
use crate::discord::{ActivityDetails, DiscordRpc};
use crate::downloader::Downloader;
use crate::innertube::InnertubeClient;
use crate::library::Library;
use crate::lyrics::LyricsClient;
use crate::models::{AccountProfile, ActiveTab, Lyrics, Playlist, Track};
use crate::player::AudioPlayer;
use crate::settings::AppSettings;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

enum AppEvent {
    SearchResults(Result<Vec<Track>, String>),
    LikedMusicLoaded(Result<Vec<Track>, String>),
    PlaylistsLoaded(Result<Vec<Playlist>, String>),
    PlaylistTracksLoaded { playlist_id: String, result: Result<Vec<Track>, String> },
    LyricsLoaded(Option<Lyrics>),
    DownloadFinished { track: Track, result: Result<String, String> },
    AccountsDiscovered(Vec<AccountProfile>),
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
    search_query: String,
    search_results: Vec<Track>,
    is_searching: bool,
    status_message: Option<String>,
    download_status: Option<String>,

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
    _dl_rx: mpsc::Receiver<(Track, Result<String, String>)>,
}

impl MetrolistApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let (event_tx, event_rx) = mpsc::channel();
        let (downloader, dl_rx) = Downloader::new();
        let innertube = Arc::new(InnertubeClient::new());
        let lyrics_client = Arc::new(LyricsClient::new());
        let player = AudioPlayer::new().expect("Failed to initialize MPV player");
        let discord = Arc::new(DiscordRpc::new());
        let library = Library::load();
        let settings = AppSettings::load();

        let mut app = Self {
            innertube,
            lyrics_client,
            player,
            discord,
            downloader,
            library,
            settings,
            active_tab: ActiveTab::Search,
            search_query: String::new(),
            search_results: Vec::new(),
            is_searching: false,
            status_message: None,
            download_status: None,
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
            _dl_rx: dl_rx,
        };

        app.apply_theme(&cc.egui_ctx);

        // Auto-refresh cookies from Firefox on startup if available to keep tokens fresh
        if let Ok(ff_cookies) = BrowserAuth::extract_firefox_cookies() {
            app.settings.google_cookie = ff_cookies;
            app.settings.save();
        }

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
        Self::accent_colors()[self.settings.accent_color_idx.min(Self::accent_colors().len() - 1)]
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
    }

    fn play_next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if let Some(idx) = self.current_index {
            if idx + 1 < self.queue.len() {
                let next_idx = idx + 1;
                let next_track = self.queue[next_idx].clone();
                self.current_index = Some(next_idx);
                self.play_track(next_track);
            } else {
                // Loop to start of queue
                let next_idx = 0;
                let next_track = self.queue[next_idx].clone();
                self.current_index = Some(next_idx);
                self.play_track(next_track);
            }
        } else {
            let next_idx = 0;
            let next_track = self.queue[next_idx].clone();
            self.current_index = Some(next_idx);
            self.play_track(next_track);
        }
    }

    fn play_prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if let Some(idx) = self.current_index {
            if idx > 0 {
                let prev_idx = idx - 1;
                let prev_track = self.queue[prev_idx].clone();
                self.current_index = Some(prev_idx);
                self.play_track(prev_track);
            } else {
                // Loop to end of queue
                let prev_idx = self.queue.len() - 1;
                let prev_track = self.queue[prev_idx].clone();
                self.current_index = Some(prev_idx);
                self.play_track(prev_track);
            }
        }
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
        if let Ok(fresh_cookie) = BrowserAuth::extract_firefox_cookies() {
            self.settings.google_cookie = fresh_cookie;
            self.settings.save();
        }
        if self.settings.google_cookie.is_empty() {
            self.status_message = Some("Войдите в аккаунт для синхронизации плейлистов".to_string());
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

        if let Ok(fresh_cookie) = BrowserAuth::extract_firefox_cookies() {
            self.settings.google_cookie = fresh_cookie;
            self.settings.save();
        }
        let client = Arc::clone(&self.innertube);
        let cookie = self.settings.google_cookie.clone();
        let auth_user = self.settings.auth_user_index;
        let pid = pl.id.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch_playlist_tracks(&pid, &cookie, auth_user).await;
            let _ = tx.send(AppEvent::PlaylistTracksLoaded { playlist_id: pid, result: res });
        });
    }

    fn download_track(&mut self, track: Track) {
        self.download_status = Some(format!("Скачивание: {}", track.title));
        let (task_tx, task_rx) = mpsc::channel();
        let main_tx = self.event_tx.clone();

        self.downloader.download(track.clone(), &self.settings.download_dir, task_tx);

        thread::spawn(move || {
            if let Ok((t, res)) = task_rx.recv() {
                let _ = main_tx.send(AppEvent::DownloadFinished { track: t, result: res });
            }
        });
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
}

impl eframe::App for MetrolistApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Continuous heartbeat so Wayland ping/pong events are always processed on time
        let playback_info = self.player.info.lock().unwrap().clone();
        let repaint_interval = if playback_info.is_playing { 250 } else { 500 };
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_interval));

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
                AppEvent::LikedMusicLoaded(res) => {
                    match res {
                        Ok(tracks) => {
                            let count = tracks.len();
                            for t in tracks {
                                if !self.library.is_liked(&t.id) {
                                    self.library.liked_tracks.push(t);
                                }
                            }
                            self.library.save();
                            self.status_message = Some(format!("Синхронизировано любимых треков: {}", count));
                        }
                        Err(err) => {
                            self.status_message = Some(err);
                        }
                    }
                }
                AppEvent::PlaylistsLoaded(res) => {
                    self.is_loading_playlists = false;
                    match res {
                        Ok(pls) => {
                            let count = pls.len();
                            self.playlists = pls;
                            self.status_message = Some(format!("Синхронизировано плейлистов: {}", count));
                        }
                        Err(err) => {
                            self.status_message = Some(format!("Ошибка плейлистов: {}", err));
                        }
                    }
                }
                AppEvent::PlaylistTracksLoaded { playlist_id, result } => {
                    self.is_loading_playlist_tracks = false;
                    if self.selected_playlist.as_ref().map(|p| p.id.as_str()) == Some(&playlist_id) {
                        match result {
                            Ok(tracks) => {
                                self.playlist_tracks = tracks;
                            }
                            Err(err) => {
                                self.status_message = Some(format!("Ошибка треков плейлиста: {}", err));
                            }
                        }
                    }
                }
                AppEvent::LyricsLoaded(lyrics) => {
                    self.lyrics = lyrics;
                }
                AppEvent::DownloadFinished { track, result } => {
                    match result {
                        Ok(msg) => {
                            self.download_status = Some(format!("✓ {} ({})", track.title, msg));
                            self.library.add_downloaded(track);
                        }
                        Err(err) => {
                            self.download_status = Some(format!("Ошибка скачивания: {}", err));
                        }
                    }
                }
                AppEvent::AccountsDiscovered(accounts) => {
                    let total = accounts.len();
                    self.settings.available_accounts = accounts;

                    // Ensure selected index is valid
                    let current_user = self.settings.auth_user_index;
                    if let Some(active) = self.settings.available_accounts.iter().find(|a| a.auth_user == current_user) {
                        self.settings.account_name = Some(active.name.clone());
                    } else if let Some(first) = self.settings.available_accounts.first() {
                        self.settings.auth_user_index = first.auth_user;
                        self.settings.account_name = Some(first.name.clone());
                    }

                    self.settings.save();
                    if let Some(name) = &self.settings.account_name {
                        self.status_message = Some(format!("Аккаунтов Google: {}. Активен: {}", total, name));
                    }
                    if self.playlists.is_empty() {
                        self.sync_playlists();
                    }
                }
            }
        }

        // Auto next track when EOF reached
        if self.player.track_finished.swap(false, Ordering::SeqCst) {
            self.play_next();
        }

        let accent = self.accent_color();

        // 1. Sidebar (Left panel)
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("♫").size(24.0).color(accent));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("METROLIST").size(18.0).strong().color(accent));
                        ui.label(RichText::new("YouTube Music Client").size(10.0).weak());
                    });
                });

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);

                // Navigation buttons with reliable Unicode icons
                let tabs = [
                    (ActiveTab::Search, "🔍  Поиск"),
                    (ActiveTab::Liked, "♥  Любимые треки"),
                    (ActiveTab::Playlists, "📑  Плейлисты"),
                    (ActiveTab::Queue, "☰  Очередь"),
                    (ActiveTab::Lyrics, "♫  Текст песни"),
                    (ActiveTab::Downloads, "⬇  Скачанные"),
                    (ActiveTab::Settings, "⚙  Настройки"),
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
                        ui.label(RichText::new("●").size(10.0).color(Color32::from_rgb(16, 185, 129)));
                        ui.label(RichText::new(acc).size(11.0).strong().color(Color32::WHITE));
                    });
                    ui.add_space(6.0);
                }

                if let Some(msg) = &self.status_message {
                    ui.label(RichText::new(msg).size(11.0).color(accent));
                }

                if let Some(dl) = &self.download_status {
                    ui.add_space(6.0);
                    ui.label(RichText::new(dl).size(10.0).color(Color32::from_rgb(56, 189, 248)));
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Metrolist Rust v0.2.1").size(10.0).weak());
                });
            });

        // 2. Bottom Player Bar
        let bar_bg = if self.settings.mini_player_pure_black {
            Color32::from_rgb(0, 0, 0)
        } else {
            Color32::from_rgb(20, 20, 26)
        };

        egui::TopBottomPanel::bottom("player_bar")
            .exact_height(86.0)
            .frame(egui::Frame::none().fill(bar_bg).inner_margin(egui::Margin::symmetric(14.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Track info (Left)
                    let rounding = Rounding::same(self.settings.album_art_rounding);
                    ui.allocate_ui_with_layout(
                        Vec2::new(260.0, 64.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if let Some(track) = self.current_track() {
                                if let Some(thumb) = &track.thumbnail_url {
                                    ui.add(egui::Image::new(thumb)
                                        .fit_to_exact_size(Vec2::new(48.0, 48.0))
                                        .rounding(rounding));
                                } else {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(48.0, 48.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, rounding, Color32::from_rgb(40, 40, 50));
                                }
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&track.title).size(13.0).strong().color(Color32::WHITE));
                                    ui.label(RichText::new(&track.artist).size(11.0).weak());
                                });

                                // Like button in player bar - borderless
                                let is_liked = self.library.is_liked(&track.id);
                                let heart_color = if is_liked { Color32::from_rgb(244, 63, 94) } else { Color32::GRAY };
                                let heart_symbol = if is_liked { "♥" } else { "♡" };
                                let heart_btn = egui::Button::new(RichText::new(heart_symbol).size(18.0).color(heart_color)).frame(false);
                                if ui.add(heart_btn).clicked() {
                                    self.library.toggle_like(track.clone());
                                }
                            } else {
                                ui.label(RichText::new("Ничего не воспроизводится").weak());
                            }
                        },
                    );

                    // Central controls (Play/Pause, Slider)
                    let center_width = (ui.available_width() - 170.0).max(220.0);
                    ui.allocate_ui_with_layout(Vec2::new(center_width, 68.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                        // Buttons row
                        ui.horizontal(|ui| {
                            let controls_width = 36.0 + 16.0 + 40.0 + 16.0 + 36.0;
                            let pad = ((center_width - controls_width) / 2.0).max(0.0);
                            ui.add_space(pad);

                            let prev_btn = egui::Button::new(RichText::new("⏮").size(18.0).strong()).frame(false);
                            if ui.add_sized([36.0, 36.0], prev_btn).clicked() {
                                self.play_prev();
                            }
                            ui.add_space(16.0);

                            let play_icon = if playback_info.is_playing { "⏸" } else { "▶" };
                            let play_btn = egui::Button::new(RichText::new(play_icon).size(18.0).color(Color32::WHITE).strong())
                                .fill(accent)
                                .rounding(Rounding::same(20.0));
                            if ui.add_sized([40.0, 40.0], play_btn).clicked() {
                                self.player.toggle_pause();
                            }
                            ui.add_space(16.0);

                            let next_btn = egui::Button::new(RichText::new("⏭").size(18.0).strong()).frame(false);
                            if ui.add_sized([36.0, 36.0], next_btn).clicked() {
                                self.play_next();
                            }
                        });

                        ui.add_space(4.0);

                        // Progress slider
                        ui.horizontal(|ui| {
                            let curr_str = Self::format_time(playback_info.position_secs);
                            let dur_str = if self.settings.show_remaining_time && playback_info.duration_secs > playback_info.position_secs {
                                format!("-{}", Self::format_time(playback_info.duration_secs - playback_info.position_secs))
                            } else {
                                Self::format_time(playback_info.duration_secs)
                            };

                            let time_width = 44.0;
                            let spacing = 8.0;
                            let slider_width = (center_width - time_width * 2.0 - spacing * 2.0).max(80.0);

                            ui.add_sized([time_width, 14.0], egui::Label::new(RichText::new(curr_str).size(11.0).weak()));
                            ui.add_space(spacing);

                            let max_d = if playback_info.duration_secs > 0.0 { playback_info.duration_secs } else { 100.0 };
                            let mut pos = playback_info.position_secs;
                            let slider = egui::Slider::new(&mut pos, 0.0..=max_d)
                                .show_value(false)
                                .trailing_fill(true);
                            let resp = ui.add_sized([slider_width, 14.0], slider);
                            if resp.dragged() || resp.clicked() {
                                self.player.seek(pos);
                            }
                            ui.add_space(spacing);

                            ui.add_sized([time_width, 14.0], egui::Label::new(RichText::new(dur_str).size(11.0).weak()));
                        });
                    });

                    // Volume slider & Download current track (Right)
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut vol = playback_info.volume;
                        ui.label(format!("{}%", vol as u32));
                        let vol_slider = egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false);
                        if ui.add_sized([80.0, 12.0], vol_slider).changed() {
                            self.player.set_volume(vol);
                        }
                        ui.label(RichText::new("🔊").size(14.0));

                        if let Some(track) = self.current_track() {
                            let track_clone = track.clone();
                            if ui.button(RichText::new("⬇").size(14.0)).on_hover_text("Скачать этот трек").clicked() {
                                self.download_track(track_clone);
                            }
                        }
                    });
                });
            });

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
                ui.label(RichText::new("♫ Введите название трека или исполнителя для поиска").size(15.0).weak());
            });
            return;
        }

        // List results
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                            ui.add(egui::Image::new(thumb)
                                .fit_to_exact_size(Vec2::new(44.0, 44.0))
                                .rounding(rounding));
                        } else {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(44.0, 44.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, rounding, Color32::from_rgb(45, 45, 55));
                        }
                        ui.add_space(10.0);

                        // Title & Artist
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&track.title).size(14.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(format!("{} • {}", track.artist, track.duration_text)).size(12.0).weak());
                        });

                        // Right action buttons
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let play_btn = egui::Button::new(RichText::new("▶ Играть").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_btn).clicked() {
                                track_idx_to_play = Some(idx);
                            }

                            // Like button
                            let is_liked = self.library.is_liked(&track.id);
                            let heart_col = if is_liked { Color32::from_rgb(244, 63, 94) } else { Color32::GRAY };
                            let heart_symbol = if is_liked { "♥" } else { "♡" };
                            if ui.button(RichText::new(heart_symbol).size(15.0).color(heart_col)).clicked() {
                                self.library.toggle_like(track.clone());
                            }

                            // Add to queue
                            if ui.button("+ Очередь").clicked() {
                                track_to_queue = Some(track.clone());
                            }

                            // Download
                            if ui.button("⬇").on_hover_text("Скачать MP3").clicked() {
                                track_to_download = Some(track.clone());
                            }
                        });
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
                    if let Ok(fresh_cookie) = BrowserAuth::extract_firefox_cookies() {
                        self.settings.google_cookie = fresh_cookie;
                        self.settings.save();
                    }
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
                let dl_all_btn = egui::Button::new(RichText::new("⬇ Скачать все").strong())
                    .fill(Color32::from_rgb(32, 45, 58))
                    .rounding(Rounding::same(6.0));
                if ui.add(dl_all_btn).on_hover_text("Скачать все любимые треки в папку Музыка").clicked() {
                    self.download_playlist(self.library.liked_tracks.clone());
                }
            }
        });

        ui.add_space(12.0);

        if self.library.liked_tracks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("У вас пока нет любимых треков.").size(15.0).weak());
                ui.add_space(6.0);
                ui.label(RichText::new("Нажмите «Синхронизировать с YouTube» или ставьте ♥ в поиске.").size(12.0).weak());
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                            ui.add(egui::Image::new(thumb)
                                .fit_to_exact_size(Vec2::new(40.0, 40.0))
                                .rounding(rounding));
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&track.title).size(13.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(&track.artist).size(11.0).weak());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let play_btn = egui::Button::new(RichText::new("▶ Играть").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_btn).clicked() {
                                track_idx_to_play = Some(idx);
                            }

                            if ui.button(RichText::new("♥").size(15.0).color(Color32::from_rgb(244, 63, 94))).clicked() {
                                track_to_toggle_like = Some(track.clone());
                            }

                            if ui.button("⬇").on_hover_text("Скачать MP3").clicked() {
                                track_to_download = Some(track.clone());
                            }
                        });
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
                    ui.add(egui::Image::new(thumb)
                        .fit_to_exact_size(Vec2::new(64.0, 64.0))
                        .rounding(Rounding::same(self.settings.album_art_rounding)));
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&pl.subtitle).size(12.0).weak());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if !self.playlist_tracks.is_empty() {
                            let play_all_btn = egui::Button::new(RichText::new("▶ Слушать всё").color(Color32::WHITE).strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_all_btn).clicked() {
                                self.queue = self.playlist_tracks.clone();
                                if let Some(first) = self.playlist_tracks.first().cloned() {
                                    self.play_track(first);
                                }
                            }

                            if ui.button("+ Вся в очередь").clicked() {
                                self.queue.extend(self.playlist_tracks.clone());
                                self.status_message = Some("Плейлист добавлен в очередь".to_string());
                            }

                            // Download whole playlist button
                            let dl_all_btn = egui::Button::new(RichText::new("⬇ Скачать весь плейлист").strong())
                                .fill(Color32::from_rgb(32, 45, 58))
                                .rounding(Rounding::same(6.0));
                            if ui.add(dl_all_btn).on_hover_text("Скачать все треки из этого плейлиста в папку Музыка").clicked() {
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
                                ui.add(egui::Image::new(thumb)
                                    .fit_to_exact_size(Vec2::new(36.0, 36.0))
                                    .rounding(rounding));
                            }
                            ui.add_space(8.0);

                            ui.vertical(|ui| {
                                ui.label(RichText::new(&track.title).size(13.0).strong().color(Color32::WHITE));
                                ui.label(RichText::new(&track.artist).size(11.0).weak());
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(RichText::new("▶").strong()).clicked() {
                                    track_idx_to_play = Some(idx);
                                }
                                if ui.button("+").on_hover_text("Добавить в очередь").clicked() {
                                    track_to_queue = Some(track.clone());
                                }
                                if ui.button("⬇").on_hover_text("Скачать MP3").clicked() {
                                    track_to_download = Some(track.clone());
                                }
                            });
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

        // 2. Playlists grid / list
        ui.horizontal(|ui| {
            ui.label(RichText::new("📑 Плейлисты YouTube Music").size(18.0).strong());
            ui.add_space(16.0);
            if ui.button(RichText::new("🔄 Синхронизировать плейлисты").strong()).clicked() {
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
                            ui.add(egui::Image::new(thumb)
                                .fit_to_exact_size(Vec2::new(56.0, 56.0))
                                .rounding(rounding));
                        } else {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(56.0, 56.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, rounding, Color32::from_rgb(45, 45, 55));
                        }
                        ui.add_space(10.0);

                        ui.vertical(|ui| {
                            ui.label(RichText::new(&pl.title).size(15.0).strong().color(Color32::WHITE));
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
            ui.label(RichText::new("☰ Очередь воспроизведения").size(18.0).strong());
            ui.add_space(20.0);
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
                            ui.label(RichText::new(&track.title).size(13.0).strong().color(title_color));
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
            self.play_track(t);
        }
    }

    fn render_lyrics_tab(&mut self, ui: &mut egui::Ui, current_time: f64) {
        let accent = self.accent_color();

        ui.label(RichText::new("♫ Текст песни").size(18.0).strong());
        ui.add_space(12.0);

        let Some(lyrics) = &self.lyrics else {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new("Текст песни не найден или трек ещё не воспроизводится").weak());
            });
            return;
        };

        if !lyrics.is_synced {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(RichText::new(&lyrics.plain).size(15.0).line_height(Some(24.0)));
            });
            return;
        }

        // Synced lyrics with auto-scroll
        let current_line_idx = lyrics.lines.iter().rposition(|l| l.seconds <= current_time);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (idx, line) in lyrics.lines.iter().enumerate() {
                let is_active = Some(idx) == current_line_idx;
                let text = if is_active {
                    RichText::new(&line.text)
                        .size(18.0)
                        .strong()
                        .color(accent)
                } else if Some(idx) < current_line_idx {
                    RichText::new(&line.text)
                        .size(15.0)
                        .color(Color32::from_rgb(120, 120, 130))
                } else {
                    RichText::new(&line.text)
                        .size(15.0)
                        .color(Color32::from_rgb(200, 200, 210))
                };

                let resp = ui.label(text);
                if is_active && self.settings.lyrics_auto_scroll {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
                ui.add_space(8.0);
            }
        });
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
                ui.label(RichText::new(format!("Папка для сохранения: {}", self.settings.download_dir)).size(11.0).weak());
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
                            ui.add(egui::Image::new(thumb)
                                .fit_to_exact_size(Vec2::new(40.0, 40.0))
                                .rounding(rounding));
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&track.title).size(13.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(&track.artist).size(11.0).weak());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let play_btn = egui::Button::new(RichText::new("▶ Играть").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_btn).clicked() {
                                track_to_play = Some(track.clone());
                            }
                            ui.label(RichText::new("✓ MP3").size(11.0).color(Color32::from_rgb(16, 185, 129)));
                        });
                    });
                });
                ui.add_space(4.0);
            }

            if let Some(t) = track_to_play {
                self.play_track(t);
            }
        });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("⚙ Настройки Metrolist").size(20.0).strong());
        ui.add_space(14.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // 1. АККАУНТ YOUTUBE MUSIC (Account Settings)
            ui.group(|ui| {
                ui.label(RichText::new("👤 Аккаунты Google / YouTube Music").size(15.0).strong());
                ui.add_space(6.0);

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
                        // Refresh accounts button
                        if ui.button("🔄 Обновить список аккаунтов").clicked() {
                            let client = Arc::clone(&self.innertube);
                            let cookie = self.settings.google_cookie.clone();
                            let tx = self.event_tx.clone();
                            tokio::spawn(async move {
                                let accounts = client.discover_accounts(&cookie).await;
                                let _ = tx.send(AppEvent::AccountsDiscovered(accounts));
                            });
                        }

                        // Logout button
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

                    // ONE-CLICK FIREFOX LOGIN BUTTON
                    let ff_btn = egui::Button::new(RichText::new("🦊 Войти через Firefox в один клик").size(13.0).strong().color(Color32::WHITE))
                        .fill(Color32::from_rgb(234, 88, 12)) // Firefox Orange
                        .rounding(Rounding::same(8.0));

                    if ui.add_sized([260.0, 36.0], ff_btn).clicked() {
                        match BrowserAuth::extract_firefox_cookies() {
                            Ok(cookie) => {
                                self.settings.google_cookie = cookie.clone();
                                self.settings.save();
                                self.status_message = Some("Куки получены! Ищем аккаунты Google...".to_string());

                                let client = Arc::clone(&self.innertube);
                                let tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    let accounts = client.discover_accounts(&cookie).await;
                                    let _ = tx.send(AppEvent::AccountsDiscovered(accounts));
                                });
                            }
                            Err(err) => {
                                self.status_message = Some(format!("Ошибка импорта: {}", err));
                            }
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Автоматически обнаруживает все ваши аккаунты Google в Firefox и даёт выбрать нужный.").size(11.0).weak());

                    ui.add_space(8.0);
                    // Collapsible manual cookie entry
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

            ui.add_space(10.0);

            // 2. ВНЕШНИЙ ВИД И КАСТОМИЗАЦИЯ ИНТЕРФЕЙСА (Appearance & Customization from Metrolist)
            ui.group(|ui| {
                ui.label(RichText::new("🎨 Внешний вид и Кастомизация (Metrolist Style)").size(15.0).strong());
                ui.add_space(6.0);

                // Тема
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
                // Акцент
                ui.horizontal(|ui| {
                    ui.label("Акцентный цвет:");
                    let color_names = ["Изумруд (Metrolist)", "Рубин", "Аметист", "Кибер-синий", "Янтарь"];
                    for (i, name) in color_names.iter().enumerate() {
                        let color = Self::accent_colors()[i];
                        let is_sel = self.settings.accent_color_idx == i;
                        let btn = egui::Button::new(RichText::new(*name).color(Color32::WHITE))
                            .fill(if is_sel { color } else { Color32::from_rgb(35, 35, 45) });
                        if ui.add(btn).clicked() {
                            self.settings.accent_color_idx = i;
                            self.settings.save();
                            self.apply_theme(ctx);
                        }
                    }
                });

                ui.add_space(8.0);
                // Плотность интерфейса (Density scale)
                ui.horizontal(|ui| {
                    ui.label("Плотность интерфейса (Density):");
                    let scales = [(0.85, "Компактная (85%)"), (1.0, "Стандартная (100%)"), (1.15, "Просторная (115%)")];
                    for (s, label) in scales {
                        let is_sel = (self.settings.density_scale - s).abs() < 0.05;
                        if ui.selectable_label(is_sel, label).clicked() {
                            self.settings.density_scale = s;
                            self.settings.save();
                        }
                    }
                });

                ui.add_space(8.0);
                // Скругление обложек
                ui.horizontal(|ui| {
                    ui.label("Скругление обложек альбомов:");
                    let roundings = [(4.0, "4 px"), (8.0, "8 px (Metrolist)"), (16.0, "16 px"), (24.0, "Круглые")];
                    for (r, label) in roundings {
                        let is_sel = (self.settings.album_art_rounding - r).abs() < 0.5;
                        if ui.selectable_label(is_sel, label).clicked() {
                            self.settings.album_art_rounding = r;
                            self.settings.save();
                        }
                    }
                });

                ui.add_space(6.0);
                // Переключатели кастомизации
                if ui.checkbox(&mut self.settings.show_remaining_time, "Отображать оставшееся время трека (-01:23) вместо полной длины").changed() {
                    self.settings.save();
                }

                if ui.checkbox(&mut self.settings.lyrics_auto_scroll, "Автоматическая плавная прокрутка синхронного текста песни").changed() {
                    self.settings.save();
                }

                if ui.checkbox(&mut self.settings.mini_player_pure_black, "Чисто чёрный AMOLED фон нижней панели плеера").changed() {
                    self.settings.save();
                }
            });

            ui.add_space(10.0);

            // 3. ПЛЕЕР И АУДИО (Player & Audio)
            ui.group(|ui| {
                ui.label(RichText::new("♫ Плеер и Звук").size(15.0).strong());
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("Качество потока:");
                    egui::ComboBox::from_id_salt("quality")
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
                if ui.checkbox(&mut self.settings.audio_normalization, "Нормализация громкости (Loudness Normalization / ReplayGain)").changed() {
                    self.settings.save();
                }

                if ui.checkbox(&mut self.settings.skip_silence, "Пропуск тишины в начале и конце треков").changed() {
                    self.settings.save();
                }

                if ui.checkbox(&mut self.settings.sponsorblock_enabled, "SponsorBlock (автоматический пропуск немузыкальных фрагментов)").changed() {
                    self.settings.save();
                }
            });

            ui.add_space(10.0);

            // 4. ИНТЕГРАЦИЯ С DISCORD (Discord RPC)
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🎮 Discord Rich Presence").size(15.0).strong());
                    ui.add_space(10.0);
                    ui.label(RichText::new("🟢 IPC подключение активно").size(11.0).color(Color32::from_rgb(16, 185, 129)));
                });
                ui.add_space(6.0);

                let mut enabled = self.settings.discord_rpc_enabled;
                if ui.checkbox(&mut enabled, "Отображать в Discord статус прослушивания").changed() {
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
                if ui.checkbox(&mut self.settings.discord_show_buttons, "Показывать кнопки «Слушать в YouTube Music» и «Metrolist»").changed() {
                    self.settings.save();
                }

                ui.add_space(4.0);
                ui.label(RichText::new("В профиле Discord показывается точное название песни («Слушает <Название трека>»), исполнитель и таймкод.").size(11.0).weak());
            });

            ui.add_space(10.0);

            // 5. КОНТЕНТ И ЯЗЫК (Content & Language)
            ui.group(|ui| {
                ui.label(RichText::new("🌐 Контент и Язык").size(15.0).strong());
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("Регион контента (GL):");
                    let countries = ["RU", "US", "DE", "GB", "JP", "KR"];
                    for c in countries {
                        let is_sel = self.settings.content_country == c;
                        if ui.selectable_label(is_sel, c).clicked() {
                            self.settings.content_country = c.to_string();
                            self.settings.save();
                        }
                    }
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Язык интерфейса и треков (HL):");
                    let langs = [("ru", "Русский"), ("en", "English")];
                    for (code, name) in langs {
                        let is_sel = self.settings.content_language == code;
                        if ui.selectable_label(is_sel, name).clicked() {
                            self.settings.content_language = code.to_string();
                            self.settings.save();
                        }
                    }
                });
            });

            ui.add_space(10.0);

            // 6. ХРАНИЛИЩЕ И ЗАГРУЗКИ (Storage)
            ui.group(|ui| {
                ui.label(RichText::new("💾 Хранилище и Загрузки").size(15.0).strong());
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("Папка загрузок:");
                    let edit = egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width(ui.available_width() - 100.0);
                    if ui.add(edit).lost_focus() {
                        self.settings.save();
                    }
                    if ui.button("Обзор").clicked() {
                        let dir = self.settings.download_dir.clone();
                        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                    }
                });

                ui.add_space(4.0);
                ui.label(RichText::new("Треки скачиваются с обложками в высоком разрешении и ID3-метаданными исполнителей.").size(11.0).weak());
            });

            ui.add_space(10.0);

            // 7. О ПРОГРАММЕ (About)
            ui.group(|ui| {
                ui.label(RichText::new("ℹ О приложении Metrolist").size(15.0).strong());
                ui.add_space(6.0);
                ui.label(RichText::new("Metrolist Desktop v0.2.1 (Rust / Wayland)").strong());
                ui.label(RichText::new("Портировано с оригинального Android-клиента Metrolist:").size(12.0).weak());
                if ui.link("github.com/metrolistgroup/metrolist").clicked() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg("https://github.com/metrolistgroup/metrolist")
                        .spawn();
                }
            });

            ui.add_space(20.0);
        });
    }
}
