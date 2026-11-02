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
    CookiesExtracted(Result<String, String>),
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

    fn shuffle_queue(&mut self) {
        if self.queue.len() <= 1 { return; }
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        for i in (1..self.queue.len()).rev() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (seed >> 32) as usize % (i + 1);
            self.queue.swap(i, j);
        }
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
        let playback_info = self.player.info.lock().unwrap().clone();
        // Snappy heartbeat: 16ms when high_refresh_rate (120Hz) is enabled, guarantees 0 Wayland stalls!
        let repaint_interval = if self.settings.high_refresh_rate {
            if playback_info.is_playing { 16 } else { 50 }
        } else {
            if playback_info.is_playing { 40 } else { 80 }
        };
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
                AppEvent::CookiesExtracted(res) => {
                    match res {
                        Ok(fresh_cookie) => {
                            if !fresh_cookie.is_empty() {
                                let was_empty = self.settings.google_cookie.is_empty();
                                self.settings.google_cookie = fresh_cookie.clone();
                                self.settings.save();
                                if was_empty {
                                    self.status_message = Some("Куки из Firefox успешно получены!".to_string());
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
                                    let pls = client2.fetch_user_playlists(&cookie_for_pls, auth_user).await;
                                    let _ = tx2.send(AppEvent::PlaylistsLoaded(pls));
                                });
                            }
                        }
                        Err(err) => {
                            if self.settings.google_cookie.is_empty() {
                                self.status_message = Some(format!("Firefox: {}", err));
                            }
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

        // 1. Sidebar (Left panel - hidden in fullscreen NowPlaying if enabled)
        let show_sidebar = !(self.active_tab == ActiveTab::NowPlaying && self.settings.hide_status_bar_fullscreen);
        if show_sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(false)
                .exact_width(if self.settings.slim_nav_bar { 180.0 } else { 220.0 })
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
        }

        // 2. Bottom Player Bar (only visible when not in full NowPlaying tab)
        if self.active_tab != ActiveTab::NowPlaying {
            let bar_bg = if self.settings.mini_player_bg_style == 1 || self.settings.mini_player_pure_black {
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
                .exact_height(80.0)
                .frame(egui::Frame::none().fill(bar_bg).inner_margin(egui::Margin::symmetric(14.0, 8.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        // Left: Track thumbnail + Title & Artist + Pill Heart button [ ❤️ ]
                        let rounding = Rounding::same(self.settings.album_art_rounding);
                        ui.allocate_ui_with_layout(
                            Vec2::new(320.0, 64.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                if let Some(track) = &current_track_clone {
                                    // Clicking cover art opens Now Playing screen
                                    let img_resp = if let Some(thumb) = &track.thumbnail_url {
                                        ui.add(egui::Image::new(thumb)
                                            .fit_to_exact_size(Vec2::new(48.0, 48.0))
                                            .rounding(rounding))
                                    } else {
                                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(48.0, 48.0), egui::Sense::click());
                                        ui.painter().rect_filled(rect, rounding, Color32::from_rgb(40, 40, 50));
                                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♫", egui::FontId::proportional(20.0), Color32::GRAY);
                                        resp
                                    };

                                    if img_resp.clicked() {
                                        open_now_playing = true;
                                    }

                                    ui.add_space(8.0);

                                    // Clicking title or artist opens Now Playing screen
                                    let text_resp = ui.vertical(|ui| {
                                        ui.label(RichText::new(&track.title).size(13.0).strong().color(Color32::WHITE));
                                        ui.label(RichText::new(&track.artist).size(11.0).weak());
                                    }).response;

                                    if text_resp.clicked() {
                                        open_now_playing = true;
                                    }

                                    ui.add_space(8.0);

                                    // Pill Heart button [ ❤️ ] matching user screenshot!
                                    let is_liked = self.library.is_liked(&track.id);
                                    let heart_color = if is_liked { Color32::from_rgb(244, 63, 94) } else { Color32::from_rgb(160, 160, 175) };
                                    let heart_symbol = if is_liked { "♥" } else { "♡" };
                                    let heart_btn = egui::Button::new(RichText::new(heart_symbol).size(15.0).color(heart_color))
                                        .fill(Color32::from_rgb(44, 44, 52))
                                        .rounding(Rounding::same(6.0));
                                    if ui.add_sized([30.0, 26.0], heart_btn).on_hover_text("Понравилось").clicked() {
                                        track_to_toggle_like = Some(track.clone());
                                    }
                                } else {
                                    ui.label(RichText::new("Ничего не воспроизводится").weak());
                                }
                            },
                        );

                        // Center: Controls row & Time Slider underneath matching screenshot!
                        let center_width = (ui.available_width() - 140.0).max(220.0);
                        ui.allocate_ui_with_layout(Vec2::new(center_width, 68.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                            // Row 1: Buttons
                            ui.horizontal(|ui| {
                                let prev_btn = egui::Button::new(RichText::new("⏮").size(18.0).strong().color(Color32::from_rgb(210, 210, 220))).frame(false);
                                if ui.add_sized([32.0, 32.0], prev_btn).clicked() {
                                    self.play_prev();
                                }
                                ui.add_space(14.0);

                                let play_icon = if playback_info.is_playing { "⏸" } else { "▶" };
                                let play_btn = egui::Button::new(RichText::new(play_icon).size(18.0).color(Color32::WHITE).strong())
                                    .fill(Color32::from_rgb(29, 185, 84)) // Official Metrolist Emerald Accent!
                                    .rounding(Rounding::same(20.0));
                                if ui.add_sized([40.0, 40.0], play_btn).clicked() {
                                    self.player.toggle_pause();
                                }
                                ui.add_space(14.0);

                                let next_btn = egui::Button::new(RichText::new("⏭").size(18.0).strong().color(Color32::from_rgb(210, 210, 220))).frame(false);
                                if ui.add_sized([32.0, 32.0], next_btn).clicked() {
                                    self.play_next();
                                }
                            });

                            ui.add_space(2.0);

                            // Row 2: Progress Slider with times directly underneath controls
                            ui.horizontal(|ui| {
                                let curr_str = Self::format_time(playback_info.position_secs);
                                let dur_str = if self.settings.show_remaining_time && playback_info.duration_secs > playback_info.position_secs {
                                    format!("-{}", Self::format_time(playback_info.duration_secs - playback_info.position_secs))
                                } else {
                                    Self::format_time(playback_info.duration_secs)
                                };

                                let time_width = 38.0;
                                let spacing = 6.0;
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

                        // Right: Volume & Expand Fullscreen button
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Expand to Now Playing button
                            let expand_btn = egui::Button::new(RichText::new("⛶").size(16.0).color(Color32::from_rgb(200, 200, 210))).frame(false);
                            if ui.add(expand_btn).on_hover_text("Развернуть плеер").clicked() {
                                open_now_playing = true;
                            }

                            ui.add_space(8.0);

                            let mut vol = playback_info.volume;
                            let vol_slider = egui::Slider::new(&mut vol, 0.0..=100.0).show_value(false);
                            if ui.add_sized([60.0, 12.0], vol_slider).changed() {
                                self.player.set_volume(vol);
                            }
                            ui.label(RichText::new("🔊").size(13.0));
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

        // 2. Auto-created playlists (photo_1)
        let any_auto = self.settings.show_liked_playlist || self.settings.show_downloaded_playlist || self.settings.show_top_playlist || self.settings.show_cached_playlist || self.settings.show_uploaded_playlist;
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
                    if ui.add_sized([160.0, 36.0], btn).clicked() {
                        if !self.library.liked_tracks.is_empty() {
                            self.queue = self.library.liked_tracks.clone();
                            if let Some(first) = self.library.liked_tracks.first().cloned() {
                                self.play_track(first);
                            }
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
                self.play_track(t.clone());
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

            // 2. АВТОСОЗДАННЫЕ ПЛЕЙЛИСТЫ (photo_1)
            ui.group(|ui| {
                ui.label(RichText::new("📑 Автосозданные плейлисты").size(15.0).strong());
                ui.label(RichText::new("Отображать автосозданные плейлисты в библиотеке").size(11.0).weak());
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

            ui.add_space(10.0);

            // 3. ВНЕШНИЙ ВИД И ТЕМА (photo_3)
            ui.group(|ui| {
                ui.label(RichText::new("🎨 Внешний вид и Тема").size(15.0).strong());
                ui.add_space(6.0);

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

                ui.add_space(6.0);
                if ui.checkbox(&mut self.settings.high_refresh_rate, "Включить высокую частоту обновления (120 Гц)").changed() {
                    self.settings.save();
                }
                if ui.checkbox(&mut self.settings.landscape_scaling, "Масштабирование в альбомной ориентации").changed() {
                    self.settings.save();
                }
                if ui.checkbox(&mut self.settings.dynamic_icon_colors, "Динамические цвета значка").changed() {
                    self.settings.save();
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Плотность интерфейса:");
                    let scales = [(0.85, "Компактная (85%)"), (1.0, "Стандартная (100%)"), (1.15, "Просторная (115%)")];
                    for (s, label) in scales {
                        let is_sel = (self.settings.density_scale - s).abs() < 0.05;
                        if ui.selectable_label(is_sel, label).clicked() {
                            self.settings.density_scale = s;
                            self.settings.save();
                        }
                    }
                });
            });

            ui.add_space(10.0);

            // 4. МИНИ-ПЛЕЕР И ПЛЕЕР (photo_4, photo_5, photo_6)
            ui.group(|ui| {
                ui.label(RichText::new("▶ Мини-плеер и Плеер").size(15.0).strong());
                ui.add_space(6.0);

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
                    let slider_styles = ["Прямой", "Волнистый", "Тонкий"];
                    for (i, name) in slider_styles.iter().enumerate() {
                        if ui.selectable_label(self.settings.slider_style == i, *name).clicked() {
                            self.settings.slider_style = i;
                            self.settings.save();
                        }
                    }
                });

                ui.add_space(4.0);
                if ui.checkbox(&mut self.settings.swipe_track_switch, "Включить переключение треков свайпом").changed() {
                    self.settings.save();
                }

                ui.horizontal(|ui| {
                    ui.label("Чувствительность свайпа мини-плеера:");
                    let mut sens = self.settings.swipe_sensitivity;
                    if ui.add(egui::Slider::new(&mut sens, 0.5..=2.0).text("x")).changed() {
                        self.settings.swipe_sensitivity = sens;
                        self.settings.save();
                    }
                });

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

            ui.add_space(10.0);

            // 5. ТЕКСТ ПЕСНИ (photo_7, photo_8)
            ui.group(|ui| {
                ui.label(RichText::new("♫ Текст песни (Lyrics)").size(15.0).strong());
                ui.add_space(6.0);

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
                if ui.checkbox(&mut self.settings.lyrics_change_on_tap, "Менять текст песни по касанию").changed() {
                    self.settings.save();
                }
                if ui.checkbox(&mut self.settings.lyrics_auto_scroll, "Автоматическая плавная прокрутка текста").changed() {
                    self.settings.save();
                }
            });

            ui.add_space(10.0);

            // 6. ПОЛНОЭКРАННЫЙ РЕЖИМ И НАВИГАЦИЯ (photo_8, photo_9, photo_10)
            ui.group(|ui| {
                ui.label(RichText::new("📱 Полноэкранный режим и Навигация").size(15.0).strong());
                ui.add_space(6.0);

                if ui.checkbox(&mut self.settings.hide_status_bar_fullscreen, "Скрывать строку состояния в полноэкранном режиме").changed() {
                    self.settings.save();
                }

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
                if ui.checkbox(&mut self.settings.slim_nav_bar, "Тонкая панель навигации").changed() {
                    self.settings.save();
                }

                ui.horizontal(|ui| {
                    ui.label("Размер ячейки сетки:");
                    let grid_sizes = ["Маленький", "Средний", "Большой"];
                    for (i, name) in grid_sizes.iter().enumerate() {
                        if ui.selectable_label(self.settings.grid_cell_size == i, *name).clicked() {
                            self.settings.grid_cell_size = i;
                            self.settings.save();
                        }
                    }
                });
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

    fn render_now_playing_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();
        let playback_info = self.player.info.lock().unwrap().clone();
        let current_track = self.current_track().cloned();

        // 1. Top Bar: ← Назад, "СЕЙЧАС ИГРАЕТ" / Source name, Share, Download
        ui.horizontal(|ui| {
            let back_btn = egui::Button::new(RichText::new("← Назад").size(14.0).strong().color(Color32::WHITE))
                .fill(Color32::from_rgb(32, 32, 40))
                .rounding(Rounding::same(8.0));
            if ui.add_sized([90.0, 32.0], back_btn).clicked() {
                self.active_tab = self.previous_tab;
                return;
            }

            let center_w = (ui.available_width() - 100.0).max(100.0);
            ui.allocate_ui_with_layout(Vec2::new(center_w, 36.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label(RichText::new("СЕЙЧАС ИГРАЕТ").size(10.0).weak());
                let source_title = self.selected_playlist.as_ref().map(|p| p.title.as_str()).unwrap_or("Очередь воспроизведения");
                ui.label(RichText::new(source_title).size(13.0).strong().color(Color32::WHITE));
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(track) = &current_track {
                    let track_clone = track.clone();
                    if ui.button(RichText::new("⬇").size(15.0)).on_hover_text("Скачать трек").clicked() {
                        self.download_track(track_clone);
                    }
                }
            });
        });

        ui.add_space(16.0);

        // 2. Central content: large album art, track info, slider, controls
        let max_content_w = 440.0;
        let content_x = ((ui.available_width() - max_content_w) / 2.0).max(0.0);

        ui.horizontal(|ui| {
            ui.add_space(content_x);
            ui.allocate_ui_with_layout(Vec2::new(max_content_w, ui.available_height()), egui::Layout::top_down(egui::Align::Center), |ui| {
                let art_size = 320.0;
                let art_rounding = Rounding::same(16.0);

                if !self.settings.hide_player_thumbnail {
                    if let Some(track) = &current_track {
                        if let Some(thumb) = &track.thumbnail_url {
                            ui.add(egui::Image::new(thumb)
                                .fit_to_exact_size(Vec2::new(art_size, art_size))
                                .rounding(art_rounding));
                        } else {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(art_size, art_size), egui::Sense::hover());
                            ui.painter().rect_filled(rect, art_rounding, Color32::from_rgb(32, 32, 40));
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♫", egui::FontId::proportional(64.0), Color32::GRAY);
                        }
                    } else {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(art_size, art_size), egui::Sense::hover());
                        ui.painter().rect_filled(rect, art_rounding, Color32::from_rgb(32, 32, 40));
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♫", egui::FontId::proportional(64.0), Color32::GRAY);
                    }
                }

                ui.add_space(20.0);

                // Track title & artist + Share & Heart
                ui.allocate_ui_with_layout(Vec2::new(max_content_w, 54.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.allocate_ui_with_layout(Vec2::new(max_content_w - 90.0, 50.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        if let Some(track) = &current_track {
                            ui.label(RichText::new(&track.title).size(18.0).strong().color(Color32::WHITE));
                            ui.label(RichText::new(&track.artist).size(13.0).weak());
                        } else {
                            ui.label(RichText::new("Ничего не играет").size(16.0).weak());
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(track) = &current_track {
                            let is_liked = self.library.is_liked(&track.id);
                            let heart_color = if is_liked { Color32::from_rgb(244, 63, 94) } else { Color32::GRAY };
                            let heart_symbol = if is_liked { "♥" } else { "♡" };
                            let heart_btn = egui::Button::new(RichText::new(heart_symbol).size(22.0).color(heart_color)).frame(false);
                            if ui.add(heart_btn).clicked() {
                                self.library.toggle_like(track.clone());
                            }

                            ui.add_space(8.0);

                            let share_btn = egui::Button::new(RichText::new("↗").size(20.0).color(Color32::WHITE)).frame(false);
                            if ui.add(share_btn).on_hover_text("Копировать ссылку на трек").clicked() {
                                let link = format!("https://music.youtube.com/watch?v={}", track.id);
                                ui.output_mut(|o| o.copied_text = link);
                                self.status_message = Some("Ссылка на трек скопирована!".to_string());
                            }
                        }
                    });
                });

                ui.add_space(14.0);

                // Slider and Time
                let max_d = if playback_info.duration_secs > 0.0 { playback_info.duration_secs } else { 100.0 };
                let mut pos = playback_info.position_secs;
                let slider = egui::Slider::new(&mut pos, 0.0..=max_d)
                    .show_value(false)
                    .trailing_fill(true);
                let resp = ui.add_sized([max_content_w, 16.0], slider);
                if resp.dragged() || resp.clicked() {
                    self.player.seek(pos);
                }

                ui.add_space(4.0);

                ui.allocate_ui_with_layout(Vec2::new(max_content_w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let curr_str = Self::format_time(playback_info.position_secs);
                    let dur_str = if self.settings.show_remaining_time && playback_info.duration_secs > playback_info.position_secs {
                        format!("-{}", Self::format_time(playback_info.duration_secs - playback_info.position_secs))
                    } else {
                        Self::format_time(playback_info.duration_secs)
                    };
                    ui.label(RichText::new(curr_str).size(12.0).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(dur_str).size(12.0).weak());
                    });
                });

                ui.add_space(20.0);

                // Controls row: Previous pill, Play/Pause large pill with text and icon, Next pill
                ui.horizontal(|ui| {
                    let prev_btn = egui::Button::new(RichText::new("⏮").size(20.0).strong().color(Color32::WHITE))
                        .fill(Color32::from_rgb(34, 34, 44))
                        .rounding(Rounding::same(24.0));
                    if ui.add_sized([52.0, 48.0], prev_btn).clicked() {
                        self.play_prev();
                    }

                    ui.add_space(16.0);

                    let (icon, text) = if playback_info.is_playing { ("⏸", "Пауза") } else { ("▶", "Играть") };
                    let play_btn = egui::Button::new(RichText::new(format!("{}  {}", icon, text)).size(16.0).color(Color32::WHITE).strong())
                        .fill(accent)
                        .rounding(Rounding::same(24.0));
                    if ui.add_sized([150.0, 48.0], play_btn).clicked() {
                        self.player.toggle_pause();
                    }

                    ui.add_space(16.0);

                    let next_btn = egui::Button::new(RichText::new("⏭").size(20.0).strong().color(Color32::WHITE))
                        .fill(Color32::from_rgb(34, 34, 44))
                        .rounding(Rounding::same(24.0));
                    if ui.add_sized([52.0, 48.0], next_btn).clicked() {
                        self.play_next();
                    }
                });

                ui.add_space(24.0);

                // Bottom Action Icons Row (Queue, Sleep, Shuffle, Lyrics, Repeat)
                ui.horizontal(|ui| {
                    let queue_btn = egui::Button::new(RichText::new("☰").size(18.0).color(Color32::WHITE)).frame(false);
                    if ui.add_sized([42.0, 36.0], queue_btn).on_hover_text("Очередь треков").clicked() {
                        self.active_tab = ActiveTab::Queue;
                    }

                    ui.add_space(14.0);

                    let shuffle_col = if self.shuffle_state { accent } else { Color32::GRAY };
                    let shuffle_btn = egui::Button::new(RichText::new("🔀").size(18.0).color(shuffle_col)).frame(false);
                    if ui.add_sized([42.0, 36.0], shuffle_btn).on_hover_text("Перемешать очередь").clicked() {
                        self.shuffle_state = !self.shuffle_state;
                        if self.shuffle_state {
                            self.shuffle_queue();
                            self.status_message = Some("Очередь перемешана".to_string());
                        }
                    }

                    ui.add_space(14.0);

                    let lyrics_btn = egui::Button::new(RichText::new("♫").size(18.0).color(Color32::WHITE)).frame(false);
                    if ui.add_sized([42.0, 36.0], lyrics_btn).on_hover_text("Текст песни").clicked() {
                        self.active_tab = ActiveTab::Lyrics;
                    }

                    ui.add_space(14.0);

                    let (rep_icon, rep_col) = match self.repeat_mode {
                        1 => ("🔁", accent),
                        2 => ("🔂", accent),
                        _ => ("🔁", Color32::GRAY),
                    };
                    let rep_btn = egui::Button::new(RichText::new(rep_icon).size(18.0).color(rep_col)).frame(false);
                    if ui.add_sized([42.0, 36.0], rep_btn).on_hover_text("Режим повтора").clicked() {
                        self.repeat_mode = (self.repeat_mode + 1) % 3;
                    }
                });
            });
        });
    }
}
