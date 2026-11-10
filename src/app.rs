use crate::browser_auth::BrowserAuth;
use crate::discord::{ActivityDetails, DiscordRpc};
use crate::downloader::Downloader;
use crate::innertube::{InnertubeClient, PlaylistChoice, PlaylistMembership};
use crate::library::Library;
use crate::lyrics::LyricsClient;
use crate::models::{AccountProfile, ActiveTab, Lyrics, PlaybackInfo, Playlist, Track};
use crate::player::AudioPlayer;
use crate::settings::AppSettings;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Rounding, Stroke, Vec2};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;

const PLAYER_CLICK_HINT: &str = "player_click_hint";

#[derive(Clone, Copy)]
struct PlayerClickHint {
    anchor: Pos2,
    text: &'static str,
    expires_at: f64,
}

struct PlaylistPicker {
    track: Track,
    auth_user: usize,
    choices: Vec<PlaylistChoice>,
    pending: Option<tokio::sync::oneshot::Receiver<PlaylistPickerResult>>,
    message: Option<(bool, String)>,
}

enum PlaylistPickerResult {
    Choices(Result<Vec<PlaylistChoice>, String>),
    Edited(
        usize,
        PlaylistPickerAction,
        Result<PlaylistMembership, String>,
    ),
}

#[derive(Clone, Copy)]
enum PlaylistPickerAction {
    Add,
    AddAgain,
    Remove,
}

pub(crate) enum AppEvent {
    WaveSearchLoaded {
        request: u64,
        result: Result<Vec<Track>, String>,
    },
    WaveLoaded {
        request: u64,
        result: Result<Vec<Track>, String>,
        resume_after: Option<String>,
    },
    SearchResults(Result<Vec<Track>, String>),
    LikedMusicLoaded(Result<Vec<Track>, String>),
    PlaylistsLoaded(usize, Result<Vec<Playlist>, String>),
    PlaylistTracksLoaded {
        playlist_id: String,
        request: u64,
        auth_user: usize,
        result: Result<Vec<Track>, String>,
    },
    LyricsLoaded {
        track_id: String,
        req_id: u64,
        lyrics: Option<Lyrics>,
        is_upgraded: bool,
    },
    LocalLyricsFileSelected(std::path::PathBuf),
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
    pub(crate) wave_request: u64,
    pub(crate) wave_search_request: u64,
    pub(crate) wave_search_query: String,
    pub(crate) wave_search_busy: bool,
    pub(crate) wave_search_results: Vec<Track>,
    pub(crate) wave_search_message: String,
    pub(crate) wave_busy: bool,
    pub(crate) wave_message: String,
    pub(crate) innertube: Arc<InnertubeClient>,
    pub(crate) lyrics_client: Arc<LyricsClient>,
    pub(crate) player: AudioPlayer,
    pub(crate) discord: Arc<DiscordRpc>,
    pub(crate) downloader: Downloader,
    pub(crate) library: Library,
    pub(crate) settings: AppSettings,

    // UI State
    pub(crate) active_tab: ActiveTab,
    pub(crate) previous_tab: ActiveTab,
    pub(crate) repeat_mode: usize,
    pub(crate) shuffle_state: bool,
    pub(crate) welcome_step: Option<usize>,
    pub(crate) settings_menu_tab: usize,
    pub(crate) show_lyrics_in_player: bool,
    pub(crate) search_query: String,
    pub(crate) search_results: Vec<Track>,
    pub(crate) is_searching: bool,
    pub(crate) status_message: Option<String>,
    pub(crate) download_status: Option<String>,

    // Updater State
    pub(crate) update_info: Option<crate::updater::UpdateInfo>,
    pub(crate) is_checking_update: bool,
    pub(crate) is_applying_update: bool,
    pub(crate) update_status: Option<String>,
    pub(crate) show_update_confirm: bool,

    // Playlists state
    pub(crate) playlists: Vec<Playlist>,
    pub(crate) selected_playlist: Option<Playlist>,
    pub(crate) playlist_tracks: Vec<Track>,
    pub(crate) is_loading_playlists: bool,
    pub(crate) is_loading_playlist_tracks: bool,
    playlist_picker: Option<PlaylistPicker>,
    playlist_tracks_request: u64,
    playlists_account: Option<usize>,

    // Queue & Playback
    pub(crate) queue: Vec<Track>,
    pub(crate) current_index: Option<usize>,
    pub(crate) lyrics: Option<Lyrics>,
    pub(crate) lyrics_req_id: u64,
    pub(crate) is_local_lyrics: bool,

    // Channels
    pub(crate) event_tx: mpsc::Sender<AppEvent>,
    pub(crate) event_rx: mpsc::Receiver<AppEvent>,
    pub(crate) sync_status_message: Option<String>,
    pub(crate) last_auto_sync: std::time::Instant,
    pub(crate) volume_popup_open: bool,
    pub(crate) np_volume_popup_open: bool,
    pub(crate) pre_mute_volume: f64,
}

impl MetrolistApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::cover_loader::CoverLoader::install(&cc.egui_ctx);

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
        player.set_audio_processing(
            settings.audio_normalization,
            settings.skip_silence,
            settings.cover_ambient_light,
        );
        let discord = Arc::new(DiscordRpc::new());
        let library = Library::load();

        let default_tab = match settings.default_nav_tab {
            2 if settings.show_nav_liked => ActiveTab::Liked,
            3 if settings.show_nav_search => ActiveTab::Search,
            0 | 1 if settings.show_nav_playlists => ActiveTab::Playlists,
            _ => ActiveTab::Settings,
        };

        let welcome_step = (!settings.welcome_tour_seen).then_some(0);
        let mut app = Self {
            wave_request: 0,
            wave_search_request: 0,
            wave_search_query: String::new(),
            wave_search_busy: false,
            wave_search_results: Vec::new(),
            wave_search_message: String::new(),
            wave_busy: false,
            wave_message: String::new(),
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
            welcome_step,
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
            playlist_picker: None,
            playlist_tracks_request: 0,
            playlists_account: None,
            queue: Vec::new(),
            current_index: None,
            lyrics: None,
            lyrics_req_id: 0,
            is_local_lyrics: false,
            event_tx,
            event_rx,
            sync_status_message: None,
            last_auto_sync: std::time::Instant::now(),
            volume_popup_open: false,
            np_volume_popup_open: false,
            pre_mute_volume: 75.0,
        };

        app.apply_theme(&cc.egui_ctx);
        app.player.set_volume(app.settings.volume);

        // Восстановление последнего сеанса (очередь, трек, точная секунда, повтор/шаффл)
        if let Some(state) =
            crate::sync::SyncManager::load_state(&app.settings.google_drive_sync_folder)
        {
            let restored_queue: Vec<Track> = state
                .queue
                .iter()
                .map(|qe| Track {
                    id: qe.id.clone(),
                    title: qe.title.clone(),
                    artist: qe.artist.clone(),
                    duration_text: Self::format_time(qe.duration_secs),
                    thumbnail_url: qe.thumbnail.clone(),
                })
                .collect();

            if !restored_queue.is_empty() {
                app.queue = restored_queue;
                let idx = state.queue_index.min(app.queue.len() - 1);
                app.current_index = Some(idx);
                app.repeat_mode = match state.repeat_mode.as_str() {
                    "one" => 1,
                    "all" => 2,
                    _ => 0,
                };
                app.shuffle_state = state.shuffle_enabled;

                if let Some(track) = app.queue.get(idx).cloned() {
                    if let Ok(mut lock) = app.player.info.lock() {
                        lock.is_playing = false;
                        lock.position_secs = state.position_secs;
                        lock.duration_secs = state.duration_secs;
                    }
                    let track_target = if let Some(local_path) = app.find_local_track_file(&track) {
                        local_path.to_string_lossy().to_string()
                    } else {
                        format!("https://www.youtube.com/watch?v={}", track.id)
                    };
                    app.player.play_url(
                        &track_target,
                        app.track_fade_gain(state.position_secs, state.duration_secs),
                    );
                    app.player.toggle_pause(); // оставляем на паузе при старте
                    app.player.seek(state.position_secs); // перематываем на сохраненную секунду
                    app.load_track_lyrics(&track);
                }
            }
        }

        // Heartbeat thread: wakes up event loop periodically (every 500ms)
        let heartbeat_ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
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
                let _ = tx2.send(AppEvent::PlaylistsLoaded(auth_user, pls));
            });
        }

        app
    }

    pub(crate) fn apply_theme(&self, ctx: &egui::Context) {
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
        visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
        visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.7);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.rounding = Rounding::same(12.0);
        }
        ctx.style_mut(|style| {
            style.visuals = visuals;
            style.spacing.item_spacing = Vec2::new(8.0, 7.0);
            style.spacing.button_padding = Vec2::new(12.0, 6.0);
            style.animation_time = 0.16;
        });
    }

    pub(crate) fn accent_colors() -> [Color32; 5] {
        [
            Color32::from_rgb(16, 185, 129), // Emerald (Metrolist Default)
            Color32::from_rgb(244, 63, 94),  // Rose / Ruby
            Color32::from_rgb(168, 85, 247), // Violet / Amethyst
            Color32::from_rgb(14, 165, 233), // Sky / Cyberpunk
            Color32::from_rgb(245, 158, 11), // Amber Gold
        ]
    }

    pub(crate) fn accent_color(&self) -> Color32 {
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

    pub(crate) fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.queue.get(idx))
    }

    fn find_local_track_file(&self, track: &Track) -> Option<std::path::PathBuf> {
        let dir = std::path::PathBuf::from(&self.settings.download_dir);
        if !dir.exists() {
            return None;
        }

        let safe_title =
            Downloader::sanitize_filename(&format!("{} - {}", track.artist, track.title));
        for ext in &["mp3", "m4a", "opus", "flac", "webm", "wav"] {
            let p = dir.join(format!("{safe_title}.{ext}"));
            if p.exists() {
                return Some(p);
            }
        }

        let safe_title_only = Downloader::sanitize_filename(&track.title);
        for ext in &["mp3", "m4a", "opus", "flac", "webm", "wav"] {
            let p = dir.join(format!("{safe_title_only}.{ext}"));
            if p.exists() {
                return Some(p);
            }
        }

        None
    }

    fn track_cover(&self, ctx: &egui::Context, track: &Track) -> Option<egui::Image<'static>> {
        type Cover = Option<Arc<[u8]>>;
        if let Some(path) = self.find_local_track_file(track) {
            let sidecar = ["jpg", "jpeg", "png"]
                .into_iter()
                .map(|ext| path.with_extension(ext))
                .find(|candidate| candidate.is_file());
            let source = sidecar.as_ref().unwrap_or(&path);
            let modified = source.metadata().and_then(|meta| meta.modified()).ok();
            let key = egui::Id::new(("local_track_cover", source, modified));
            match ctx.data(|data| data.get_temp::<Cover>(key)) {
                Some(Some(bytes)) if !bytes.is_empty() => {
                    return Some(egui::Image::from_bytes(
                        format!("bytes://track-cover-{:?}.png", key),
                        bytes,
                    ));
                }
                None => {
                    ctx.data_mut(|data| data.insert_temp::<Cover>(key, None));
                    let ctx = ctx.clone();
                    tokio::task::spawn_blocking(move || {
                        let bytes = sidecar
                            .and_then(|file| image::open(file).ok())
                            .and_then(|cover| {
                                let mut png = std::io::Cursor::new(Vec::new());
                                cover
                                    .thumbnail(640, 640)
                                    .write_to(&mut png, image::ImageFormat::Png)
                                    .ok()?;
                                Some(png.into_inner())
                            })
                            .or_else(|| {
                                let output = std::process::Command::new("ffmpeg")
                                    .args(["-nostdin", "-v", "error", "-i"])
                                    .arg(path)
                                    .args([
                                        "-map",
                                        "0:v:disp:attached_pic",
                                        "-frames:v",
                                        "1",
                                        "-vf",
                                        "scale=640:640:force_original_aspect_ratio=decrease",
                                        "-c:v",
                                        "png",
                                        "-f",
                                        "image2pipe",
                                        "pipe:1",
                                    ])
                                    .output()
                                    .ok()?;
                                output.status.success().then_some(output.stdout)
                            })
                            .unwrap_or_default();
                        ctx.data_mut(|data| data.insert_temp::<Cover>(key, Some(bytes.into())));
                        ctx.request_repaint();
                    });
                }
                _ => {}
            }
        }
        track.thumbnail_url.clone().map(egui::Image::new)
    }

    fn paint_square_cover(
        ui: &mut egui::Ui,
        cover: egui::Image<'_>,
        side: f32,
        rounding: Rounding,
    ) -> egui::Response {
        let size = Vec2::splat(side);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let mut cover = cover.rounding(rounding);
        if let Some(source) = cover
            .load_for_size(ui.ctx(), size)
            .ok()
            .and_then(|t| t.size())
        {
            if source.x > 0.0 && source.y > 0.0 {
                let short_side = source.x.min(source.y);
                cover = cover.uv(Rect::from_center_size(
                    Pos2::new(0.5, 0.5),
                    Vec2::new(short_side / source.x, short_side / source.y),
                ));
            }
            cover.paint_at(ui, rect);
        } else {
            ui.painter()
                .rect_filled(rect, rounding, Color32::from_rgb(38, 38, 48));
            Self::draw_vector_logo(
                ui.painter(),
                rect.shrink(rect.width() * 0.29),
                Color32::from_rgb(112, 116, 130),
            );
        }
        response
    }

    fn cover_ambient_mesh(
        &self,
        ctx: &egui::Context,
        cover: &egui::Image<'_>,
        playing: bool,
        artwork: Rect,
    ) -> Option<egui::Mesh> {
        const ZONES: usize = 5;
        let uri = cover.uri()?;
        let key = egui::Id::new(("ambient_palette", uri));
        let palette = ctx.data(|data| data.get_temp::<[Color32; ZONES * ZONES]>(key));
        let colors = if let Some(palette) = palette {
            palette
        } else {
            let Ok(egui::load::ImagePoll::Ready { image }) =
                ctx.try_load_image(uri, egui::load::SizeHint::default())
            else {
                return None;
            };
            let [w, h] = image.size;
            if w == 0 || h == 0 {
                return None;
            }
            let side = w.min(h);
            let colors = std::array::from_fn(|region| {
                let mut rgb = [0.0; 3];
                let mut total = 0.0;
                for y in 0..10 {
                    for x in 0..10 {
                        let px = (w - side) / 2 + (region % ZONES * 10 + x) * side / (ZONES * 10);
                        let py = (h - side) / 2 + (region / ZONES * 10 + y) * side / (ZONES * 10);
                        let [r, g, b, a] = image.pixels[py * w + px].to_srgba_unmultiplied();
                        let weight =
                            (24.0 + f32::from(r.max(g).max(b) - r.min(g).min(b))) * f32::from(a);
                        for (sum, channel) in rgb.iter_mut().zip([r, g, b]) {
                            *sum += f32::from(channel) * weight;
                        }
                        total += weight;
                    }
                }
                let [r, g, b] = rgb.map(|channel| (channel / total.max(1.0)) as u8);
                Color32::from_rgb(r, g, b)
            });
            ctx.data_mut(|data| data.insert_temp(key, colors));
            colors
        };
        let strength = f32::from(self.settings.ambient_pulse_strength.min(100)) / 100.0;
        let pulse = if playing && strength > 0.0 {
            self.player.poll_audio_pulse() * strength
        } else {
            0.0
        };
        let blur = f32::from(self.settings.ambient_blur.min(100)) / 100.0;
        let radius =
            (artwork.width() * (0.12 + blur * 0.36)).clamp(12.0, 210.0) * (1.0 + pulse * 0.25);
        let rect = artwork.expand(radius * 4.4);
        let colors = colors.map(|color| [color.r(), color.g(), color.b()].map(f32::from));
        let grid = 33 + 96 * u32::from(self.settings.ambient_density.min(100)) / 100;
        // A Gaussian separates into horizontal and vertical weights.
        let axis_weights = |start: f32, span: f32, art_start: f32, art_span: f32| {
            (0..grid)
                .map(|i| {
                    let pos = start + span * i as f32 / (grid - 1) as f32;
                    let weights: [(f32, f32); ZONES] = std::array::from_fn(|region| {
                        let center = art_start + art_span * (region as f32 + 0.5) / ZONES as f32;
                        let d = (pos - center) / radius;
                        let core = (-0.5 * d * d).exp();
                        let tail = (-0.5 * (d / 1.65).powi(2)).exp();
                        (core, tail)
                    });
                    (pos, weights)
                })
                .collect::<Vec<_>>()
        };
        let columns = axis_weights(rect.left(), rect.width(), artwork.left(), artwork.width());
        let rows = axis_weights(rect.top(), rect.height(), artwork.top(), artwork.height());
        let mut mesh = egui::Mesh::default();
        for (y, &(py, vertical)) in rows.iter().enumerate() {
            for (x, &(px, horizontal)) in columns.iter().enumerate() {
                let pos = Pos2::new(px, py);
                let mut rgb = [0.0; 3];
                let mut energy = 0.0;
                for (i, color) in colors.iter().enumerate() {
                    let (core_x, tail_x) = horizontal[i % ZONES];
                    let (core_y, tail_y) = vertical[i / ZONES];
                    let weight = 0.84 * core_x * core_y + 0.16 * tail_x * tail_y;
                    for (sum, channel) in rgb.iter_mut().zip(color) {
                        *sum += channel * weight;
                    }
                    energy += weight;
                }
                let (x, y) = (x as u32, y as u32);
                let [r, g, b] = rgb.map(|sum| (sum / energy.max(f32::EPSILON)) as u8);
                let alpha = (1.0 - (-energy * 0.21).exp()) * (115.0 + pulse * 150.0);
                let alpha = if x == 0 || y == 0 || x == grid - 1 || y == grid - 1 {
                    0
                } else {
                    alpha as u8
                };
                mesh.colored_vertex(pos, Color32::from_rgba_unmultiplied(r, g, b, alpha));
                if x > 0 && y > 0 {
                    let i = y * grid + x;
                    mesh.add_triangle(i - grid - 1, i - grid, i);
                    mesh.add_triangle(i - grid - 1, i, i - 1);
                }
            }
        }
        Some(mesh)
    }

    fn track_fade_gain(&self, position: f64, duration: f64) -> f64 {
        let ramp = |time: f64, seconds: f64| {
            if seconds > 0.0 {
                (time / seconds).clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        let fade_in = ramp(position, self.settings.fade_in_seconds);
        let fade_out = if duration > 0.0 {
            ramp(duration - position, self.settings.fade_out_seconds)
        } else {
            1.0
        };
        fade_in.min(fade_out)
    }

    fn load_track_lyrics(&mut self, track: &Track) {
        // 1. Проверяем наличие локального файла .lrc или .ttml в папке загрузок
        let mut lyrics_loaded = false;
        let dir = std::path::PathBuf::from(&self.settings.download_dir);
        let safe_title =
            Downloader::sanitize_filename(&format!("{} - {}", track.artist, track.title));
        let safe_title_only = Downloader::sanitize_filename(&track.title);

        for name in &[&safe_title, &safe_title_only] {
            for ext in &["lrc", "ttml"] {
                let p = dir.join(format!("{}.{}", name, ext));
                if p.exists() {
                    if let Some(loaded) = crate::lyrics::LocalFileAdapter::load_file(&p) {
                        self.lyrics_req_id = self.lyrics_req_id.wrapping_add(1);
                        self.lyrics = Some(loaded);
                        self.is_local_lyrics = true;
                        lyrics_loaded = true;
                        break;
                    }
                }
            }
            if lyrics_loaded {
                break;
            }
        }

        // 2. Если локальный файл не найден, запускаем многоисточниковый поиск
        if !lyrics_loaded {
            self.is_local_lyrics = false;
            self.start_lyrics_search_for_track(track, false);
        }
    }

    pub(crate) fn play_track(&mut self, track: Track) {
        // Мгновенное воспроизведение локального файла, если трек уже скачан (задержка 0.0с вместо 3-5с)
        if let Some(local_path) = self.find_local_track_file(&track) {
            let path_str = local_path.to_string_lossy().to_string();
            self.player
                .play_url(&path_str, self.track_fade_gain(0.0, 0.0));
        } else {
            let url = format!("https://www.youtube.com/watch?v={}", track.id);
            self.player.play_url(&url, self.track_fade_gain(0.0, 0.0));
        }

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

        self.load_track_lyrics(&track);

        // Add to queue if not present
        if let Some(pos) = self.queue.iter().position(|t| t.id == track.id) {
            self.current_index = Some(pos);
        } else {
            self.queue.push(track);
            self.current_index = Some(self.queue.len() - 1);
        }
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

    fn next_track_index(&self) -> Option<usize> {
        if self.queue.is_empty() {
            return None;
        }
        match self.current_index {
            Some(i) if i + 1 < self.queue.len() => Some(i + 1),
            Some(_) if self.repeat_mode != 1 => None,
            _ => Some(0),
        }
    }

    fn play_next(&mut self) {
        let Some(next_idx) = self.next_track_index() else {
            return;
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

    pub(crate) fn sync_playlists(&mut self) {
        if self.settings.google_cookie.is_empty() {
            self.status_message =
                Some("Войдите в аккаунт для синхронизации плейлистов".to_string());
            return;
        }
        let auth_user = self.settings.auth_user_index;
        if self.playlists_account != Some(auth_user) {
            self.playlists.clear();
            self.selected_playlist = None;
            self.playlist_tracks.clear();
            self.is_loading_playlist_tracks = false;
        }
        self.is_loading_playlists = true;
        self.status_message = Some("Загрузка плейлистов из YouTube Music...".to_string());
        let client = Arc::clone(&self.innertube);
        let cookie = self.settings.google_cookie.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch_user_playlists(&cookie, auth_user).await;
            let _ = tx.send(AppEvent::PlaylistsLoaded(auth_user, res));
        });
    }

    fn open_playlist_picker(&mut self, ctx: &egui::Context, track: Track) {
        let auth_user = self.settings.auth_user_index;
        let mut picker = PlaylistPicker {
            track,
            auth_user,
            choices: Vec::new(),
            pending: None,
            message: None,
        };
        if self.settings.google_cookie.is_empty() {
            picker.message = Some((true, "Войди в аккаунт YouTube Music в настройках.".into()));
        } else {
            let (tx, rx) = tokio::sync::oneshot::channel();
            picker.pending = Some(rx);
            let ctx = ctx.clone();
            let client = Arc::clone(&self.innertube);
            let track_id = picker.track.id.clone();
            let cookie = self.settings.google_cookie.clone();
            tokio::spawn(async move {
                let result = client.playlist_choices(&track_id, &cookie, auth_user).await;
                let _ = tx.send(PlaylistPickerResult::Choices(result));
                ctx.request_repaint();
            });
        }
        self.playlist_picker = Some(picker);
    }

    fn render_playlist_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.playlist_picker.take() else {
            return;
        };
        if let Some(pending) = &mut picker.pending {
            use tokio::sync::oneshot::error::TryRecvError;
            match pending.try_recv() {
                Ok(result) => {
                    picker.pending = None;
                    match result {
                        PlaylistPickerResult::Choices(Ok(mut choices)) => {
                            if self.playlists_account == Some(picker.auth_user) {
                                for choice in &mut choices {
                                    let playlist = &mut choice.playlist;
                                    if let Some(library) = self.playlists.iter().find(|p| {
                                        p.id.strip_prefix("VL").unwrap_or(&p.id) == playlist.id
                                    }) {
                                        if library.thumbnail_url.is_some() {
                                            playlist
                                                .thumbnail_url
                                                .clone_from(&library.thumbnail_url);
                                        }
                                        if !library.subtitle.is_empty() {
                                            playlist.subtitle.clone_from(&library.subtitle);
                                        }
                                    }
                                }
                            }
                            picker.choices = choices;
                        }
                        PlaylistPickerResult::Choices(Err(error))
                        | PlaylistPickerResult::Edited(_, _, Err(error)) => {
                            picker.message = Some((true, error))
                        }
                        PlaylistPickerResult::Edited(index, action, Ok(membership)) => {
                            let choice = &mut picker.choices[index];
                            choice.membership = membership;
                            let message = match action {
                                PlaylistPickerAction::Add => "Добавлено в YouTube Music",
                                PlaylistPickerAction::AddAgain => "Добавлена ещё одна копия",
                                PlaylistPickerAction::Remove => "Удалена одна запись трека",
                            };
                            picker.message =
                                Some((false, format!("{message} · «{}»", choice.playlist.title)));
                            if self.settings.auth_user_index == picker.auth_user {
                                if let Some(playlist) = self.selected_playlist.clone().filter(|p| {
                                    p.id.strip_prefix("VL").unwrap_or(&p.id) == choice.playlist.id
                                }) {
                                    self.refresh_playlist_tracks(&playlist.id);
                                }
                                self.sync_playlists();
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Closed) => {
                    picker.pending = None;
                    picker.message = Some((true, "Запрос не завершился. Попробуй ещё раз.".into()));
                }
            }
        }

        let saving = picker.pending.is_some() && !picker.choices.is_empty();
        let account_changed = self.settings.auth_user_index != picker.auth_user;
        let mut edit = None;
        let mut close = false;
        let accent = self.accent_color();
        let rounding = Rounding::same(self.settings.album_art_rounding);
        let modal = egui::Modal::new(egui::Id::new("add_track_to_playlist")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 48.0).min(530.0));
            ui.heading(RichText::new("Добавить в плейлист").color(Color32::WHITE));
            ui.label(format!("{} — {}", picker.track.artist, picker.track.title));
            ui.add_space(8.0);
            if account_changed {
                ui.label("Активный аккаунт изменился. Открой список плейлистов заново.");
            } else if picker.choices.is_empty()
                && picker.pending.is_none()
                && picker.message.is_none()
            {
                ui.label("Нет доступных плейлистов. Создай плейлист в YouTube Music.");
            } else if !picker.choices.is_empty()
                && picker
                    .choices
                    .iter()
                    .all(|choice| self.settings.youtube_playlist_hidden(&choice.playlist.id))
            {
                ui.label(
                    "Все доступные плейлисты скрыты. Их можно вернуть в настройках библиотеки.",
                );
            }
            if picker.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if saving {
                        "Сохраняю изменения…"
                    } else {
                        "Загружаю плейлисты…"
                    });
                });
            }
            egui::ScrollArea::vertical()
                .max_height((ctx.screen_rect().height() - 185.0).clamp(155.0, 360.0))
                .show(ui, |ui| {
                    for (index, choice) in picker.choices.iter().enumerate() {
                        if self.settings.youtube_playlist_hidden(&choice.playlist.id) {
                            continue;
                        }
                        let enabled = picker.pending.is_none() && !account_changed;
                        if let Some(action) =
                            Self::playlist_card(ui, &choice.playlist, rounding, accent, |ui| {
                                Self::playlist_picker_actions(
                                    ui,
                                    &choice.membership,
                                    enabled,
                                    accent,
                                )
                            })
                        {
                            edit = Some((index, action));
                        }
                        ui.add_space(6.0);
                    }
                });
            if let Some((error, message)) = &picker.message {
                let color = if *error {
                    ui.visuals().error_fg_color
                } else {
                    self.accent_color()
                };
                ui.colored_label(color, message);
            }
            ui.add_space(8.0);
            close = ui
                .add_enabled(!saving, egui::Button::new("Закрыть"))
                .clicked();
        });
        if !saving && (close || modal.should_close()) {
            return;
        }
        if let Some((index, action)) = edit {
            let (tx, rx) = tokio::sync::oneshot::channel();
            picker.pending = Some(rx);
            picker.message = None;
            let ctx = ctx.clone();
            let client = Arc::clone(&self.innertube);
            let playlist_id = picker.choices[index].playlist.id.clone();
            let removal_entry_id = match &picker.choices[index].membership {
                PlaylistMembership::Present(id) => id.clone(),
                _ => None,
            };
            let track_id = picker.track.id.clone();
            let cookie = self.settings.google_cookie.clone();
            let auth_user = picker.auth_user;
            tokio::spawn(async move {
                let edited = match action {
                    PlaylistPickerAction::Remove => {
                        client
                            .remove_track_from_playlist(
                                &playlist_id,
                                &track_id,
                                removal_entry_id,
                                &cookie,
                                auth_user,
                            )
                            .await
                    }
                    PlaylistPickerAction::Add | PlaylistPickerAction::AddAgain => {
                        client
                            .add_track_to_playlist(
                                &playlist_id,
                                &track_id,
                                matches!(action, PlaylistPickerAction::AddAgain),
                                &cookie,
                                auth_user,
                            )
                            .await
                    }
                };
                let result = match edited {
                    Ok(()) if matches!(action, PlaylistPickerAction::Remove) => Ok(client
                        .check_playlist_membership(&playlist_id, &track_id, &cookie, auth_user)
                        .await),
                    Ok(()) => Ok(PlaylistMembership::Present(None)),
                    Err(error) => Err(error),
                };
                let _ = tx.send(PlaylistPickerResult::Edited(index, action, result));
                ctx.request_repaint();
            });
        }
        self.playlist_picker = Some(picker);
    }

    fn open_playlist(&mut self, pl: Playlist) {
        self.playlist_tracks.clear();
        self.refresh_playlist_tracks(&pl.id);
        self.selected_playlist = Some(pl);
    }

    fn refresh_playlist_tracks(&mut self, playlist_id: &str) {
        self.is_loading_playlist_tracks = true;
        self.playlist_tracks_request = self.playlist_tracks_request.wrapping_add(1);
        let request = self.playlist_tracks_request;
        let client = Arc::clone(&self.innertube);
        let cookie = self.settings.google_cookie.clone();
        let auth_user = self.settings.auth_user_index;
        let pid = playlist_id.to_owned();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let res = client.fetch_playlist_tracks(&pid, &cookie, auth_user).await;
            let _ = tx.send(AppEvent::PlaylistTracksLoaded {
                playlist_id: pid,
                request,
                auth_user,
                result: res,
            });
        });
    }

    fn download_track(&mut self, track: Track) {
        // Проверка: трек уже есть в списке скачанных
        if self
            .library
            .downloaded_tracks
            .iter()
            .any(|t| t.id == track.id)
        {
            self.status_message = Some(format!("Трек «{}» уже скачан!", track.title));
            return;
        }

        let opts = crate::downloader::DownloadOptions::from_settings(&self.settings);

        // Проверка: файл трека уже существует на диске в папке загрузок
        if self.find_local_track_file(&track).is_some() {
            if !self
                .library
                .downloaded_tracks
                .iter()
                .any(|t| t.id == track.id)
            {
                self.library.add_downloaded(track.clone());
            }
            self.status_message = Some(format!("Трек «{}» уже скачан на диск!", track.title));
            return;
        }

        self.download_status = Some(format!("Скачивание: {}", track.title));
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

    fn toggle_playback(&mut self) {
        self.player.toggle_pause();
        self.last_auto_sync = std::time::Instant::now();
        if let Ok(info) = self.player.info.lock().map(|info| info.clone()) {
            self.trigger_auto_sync(&info);
        }
    }

    pub(crate) fn trigger_auto_sync(&self, playback_info: &PlaybackInfo) {
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
        let drive_enabled = self.settings.cross_device_sync_enabled;

        std::thread::spawn(move || {
            let res = crate::sync::SyncManager::save_state_with_queue(
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
            if drive_enabled {
                if let Ok(state) = res {
                    let _ = state;
                }
            }
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

        let (hit_rect, resp) = ui.allocate_exact_size(
            Vec2::new(width, height.max(18.0)),
            egui::Sense::click_and_drag(),
        );
        // Leave room for the thumb and its stroke at both ends of the track.
        let inset = (hit_rect.width() * 0.25).min(9.0);
        let rect = hit_rect.shrink2(Vec2::new(inset, 0.0));

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

        let rounded_track = |left, right, thickness: f32, color| {
            let radius = thickness / 2.0;
            let track = Rect::from_min_max(
                Pos2::new(left, center_y - radius),
                Pos2::new(right, center_y + radius),
            );
            painter.rect_filled(track, radius.min(track.width() / 2.0), color);
        };

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
                    rounded_track(played_x, rect.right(), 3.5, unplayed_color);
                }

                // Draw animated sine wave for played portion
                if played_x > rect.left() {
                    let step = 2.0_f32;
                    let start_x = (rect.left() + 1.75).min(played_x);
                    let mut prev_pt = Pos2::new(start_x, center_y);
                    painter.circle_filled(prev_pt, 1.75, accent);

                    let mut x = start_x + step;
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
                rounded_track(rect.left(), rect.right(), 2.0, unplayed_color);
                if played_x > rect.left() {
                    rounded_track(rect.left(), played_x, 2.5, accent);
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
                rounded_track(rect.left(), rect.right(), 4.0, unplayed_color);
                if played_x > rect.left() {
                    rounded_track(rect.left(), played_x, 4.0, accent);
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

    fn player_button_hint(
        response: egui::Response,
        text: &'static str,
        show_on_click: bool,
    ) -> egui::Response {
        let ctx = &response.ctx;
        let id = egui::Id::new(PLAYER_CLICK_HINT);
        let now = ctx.input(|input| input.time);
        let anchor = response.rect.center_top();
        if show_on_click && response.clicked() {
            ctx.data_mut(|data| {
                data.insert_temp(
                    id,
                    PlayerClickHint {
                        anchor,
                        text,
                        expires_at: now + 0.8,
                    },
                );
            });
        } else if (response.hovered() || (show_on_click && response.is_pointer_button_down_on()))
            && !ctx
                .data(|data| data.get_temp::<PlayerClickHint>(id))
                .is_some_and(|hint| hint.expires_at > now)
        {
            Self::paint_player_hint(ctx, anchor, text);
        }
        response
    }

    fn paint_player_hint(ctx: &egui::Context, anchor: Pos2, text: &'static str) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("player_button_hint"),
        ));
        let style = ctx.style();
        let galley = painter.layout_no_wrap(
            text.into(),
            egui::TextStyle::Body.resolve(&style),
            style.visuals.text_color(),
        );
        let padding = Vec2::new(8.0, 4.0);
        let size = galley.size() + padding * 2.0;
        let screen = ctx.screen_rect().shrink(4.0);
        let x = (anchor.x - size.x * 0.5)
            .min(screen.right() - size.x)
            .max(screen.left());
        let y = (anchor.y - size.y - 6.0)
            .min(screen.bottom() - size.y)
            .max(screen.top());
        let rect = Rect::from_min_size(Pos2::new(x, y), size);
        painter.rect(
            rect,
            Rounding::same(6.0),
            style.visuals.window_fill(),
            style.visuals.window_stroke(),
        );
        painter.galley(rect.min + padding, galley, style.visuals.text_color());
    }

    fn heart_button(ui: &mut egui::Ui, track_id: &str, liked: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
        let key = egui::Id::new(("heart_animation", track_id));
        let now = ui.input(|input| input.time);
        let clicked = response.clicked();
        ui.ctx().data_mut(|data| {
            data.insert_temp(response.id.with("no_button_feedback"), true);
            if clicked {
                data.insert_temp(key, now);
            }
        });
        let liked = liked ^ clicked;
        let start = ui.ctx().data(|data| data.get_temp::<f64>(key));
        let progress = start
            .map(|start| ((now - start) / 0.40).clamp(0.0, 1.0) as f32)
            .unwrap_or(1.0);
        if progress < 1.0 {
            ui.ctx().request_repaint();
        }
        let scale = if liked {
            1.0 + 0.32 * (progress * std::f32::consts::PI).sin()
        } else {
            1.0
        };
        let center = rect.center() - Vec2::new(0.0, 1.0);
        let shape = |direction: f32| {
            let point = |x, y| center + Vec2::new(direction * x, y) * scale;
            let curves = [
                [
                    point(0.0, -4.0),
                    point(2.8, -9.0),
                    point(9.3, -7.8),
                    point(8.6, -2.5),
                ],
                [
                    point(8.6, -2.5),
                    point(8.2, 0.8),
                    point(4.5, 3.8),
                    point(1.8, 6.3),
                ],
                [
                    point(1.8, 6.3),
                    point(1.2, 6.9),
                    point(0.6, 7.1),
                    point(0.0, 7.1),
                ],
            ];
            let mut points = vec![curves[0][0]];
            for curve in curves {
                let path = egui::epaint::CubicBezierShape::from_points_stroke(
                    curve,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::NONE,
                );
                points.extend((1..=12).map(|i| path.sample(i as f32 / 12.0)));
            }
            points
        };
        let red = Color32::from_rgb(244, 63, 94);
        let painter = ui.painter();
        if liked || progress < 1.0 {
            for direction in [-1.0, 1.0] {
                let mut points = shape(direction);
                let mut fill = red;
                let mut offset = Vec2::ZERO;
                if !liked {
                    points.extend([center + Vec2::new(-1.4, 3.0), center + Vec2::new(1.5, 0.0)]);
                    offset = Vec2::new(direction * (1.0 + 4.0 * progress), progress * 3.0);
                    fill = red
                        .lerp_to_gamma(Color32::BLACK, progress)
                        .gamma_multiply(1.0 - progress.powi(4));
                }
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(
                    center + offset + Vec2::new(direction * 3.0, -1.0) * scale,
                    fill,
                );
                for point in &points {
                    mesh.colored_vertex(*point + offset, fill);
                }
                for i in 0..points.len() as u32 {
                    mesh.add_triangle(0, i + 1, (i + 1) % points.len() as u32 + 1);
                }
                painter.add(egui::Shape::mesh(mesh));
            }
        } else {
            let color = if response.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(160, 160, 175)
            };
            for direction in [-1.0, 1.0] {
                painter.add(egui::Shape::line(
                    shape(direction),
                    Stroke::new(1.5_f32, color),
                ));
            }
        }
        response
    }

    fn heart_hint(liked: bool) -> &'static str {
        if liked {
            "Удалить из любимых"
        } else {
            "Добавить в любимые"
        }
    }

    fn add_to_playlist_button(ui: &mut egui::Ui, size: f32, accent: Color32) -> egui::Response {
        let (rect, button) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
        button.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                "Добавить в плейлист",
            )
        });
        let color = if button.hovered() {
            accent
        } else {
            Color32::from_rgb(200, 200, 215)
        };
        let center = rect.center();
        let painter = ui.painter();
        let stroke = Stroke::new(1.7_f32, color);
        for (y, width) in [(-5.0, 13.0), (0.0, 8.0), (5.0, 6.0)] {
            painter.line_segment(
                [
                    center + Vec2::new(-9.0, y),
                    center + Vec2::new(-9.0 + width, y),
                ],
                stroke,
            );
        }
        painter.line_segment(
            [center + Vec2::new(1.0, 4.0), center + Vec2::new(9.0, 4.0)],
            stroke,
        );
        painter.line_segment(
            [center + Vec2::new(5.0, 0.0), center + Vec2::new(5.0, 8.0)],
            stroke,
        );
        Self::player_button_hint(button, "Добавить в плейлист", false)
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

    /// Renders vector dual beamed musical note icon matching `assets/icon.png`
    fn draw_playback_icon(painter: &egui::Painter, center: Pos2, playing: bool) {
        if playing {
            for offset in [-3.9, 3.9] {
                let bar =
                    Rect::from_center_size(center + Vec2::new(offset, 0.0), Vec2::new(3.6, 13.0));
                painter.rect_filled(bar, 1.0, Color32::WHITE);
            }
        } else {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + Vec2::new(-4.5, -7.0),
                    center + Vec2::new(6.5, 0.0),
                    center + Vec2::new(-4.5, 7.0),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
    }

    fn draw_vector_logo(painter: &egui::Painter, rect: Rect, color: Color32) {
        let center = rect.center();
        let w = rect.width();
        let h = rect.height();

        // Left note head
        let n1_center = Pos2::new(center.x - w * 0.22, center.y + h * 0.20);
        let n1_r = w * 0.16;
        painter.circle_filled(n1_center, n1_r, color);

        let n2_center = Pos2::new(center.x + w * 0.32, center.y + h * 0.06);
        let n2_r = w * 0.18;
        painter.add(egui::Shape::convex_polygon(
            vec![
                n2_center + Vec2::new(-n2_r, -n2_r),
                n2_center + Vec2::new(n2_r, 0.0),
                n2_center + Vec2::new(-n2_r, n2_r),
            ],
            color,
            Stroke::NONE,
        ));

        let stem_w = (w * 0.12).max(2.0);
        let stem_stroke = Stroke::new(stem_w, color);

        // Left stem
        let s1_bottom = Pos2::new(n1_center.x + n1_r * 0.72, n1_center.y);
        let s1_top = Pos2::new(s1_bottom.x, center.y - h * 0.26);
        painter.line_segment([s1_bottom, s1_top], stem_stroke);

        // Right stem
        let s2_bottom = Pos2::new(n2_center.x, n2_center.y);
        let s2_top = Pos2::new(s2_bottom.x, center.y - h * 0.40);
        painter.line_segment([s2_bottom, s2_top], stem_stroke);

        // Top angled connecting beam
        let beam_stroke = Stroke::new((w * 0.16).max(2.8), color);
        let b1 = Pos2::new(s1_top.x - stem_w * 0.4, s1_top.y);
        let b2 = Pos2::new(s2_top.x + stem_w * 0.4, s2_top.y);
        painter.line_segment([b1, b2], beam_stroke);
    }

    /// Dynamic vector speaker icon reflecting volume levels and mute state
    fn draw_vector_speaker(
        painter: &egui::Painter,
        rect: Rect,
        volume: f64,
        is_muted: bool,
        color: Color32,
    ) {
        let center = rect.center();
        let w = rect.width();
        let h = rect.height();

        // 1. Speaker body (softly rounded)
        let body_w = w * 0.22;
        let body_h = h * 0.28;
        let body_rect = Rect::from_center_size(
            Pos2::new(center.x - w * 0.26, center.y),
            Vec2::new(body_w, body_h),
        );
        painter.rect_filled(body_rect, Rounding::same(3.0), color);

        // 2. Speaker cone (smooth flared polygon)
        let cone_points = vec![
            Pos2::new(body_rect.right() - 1.0, body_rect.top()),
            Pos2::new(center.x + w * 0.04, center.y - h * 0.34),
            Pos2::new(center.x + w * 0.04, center.y + h * 0.34),
            Pos2::new(body_rect.right() - 1.0, body_rect.bottom()),
        ];
        painter.add(egui::Shape::convex_polygon(
            cone_points,
            color,
            Stroke::NONE,
        ));

        let effective_vol = if is_muted { 0.0 } else { volume };

        // Helper: draw a smooth rounded circular arc wave
        let draw_arc =
            |painter: &egui::Painter, cx_arc: f32, half_h: f32, bulge: f32, stroke: Stroke| {
                let n = 14;
                let mut pts = Vec::with_capacity(n + 1);
                for i in 0..=n {
                    let t = i as f32 / n as f32;
                    let angle = std::f32::consts::PI * (t - 0.5); // -PI/2 to PI/2
                    let x = cx_arc + bulge * angle.cos();
                    let y = center.y + half_h * angle.sin();
                    pts.push(Pos2::new(x, y));
                }
                painter.add(egui::Shape::line(pts, stroke));
            };

        if effective_vol <= 0.5 {
            // Muted: draw an 'X'
            let x_center = Pos2::new(center.x + w * 0.26, center.y);
            let x_sz = w * 0.12;
            let x_stroke = Stroke::new(1.8_f32, Color32::from_rgb(244, 63, 94));
            painter.line_segment(
                [
                    Pos2::new(x_center.x - x_sz, x_center.y - x_sz),
                    Pos2::new(x_center.x + x_sz, x_center.y + x_sz),
                ],
                x_stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(x_center.x - x_sz, x_center.y + x_sz),
                    Pos2::new(x_center.x + x_sz, x_center.y - x_sz),
                ],
                x_stroke,
            );
        } else {
            let wave_stroke = Stroke::new(1.6_f32, color);
            // 1st arc (low volume: > 0%)
            draw_arc(
                painter,
                center.x + w * 0.14,
                h * 0.17,
                w * 0.09,
                wave_stroke,
            );

            // 2nd arc (mid: > 33%)
            if effective_vol > 33.0 {
                draw_arc(
                    painter,
                    center.x + w * 0.24,
                    h * 0.27,
                    w * 0.12,
                    wave_stroke,
                );
            }

            // 3rd arc (high: > 66%)
            if effective_vol > 66.0 {
                draw_arc(
                    painter,
                    center.x + w * 0.34,
                    h * 0.37,
                    w * 0.15,
                    wave_stroke,
                );
            }
        }
    }

    /// Pure vector heart badge (liked = filled ruby, unliked = subtle outline)
    #[allow(dead_code)]
    fn draw_vector_heart(painter: &egui::Painter, rect: Rect, is_liked: bool, color: Color32) {
        let cx = rect.center().x;
        let cy = rect.center().y;
        let s = (rect.width().min(rect.height()) * 0.40) / 16.0;

        // Mathematical cardioid/heart parametric curve (32 points)
        // x(t) = 16 * sin^3(t)
        // y(t) = -(13*cos(t) - 5*cos(2t) - 2*cos(3t) - cos(4t)) - 2.6
        let n = 32;
        let mut pts: Vec<Pos2> = Vec::with_capacity(n);
        for i in 0..n {
            let t = std::f32::consts::TAU * (i as f32) / (n as f32);
            let sin_t = t.sin();
            let x = 16.0 * sin_t * sin_t * sin_t;
            let y =
                -(13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos())
                    - 2.6;
            pts.push(Pos2::new(cx + x * s, cy + y * s));
        }

        if is_liked {
            // Heart is star-convex from center (cx, cy). Draw triangle fan:
            let center = Pos2::new(cx, cy);
            for i in 0..n {
                let next = (i + 1) % n;
                let tri = vec![center, pts[i], pts[next]];
                painter.add(egui::Shape::convex_polygon(tri, color, Stroke::NONE));
            }
        } else {
            let stroke = Stroke::new(1.6_f32, color);
            let mut closed_pts = pts.clone();
            if let Some(&first) = closed_pts.first() {
                closed_pts.push(first);
            }
            painter.add(egui::Shape::line(closed_pts, stroke));
        }
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
        if playback_info.is_playing {
            self.player.set_fade_gain(
                self.track_fade_gain(playback_info.position_secs, playback_info.duration_secs),
            );
        }
        // Repaint request: 30ms when focused, 250ms when inactive/unfocused or minimized
        // (prevents compositor stalls and Wayland "Application not responding" dialogs on other workspaces)
        let is_inactive = ctx.input(|i| !i.focused || i.viewport().minimized == Some(true));
        if is_inactive {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }

        // Periodic auto-sync to local state and Google Drive (every ~3 seconds during playback)
        if playback_info.is_playing
            && self.last_auto_sync.elapsed() >= std::time::Duration::from_secs(3)
        {
            self.last_auto_sync = std::time::Instant::now();
            self.trigger_auto_sync(&playback_info);
        }

        // Poll events
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::WaveSearchLoaded { request, result } => {
                    if request != self.wave_search_request {
                        continue;
                    }
                    self.wave_search_busy = false;
                    match result {
                        Ok(tracks) => {
                            self.wave_search_message = if tracks.is_empty() {
                                "Ничего не найдено. Попробуй название и исполнителя.".into()
                            } else {
                                String::new()
                            };
                            self.wave_search_results = tracks;
                        }
                        Err(error) => self.wave_search_message = error,
                    }
                }
                AppEvent::WaveLoaded {
                    request,
                    result,
                    resume_after,
                } => {
                    if request != self.wave_request {
                        continue;
                    }
                    self.wave_busy = false;
                    match result {
                        Ok(tracks) => {
                            let first_id = tracks.first().map(|track| track.id.clone());
                            let start = self.queue.len();
                            for track in tracks {
                                if !self.queue.iter().any(|item| item.id == track.id) {
                                    self.queue.push(track);
                                }
                            }
                            let count = self.queue.len() - start;
                            self.wave_message = if count == 0 {
                                "Новых треков не найдено. Попробуй другое слово или основу волны."
                                    .into()
                            } else {
                                format!("Добавлено в очередь: {count}")
                            };
                            let resume = resume_after.is_some()
                                && self.settings.wave_auto
                                && self.current_track().map(|t| &t.id) == resume_after.as_ref()
                                && self.current_index.is_some_and(|i| i + 1 == start);
                            let next = if resume_after.is_none() {
                                first_id.and_then(|id| {
                                    self.queue.iter().position(|track| track.id == id)
                                })
                            } else if resume && count > 0 {
                                Some(start)
                            } else {
                                None
                            };
                            if let Some(index) = next {
                                self.current_index = Some(index);
                                self.play_track(self.queue[index].clone());
                                self.wave_message =
                                    format!("Волна играет · новых треков в очереди: {count}");
                            }
                        }
                        Err(error) => {
                            self.wave_message = format!("Не удалось собрать волну: {error}")
                        }
                    }
                }

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
                AppEvent::PlaylistsLoaded(auth_user, res) => {
                    if self.settings.auth_user_index != auth_user {
                        continue;
                    }
                    self.is_loading_playlists = false;
                    match res {
                        Ok(pls) => {
                            let count = pls.len();
                            if let Some(selected) = &mut self.selected_playlist {
                                if let Some(fresh) = pls.iter().find(|p| p.id == selected.id) {
                                    selected.clone_from(fresh);
                                }
                            }
                            self.playlists = pls;
                            self.playlists_account = Some(auth_user);
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
                    request,
                    auth_user,
                    result,
                } => {
                    if self.playlist_tracks_request != request
                        || self.settings.auth_user_index != auth_user
                    {
                        continue;
                    }
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
                AppEvent::LyricsLoaded {
                    track_id,
                    req_id,
                    lyrics,
                    is_upgraded,
                } => {
                    let cur_id = self.current_track().map(|t| t.id.as_str());
                    if crate::lyrics::is_lyrics_response_valid(
                        self.is_local_lyrics,
                        self.lyrics_req_id,
                        req_id,
                        cur_id,
                        &track_id,
                    ) {
                        if let Some(mut new_lyrics) = lyrics {
                            // Preserve manual user delay offset if one was set for current session
                            if let Some(existing) = &self.lyrics {
                                if existing.offset_ms != 0 {
                                    new_lyrics.offset_ms = existing.offset_ms;
                                }
                            }
                            self.lyrics = Some(new_lyrics);
                        } else if !is_upgraded && self.lyrics.is_none() {
                            self.lyrics = None;
                        }
                    }
                }
                AppEvent::LocalLyricsFileSelected(path) => {
                    self.lyrics_req_id = self.lyrics_req_id.wrapping_add(1);
                    if let Some(lyrics) = crate::lyrics::LocalFileAdapter::load_file(&path) {
                        self.is_local_lyrics = true;
                        self.status_message = Some(format!(
                            "✓ Загружен текст: {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        self.lyrics = Some(lyrics);
                    } else {
                        self.status_message =
                            Some("Не удалось разобрать файл текста (.lrc или .ttml)".to_string());
                    }
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
                                let _ = tx2.send(AppEvent::PlaylistsLoaded(auth_user, pls));
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
            } else if self.next_track_index().is_some() {
                self.play_next();
            } else if self.settings.wave_auto {
                self.request_wave(true);
            }
        }

        if self.current_index.is_some() && self.repeat_mode != 2 {
            if let Some(next) = self.next_track_index().and_then(|i| self.queue.get(i)) {
                if let Some(cover) = self.track_cover(ctx, next) {
                    let _ = cover.load_for_size(ctx, Vec2::splat(52.0));
                }
                if let Some(url) = next.thumbnail_url.as_deref() {
                    let cover = egui::Image::new(InnertubeClient::upgrade_thumbnail_quality(url));
                    let _ = cover.load_for_size(ctx, Vec2::splat(420.0));
                }
                if self.find_local_track_file(next).is_none() {
                    self.player
                        .prepare_stream(format!("https://www.youtube.com/watch?v={}", next.id));
                }
            }
        }

        let accent = self.accent_color();

        // Reserve the bottom bar across the full window before allocating the sidebar.
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
                .min_height(60.0)
                .max_height(84.0)
                .show_separator_line(false)
                .frame(egui::Frame::none().fill(bar_bg).inner_margin(egui::Margin {
                    left: 0.0,
                    right: 0.0,
                    top: 2.0,
                    bottom: 4.0,
                }))
                .show(ctx, |ui| {
                    if !self.player.is_alive() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("MPV плеер завершился")
                                    .color(Color32::from_rgb(239, 68, 68))
                                    .strong(),
                            );
                            if ui.button("Перезапустить плеер").clicked() {
                                if let Ok(new_player) =
                                    AudioPlayer::with_volume(self.settings.volume)
                                {
                                    new_player.set_speed(
                                        self.settings.playback_speed,
                                        self.settings.preserve_pitch,
                                    );
                                    new_player.set_audio_processing(
                                        self.settings.audio_normalization,
                                        self.settings.skip_silence,
                                        self.settings.cover_ambient_light,
                                    );
                                    self.player = new_player;
                                }
                            }
                        });
                    }

                    // 1. Линия длины трека по всей ширине окна
                    let total_bar_w = ui.available_width();
                    self.render_playback_slider(ui, total_bar_w, 10.0, &playback_info);

                    ui.add_space(2.0);

                    // 2. Единая сбалансированная строка управления: все элементы строго на одной горизонтальной оси!
                    let row_w = ui.available_width();
                    let content_w = (row_w - 24.0).max(0.0);
                    let center_w = (content_w * 0.28).max(130.0);
                    let right_w = (content_w * 0.39).max(if row_w < 570.0 { 140.0 } else { 200.0 });
                    let left_w = (content_w - center_w - right_w).max(48.0);
                    let center_gap = ((center_w - 98.0) / 3.0).max(4.0);
                    let right_gap =
                        ((right_w - if row_w < 570.0 { 114.0 } else { 180.0 }) / 5.0).max(4.0);
                    let bar_h = 52.0;

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(12.0);

                        // Слева: Обложка + Название и Автор (центр Y строго по центру строки)
                        let rounding = Rounding::same(self.settings.album_art_rounding);
                        ui.allocate_ui_with_layout(
                            Vec2::new(left_w, bar_h),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(left_w);
                                if let Some(track) = &current_track_clone {
                                    let img_resp =
                                        if let Some(cover) = self.track_cover(ui.ctx(), track) {
                                            Self::paint_square_cover(ui, cover, 44.0, rounding)
                                        } else {
                                            let (rect, resp) = ui.allocate_exact_size(
                                                Vec2::new(44.0, 44.0),
                                                egui::Sense::click(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                rounding,
                                                Color32::from_rgb(40, 40, 50),
                                            );
                                            Self::draw_vector_logo(
                                                ui.painter(),
                                                Rect::from_center_size(
                                                    rect.center(),
                                                    Vec2::new(24.0, 24.0),
                                                ),
                                                Color32::GRAY,
                                            );
                                            resp
                                        };

                                    if img_resp
                                        .interact(egui::Sense::click())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        self.toggle_playback();
                                    }

                                    ui.add_space(8.0);

                                    let text_resp = ui
                                        .vertical(|ui| {
                                            ui.set_max_width((left_w - 55.0).max(0.0));
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&track.title)
                                                        .size(13.0)
                                                        .strong()
                                                        .color(Color32::WHITE),
                                                )
                                                .truncate(),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&track.artist).size(11.0).weak(),
                                                )
                                                .truncate(),
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

                        // По центру: Кнопки управления (ровно по центру bar_h на одной высоте с обложкой)
                        ui.allocate_ui_with_layout(
                            Vec2::new(center_w, bar_h),
                            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            |ui| {
                                ui.set_min_width(center_w);
                                ui.horizontal(|ui| {
                                    let prev_btn = egui::Button::new(
                                        RichText::new("|◀")
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::from_rgb(210, 210, 220)),
                                    )
                                    .frame(false);
                                    let prev = ui.add_sized([30.0, 30.0], prev_btn);
                                    if Self::player_button_hint(prev, "Предыдущий трек", true)
                                        .clicked()
                                    {
                                        self.play_prev();
                                    }
                                    ui.add_space(center_gap);

                                    let (play_rect, play_resp) = ui.allocate_exact_size(
                                        Vec2::splat(38.0),
                                        egui::Sense::click(),
                                    );
                                    if ui.is_rect_visible(play_rect) {
                                        let p_center = play_rect.center();
                                        ui.painter().circle_filled(p_center, 19.0, accent);

                                        Self::draw_playback_icon(
                                            ui.painter(),
                                            p_center,
                                            playback_info.is_playing,
                                        );
                                    }
                                    let play_hint = if playback_info.is_playing {
                                        "Пауза"
                                    } else {
                                        "Воспроизвести"
                                    };
                                    if Self::player_button_hint(play_resp, play_hint, true)
                                        .clicked()
                                    {
                                        self.toggle_playback();
                                    }
                                    ui.add_space(center_gap);

                                    let next_btn = egui::Button::new(
                                        RichText::new("▶|")
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::from_rgb(210, 210, 220)),
                                    )
                                    .frame(false);
                                    let next = ui.add_sized([30.0, 30.0], next_btn);
                                    if Self::player_button_hint(next, "Следующий трек", true)
                                        .clicked()
                                    {
                                        self.play_next();
                                    }
                                });
                            },
                        );

                        // Справа: Время, Сердечко, Громкость (правее, с выпадающим вертикальным ползунком) и Разворачивание
                        ui.allocate_ui_with_layout(
                            Vec2::new(right_w, bar_h),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(right_w);
                                ui.add_space(right_gap);

                                // 1. Кнопка разворачивания экрана воспроизведения
                                let (rect, expand) =
                                    ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::click());
                                expand.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        ui.is_enabled(),
                                        "Развернуть плеер",
                                    )
                                });
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(expand.id.with("no_button_feedback"), true);
                                });
                                let hover = ui.ctx().animate_bool_with_time(
                                    expand.id.with("expand_hover"),
                                    expand.hovered(),
                                    0.22,
                                );
                                // Grow the whole icon first, then slide the four corners apart.
                                let scale = 1.0 + 0.14 * (hover / 0.5).min(1.0);
                                let spread = 1.8 * ((hover - 0.5) / 0.5).max(0.0);
                                let edge = 5.5 * scale + spread;
                                let arm = 3.4 * scale;
                                let stroke = Stroke {
                                    width: 1.8,
                                    color: ui.style().interact(&expand).fg_stroke.color,
                                };
                                for x in [-1.0, 1.0] {
                                    for y in [-1.0, 1.0] {
                                        let corner = rect.center() + Vec2::new(x * edge, y * edge);
                                        ui.painter().add(egui::Shape::line(
                                            vec![
                                                corner - Vec2::new(x * arm, 0.0),
                                                corner,
                                                corner - Vec2::new(0.0, y * arm),
                                            ],
                                            stroke,
                                        ));
                                    }
                                }
                                if Self::player_button_hint(expand, "Развернуть плеер", true)
                                    .clicked()
                                {
                                    open_now_playing = true;
                                }

                                ui.add_space(right_gap);

                                // 2. Громкость: динамическая векторная иконка + вертикальный слайдер при наведении
                                let (vol_rect, vol_resp) = ui.allocate_exact_size(
                                    Vec2::new(26.0, 26.0),
                                    egui::Sense::click(),
                                );
                                let is_muted = self.settings.volume <= 0.0;
                                let vol_icon_col = if vol_resp.hovered() {
                                    accent
                                } else {
                                    Color32::from_rgb(200, 200, 215)
                                };
                                Self::draw_vector_speaker(
                                    ui.painter(),
                                    vol_rect,
                                    self.settings.volume,
                                    is_muted,
                                    vol_icon_col,
                                );

                                // Mute toggle on click
                                if vol_resp.clicked() {
                                    if self.settings.volume > 0.0 {
                                        self.pre_mute_volume = self.settings.volume;
                                        self.settings.volume = 0.0;
                                    } else {
                                        self.settings.volume = if self.pre_mute_volume > 0.0 {
                                            self.pre_mute_volume
                                        } else {
                                            75.0
                                        };
                                    }
                                    self.player.set_volume(self.settings.volume);
                                    self.settings.save();
                                }

                                // Wheel scroll for quick adjustment
                                let scroll = ui.input(|i| i.raw_scroll_delta.y);
                                if vol_resp.hovered() && scroll.abs() > 0.1 {
                                    let delta = scroll.signum() as f64 * 5.0;
                                    let new_vol = (self.settings.volume + delta).clamp(0.0, 100.0);
                                    self.settings.volume = new_vol;
                                    self.player.set_volume(new_vol);
                                    self.settings.save();
                                }

                                // Вертикальный ползунок громкости во всплывающей панели при наведении
                                let popup_id = ui.make_persistent_id("bottom_bar_vol_popup");
                                let popup_rect = Rect::from_min_size(
                                    Pos2::new(vol_rect.center().x - 17.0, vol_rect.min.y - 146.0),
                                    Vec2::new(34.0, 140.0),
                                );
                                let pointer_pos = ui.input(|i| i.pointer.hover_pos());
                                let over_popup = pointer_pos.is_some_and(|p| {
                                    let panel = ui
                                        .ctx()
                                        .memory(|memory| memory.area_rect(popup_id))
                                        .unwrap_or(popup_rect);
                                    panel.union(vol_rect).contains(p)
                                });

                                // Open only from the speaker icon. Once open, keep the popup
                                // visible while the pointer moves from the icon onto its slider.
                                if vol_resp.hovered() || (self.volume_popup_open && over_popup) {
                                    self.volume_popup_open = true;
                                    if self.volume_popup_open {
                                        egui::Area::new(popup_id)
                                            .order(egui::Order::Foreground)
                                            .fixed_pos(popup_rect.min)
                                            .show(ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style())
                                                    .fill(Color32::from_rgb(20, 20, 26))
                                                    .stroke(Stroke::new(
                                                        1.0_f32,
                                                        Color32::from_rgb(50, 50, 65),
                                                    ))
                                                    .rounding(Rounding::same(8.0))
                                                    .inner_margin(6.0)
                                                    .show(ui, |ui| {
                                                        ui.set_width(22.0);
                                                        ui.set_height(128.0);
                                                        ui.vertical_centered(|ui| {
                                                            ui.label(
                                                                RichText::new(format!(
                                                                    "{:.0}%",
                                                                    self.settings.volume
                                                                ))
                                                                .size(9.0)
                                                                .weak(),
                                                            );
                                                            ui.add_space(4.0);
                                                            let mut vol = self.settings.volume;
                                                            let slider = egui::Slider::new(
                                                                &mut vol,
                                                                0.0..=100.0,
                                                            )
                                                            .vertical()
                                                            .show_value(false);
                                                            if ui
                                                                .add_sized([18.0, 95.0], slider)
                                                                .changed()
                                                            {
                                                                self.player.set_volume(vol);
                                                                self.settings.volume = vol;
                                                                self.settings.save();
                                                            }
                                                        });
                                                    });
                                            });
                                    }
                                } else {
                                    self.volume_popup_open = false;
                                }

                                ui.add_space(right_gap);

                                // 3. Кнопка «Сердечко» (в точности как в полноэкранном режиме)
                                if let Some(track) = &current_track_clone {
                                    let is_liked = self.library.is_liked(&track.id);
                                    let heart = Self::heart_button(ui, &track.id, is_liked);
                                    if Self::player_button_hint(
                                        heart,
                                        Self::heart_hint(is_liked),
                                        true,
                                    )
                                    .clicked()
                                    {
                                        track_to_toggle_like = Some(track.clone());
                                    }
                                    ui.add_space(right_gap);
                                    if Self::add_to_playlist_button(ui, 26.0, accent).clicked() {
                                        self.open_playlist_picker(ui.ctx(), track.clone());
                                    }
                                }

                                ui.add_space(right_gap);

                                // 4. Тайминг трека на той же горизонтальной линии
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
                                if row_w >= 570.0 {
                                    let time_text = format!("{} / {}", curr_str, dur_str);
                                    ui.label(RichText::new(time_text).size(11.0).weak());
                                }
                            },
                        );
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

        // Sidebar sits above the full-width mini player.
        let narrow_sidebar = ctx.screen_rect().width() < 720.0;
        let show_sidebar = self.active_tab != ActiveTab::NowPlaying;
        if show_sidebar {
            egui::SidePanel::left("sidebar")
                .resizable(false)
                .exact_width(if narrow_sidebar {
                    126.0
                } else if self.settings.slim_nav_bar {
                    180.0
                } else {
                    220.0
                })
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sidebar_scroll")
                        .show(ui, |ui| {
                            ui.add_space(if narrow_sidebar { 8.0 } else { 16.0 });
                            ui.horizontal(|ui| {
                                let size = Vec2::splat(if narrow_sidebar { 24.0 } else { 30.0 });
                                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                Self::draw_vector_logo(ui.painter(), rect, accent);
                                let title_size = if narrow_sidebar { 15.0 } else { 18.0 };
                                ui.label(
                                    RichText::new("CIBITUBE")
                                        .size(title_size)
                                        .strong()
                                        .color(accent),
                                );
                            });

                            ui.add_space(if narrow_sidebar { 6.0 } else { 20.0 });
                            ui.separator();
                            ui.add_space(10.0);

                            // Navigation buttons without emojis, filtered by user settings
                            let strings = crate::i18n::tr(&self.settings.ui_language);
                            let is_en = self.settings.ui_language == "en";
                            let mut tabs: Vec<(ActiveTab, String)> = Vec::new();

                            if self.settings.show_nav_search {
                                tabs.push((ActiveTab::Search, strings.nav_search.to_string()));
                            }
                            if self.settings.show_nav_liked {
                                tabs.push((
                                    ActiveTab::Liked,
                                    if is_en {
                                        "Liked Songs".to_string()
                                    } else {
                                        "Любимые треки".to_string()
                                    },
                                ));
                            }
                            if self.settings.show_nav_playlists {
                                tabs.push((ActiveTab::Playlists, strings.nav_library.to_string()));
                            }
                            if self.settings.show_nav_wave {
                                tabs.push((
                                    ActiveTab::Wave,
                                    if is_en {
                                        "My wave"
                                    } else {
                                        "Своя волна"
                                    }
                                    .into(),
                                ));
                            }
                            if self.settings.show_nav_queue {
                                tabs.push((
                                    ActiveTab::Queue,
                                    if is_en {
                                        "Queue".to_string()
                                    } else {
                                        "Очередь".to_string()
                                    },
                                ));
                            }
                            if self.settings.show_nav_lyrics {
                                tabs.push((
                                    ActiveTab::Lyrics,
                                    strings.player_lyrics_tab.to_string(),
                                ));
                            }
                            if self.settings.show_nav_downloads {
                                tabs.push((
                                    ActiveTab::Downloads,
                                    strings.nav_downloads.to_string(),
                                ));
                            }
                            // Пункт «Настройки» отображается ВСЕГДА и ни при каких условиях не скрывается!
                            tabs.push((ActiveTab::Settings, strings.nav_settings.to_string()));

                            for (tab, label) in tabs {
                                let is_active = self.active_tab == tab;
                                let text = RichText::new(label).size(13.0);
                                let button = if is_active {
                                    egui::Button::new(text.color(Color32::WHITE).strong())
                                        .fill(accent)
                                        .rounding(Rounding::same(17.0))
                                } else {
                                    egui::Button::new(text.color(Color32::from_rgb(200, 200, 210)))
                                        .fill(Color32::TRANSPARENT)
                                        .rounding(Rounding::same(8.0))
                                };

                                if ui
                                    .add_sized(
                                        [
                                            ui.available_width(),
                                            if narrow_sidebar { 27.0 } else { 34.0 },
                                        ],
                                        button,
                                    )
                                    .clicked()
                                {
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
                                    ui.label(
                                        RichText::new(acc)
                                            .size(11.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    );
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
                                    RichText::new(format!("Обновление {}", info.version))
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
                                ui.label(RichText::new("Cibitube").size(10.0).weak());
                            });
                        });
                });
        }

        // 3. Central Content Panel
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            match self.active_tab {
                ActiveTab::Search => self.render_search_tab(ui),
                ActiveTab::Liked => self.render_liked_tab(ui),
                ActiveTab::Playlists => self.render_playlists_tab(ui),
                ActiveTab::Queue => self.render_queue_tab(ui),
                ActiveTab::Wave => self.render_wave_tab(ui),
                ActiveTab::Lyrics => self.render_lyrics_tab(ui, playback_info.position_secs),
                ActiveTab::Downloads => self.render_downloads_tab(ui),
                ActiveTab::Settings => self.render_settings_tab(ui, ctx),
                ActiveTab::NowPlaying => self.render_now_playing_tab(ui),
            }
        });
        self.render_playlist_picker(ctx);
        self.render_welcome_tour(ctx);
        self.paint_button_feedback(ctx);
        let now = ctx.input(|input| input.time);
        if let Some(hint) =
            ctx.data(|data| data.get_temp::<PlayerClickHint>(egui::Id::new(PLAYER_CLICK_HINT)))
        {
            if hint.expires_at > now {
                Self::paint_player_hint(ctx, hint.anchor, hint.text);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    hint.expires_at - now,
                ));
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let playback_info = self
            .player
            .info
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        self.trigger_auto_sync(&playback_info);
    }
}

impl MetrolistApp {
    fn paint_button_feedback(&self, ctx: &egui::Context) {
        let state_id = egui::Id::new("button_feedback");
        let mut ids: std::collections::HashSet<egui::Id> =
            ctx.data(|data| data.get_temp(state_id)).unwrap_or_default();
        ctx.interaction_snapshot(|interaction| {
            ids.extend(interaction.hovered.iter().copied());
            ids.extend(interaction.clicked);
        });

        ids.retain(|id| {
            let Some(response) = ctx.read_response(*id) else {
                return false;
            };
            // Include standard buttons and custom click targets, but skip sliders.
            if !response.enabled()
                || ctx
                    .data(|data| data.get_temp::<bool>(id.with("no_button_feedback")))
                    .unwrap_or(false)
                || !response.sense.click
                || response.sense.drag
                || !response.interact_rect.is_positive()
            {
                return false;
            }

            let pressed = response.is_pointer_button_down_on() || response.clicked();
            let hover = ctx.animate_bool_with_time(id.with("hover"), response.hovered(), 0.14);
            let press = ctx.animate_bool_with_time(id.with("press"), pressed, 0.09);
            if hover > 0.0 || press > 0.0 {
                let scale = 1.0 - 0.025 * hover - 0.045 * press;
                let transform = egui::emath::TSTransform::new(
                    response.rect.center().to_vec2() * (1.0 - scale),
                    scale,
                );
                ctx.graphics_mut(|graphics| {
                    let list = graphics.entry(response.layer_id);
                    let bounds = response.rect.expand(2.0);
                    let indices: Vec<_> = list
                        .all_entries()
                        .enumerate()
                        .filter_map(|(i, entry)| {
                            // Keep glyphs at their original raster scale.
                            if matches!(entry.shape, egui::Shape::Text(_)) {
                                return None;
                            }
                            let rect = entry.shape.visual_bounding_rect();
                            (rect.is_positive() && bounds.contains_rect(rect)).then_some(i)
                        })
                        .collect();
                    for i in indices {
                        list.mutate_shape(egui::layers::ShapeIdx(i), |entry| {
                            entry.shape.transform(transform);
                        });
                    }
                });
            }
            response.hovered() || pressed || hover > 0.0 || press > 0.0
        });
        ctx.data_mut(|data| data.insert_temp(state_id, ids));
    }

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
                    RichText::new("Введите название трека или исполнителя для поиска")
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
                                Self::paint_square_cover(
                                    ui,
                                    egui::Image::new(thumb),
                                    44.0,
                                    rounding,
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
                                        egui::Button::new(RichText::new("Играть").strong())
                                            .fill(accent)
                                            .stroke(Stroke::NONE)
                                            .rounding(Rounding::same(6.0));
                                    if ui.add(play_btn).clicked() {
                                        track_idx_to_play = Some(idx);
                                    }

                                    // Like button
                                    let is_liked = self.library.is_liked(&track.id);
                                    if Self::heart_button(ui, &track.id, is_liked)
                                        .on_hover_text(Self::heart_hint(is_liked))
                                        .clicked()
                                    {
                                        self.library.toggle_like(track.clone());
                                    }

                                    // Add to queue
                                    if ui.button("+ Очередь").clicked() {
                                        track_to_queue = Some(track.clone());
                                    }

                                    // Download
                                    if ui.button("Скачать").on_hover_text("Скачать MP3").clicked()
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
            ui.label(RichText::new("Любимые треки").size(18.0).strong());
            ui.add_space(16.0);

            // Sync from YouTube button
            if !self.settings.google_cookie.is_empty() {
                let user_name = self.settings.account_name.as_deref().unwrap_or("аккаунтом");
                let sync_label = format!("Синхронизировать с {}", user_name);
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
                let shuffle_btn = egui::Button::new(RichText::new("Перемешать всё").strong())
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

                let dl_all_btn = egui::Button::new(RichText::new("Скачать все").strong())
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

        let ghost_key = egui::Id::new("liked_heart_ghost");
        let now = ui.input(|input| input.time);
        let ghost = ui
            .ctx()
            .data(|data| data.get_temp::<(usize, Track, f64)>(ghost_key))
            .filter(|(_, track, until)| now < *until && !self.library.is_liked(&track.id));
        if self.library.liked_tracks.is_empty() && ghost.is_none() {
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
                let mut track_to_play = None;
                let mut track_to_download = None;
                let mut track_to_toggle_like = None;
                let rounding = Rounding::same(self.settings.album_art_rounding);

                let mut rows: Vec<_> = self.library.liked_tracks.iter().collect();
                if let Some((index, track, _)) = &ghost {
                    rows.insert((*index).min(rows.len()), track);
                }
                for (idx, track) in rows.into_iter().enumerate() {
                    let frame = egui::Frame::none()
                        .fill(Color32::from_rgb(26, 26, 34))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(8.0)
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)));

                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(thumb) = &track.thumbnail_url {
                                Self::paint_square_cover(
                                    ui,
                                    egui::Image::new(thumb),
                                    40.0,
                                    rounding,
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
                                            .stroke(Stroke::NONE)
                                            .rounding(Rounding::same(6.0));
                                    if ui.add(play_btn).clicked() {
                                        track_to_play = Some(track.clone());
                                    }

                                    let is_liked = self.library.is_liked(&track.id);
                                    if Self::heart_button(ui, &track.id, is_liked)
                                        .on_hover_text(Self::heart_hint(is_liked))
                                        .clicked()
                                    {
                                        track_to_toggle_like = Some((idx, track.clone()));
                                    }

                                    if ui.button("Скачать").on_hover_text("Скачать MP3").clicked()
                                    {
                                        track_to_download = Some(track.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(4.0);
                }

                if let Some((index, track)) = track_to_toggle_like {
                    if self.library.is_liked(&track.id) {
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(ghost_key, (index, track.clone(), now + 0.40))
                        });
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(400));
                    }
                    self.library.toggle_like(track);
                }
                if let Some(track) = track_to_play {
                    self.queue = self.library.liked_tracks.clone();
                    self.current_index = self.queue.iter().position(|row| row.id == track.id);
                    self.play_track(track);
                }
                if let Some(t) = track_to_download {
                    self.download_track(t);
                }
            });
    }

    fn playlist_picker_actions(
        ui: &mut egui::Ui,
        membership: &PlaylistMembership,
        enabled: bool,
        accent: Color32,
    ) -> Option<PlaylistPickerAction> {
        if !matches!(membership, PlaylistMembership::Present(_)) {
            let button = ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new("Добавить +").color(Color32::WHITE)).fill(accent),
            );
            let button = if matches!(membership, PlaylistMembership::Unknown) {
                button.on_hover_text("Не удалось проверить наличие трека. Чтобы проверить снова, открой меню заново.")
            } else {
                button
            };
            return button.clicked().then_some(PlaylistPickerAction::Add);
        }

        ui.allocate_ui_with_layout(
            Vec2::new(158.0, 50.0),
            egui::Layout::top_down(egui::Align::Max),
            |ui| {
                let remove = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = ui.add_enabled(
                            enabled,
                            egui::Button::new("")
                                .min_size(Vec2::splat(25.0))
                                .rounding(Rounding::same(4.0)),
                        );
                        let center = button.rect.center();
                        let stroke = Stroke::new(
                            1.4_f32,
                            Color32::from_rgb(244, 79, 95).gamma_multiply(if enabled {
                                1.0
                            } else {
                                0.45
                            }),
                        );
                        let point = |x, y| center + Vec2::new(x, y);
                        ui.painter()
                            .line_segment([point(-5.0, -3.0), point(5.0, -3.0)], stroke);
                        ui.painter()
                            .line_segment([point(-2.0, -5.0), point(2.0, -5.0)], stroke);
                        ui.painter().add(egui::Shape::line(
                            vec![
                                point(-4.0, -2.0),
                                point(-3.0, 5.0),
                                point(3.0, 5.0),
                                point(4.0, -2.0),
                            ],
                            stroke,
                        ));
                        ui.label("✓ Уже в плейлисте");
                        button.on_hover_text("Удалить одну запись трека").clicked()
                    })
                    .inner;
                let add = ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(
                            RichText::new("Добавить всё равно").color(Color32::WHITE),
                        ),
                    )
                    .clicked();
                if remove {
                    Some(PlaylistPickerAction::Remove)
                } else {
                    add.then_some(PlaylistPickerAction::AddAgain)
                }
            },
        )
        .inner
    }

    fn playlist_card<R>(
        ui: &mut egui::Ui,
        playlist: &Playlist,
        rounding: Rounding,
        accent: Color32,
        render_action: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        egui::Frame::none()
            .fill(Color32::from_rgb(26, 26, 34))
            .rounding(Rounding::same(8.0))
            .inner_margin(10.0)
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let cover_size = Vec2::splat(56.0);
                    if let Some(url) = &playlist.thumbnail_url {
                        Self::paint_square_cover(ui, egui::Image::new(url), 56.0, rounding);
                    } else {
                        let (rect, _) = ui.allocate_exact_size(cover_size, egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, rounding, Color32::from_rgb(45, 45, 55));
                        Self::draw_vector_logo(ui.painter(), rect.shrink(17.0), accent);
                    }
                    ui.add_space(10.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let action = render_action(ui);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&playlist.title)
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .truncate(),
                            );
                            if !playlist.subtitle.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&playlist.subtitle).size(12.0).weak(),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                        action
                    })
                    .inner
                })
                .inner
            })
            .inner
    }

    fn render_playlists_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        if self
            .selected_playlist
            .as_ref()
            .is_some_and(|p| self.settings.youtube_playlist_hidden(&p.id))
        {
            self.selected_playlist = None;
            self.playlist_tracks.clear();
        }
        // 1. If viewing a specific playlist
        if let Some(pl) = self.selected_playlist.clone() {
            let regained_focus = ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(true)))
            });
            if regained_focus
                && !self.is_loading_playlist_tracks
                && !self.settings.google_cookie.is_empty()
            {
                self.refresh_playlist_tracks(&pl.id);
            }
            ui.horizontal(|ui| {
                if ui.button("← Ко всем плейлистам").clicked() {
                    self.selected_playlist = None;
                    self.playlist_tracks.clear();
                    return;
                }
                ui.add_space(10.0);
                ui.label(RichText::new(&pl.title).size(18.0).strong());
                if ui
                    .add_enabled(
                        !self.is_loading_playlist_tracks,
                        egui::Button::new("Обновить"),
                    )
                    .on_hover_text("Загрузить треки из YouTube Music")
                    .clicked()
                {
                    self.refresh_playlist_tracks(&pl.id);
                }
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if let Some(thumb) = &pl.thumbnail_url {
                    Self::paint_square_cover(
                        ui,
                        egui::Image::new(thumb),
                        64.0,
                        Rounding::same(self.settings.album_art_rounding),
                    );
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&pl.subtitle).size(12.0).weak());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if !self.playlist_tracks.is_empty() {
                            let play_all_btn = egui::Button::new(
                                RichText::new("Слушать всё").color(Color32::WHITE).strong(),
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
                                egui::Button::new(RichText::new("Перемешать плейлист").strong())
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
                            let dl_all_btn =
                                egui::Button::new(RichText::new("Скачать весь плейлист").strong())
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

            if self.is_loading_playlist_tracks && self.playlist_tracks.is_empty() {
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
                                Self::paint_square_cover(
                                    ui,
                                    egui::Image::new(thumb),
                                    36.0,
                                    rounding,
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
                                    if ui.button("Скачать").on_hover_text("Скачать MP3").clicked()
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
                    let btn = egui::Button::new(RichText::new("Понравившиеся").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    if ui.add_sized([160.0, 36.0], btn).clicked() {
                        self.active_tab = ActiveTab::Liked;
                    }
                }
                if self.settings.show_downloaded_playlist {
                    let btn = egui::Button::new(RichText::new("Скачанные").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    if ui.add_sized([160.0, 36.0], btn).clicked() {
                        self.active_tab = ActiveTab::Downloads;
                    }
                }
                if self.settings.show_top_playlist {
                    let btn = egui::Button::new(RichText::new("Топ треки").strong())
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
                    let btn = egui::Button::new(RichText::new("Кэшированные").strong())
                        .fill(Color32::from_rgb(32, 32, 40))
                        .rounding(Rounding::same(8.0));
                    let _ = ui.add_sized([160.0, 36.0], btn);
                }
                if self.settings.show_uploaded_playlist {
                    let btn = egui::Button::new(RichText::new("Загруженные").strong())
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
            ui.label(RichText::new("Плейлисты YouTube Music").size(18.0).strong());
            ui.add_space(16.0);
            if ui
                .button(RichText::new("Синхронизировать плейлисты").strong())
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

        if self
            .playlists
            .iter()
            .all(|p| self.settings.youtube_playlist_hidden(&p.id))
        {
            ui.label("Все плейлисты скрыты. Их можно вернуть в настройках библиотеки.");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let rounding = Rounding::same(self.settings.album_art_rounding);

            for pl in &self.playlists {
                if self.settings.youtube_playlist_hidden(&pl.id) {
                    continue;
                }
                if Self::playlist_card(ui, pl, rounding, accent, |ui| {
                    ui.add(
                        egui::Button::new(RichText::new("Открыть →").strong())
                            .fill(accent)
                            .rounding(Rounding::same(6.0)),
                    )
                    .clicked()
                }) {
                    playlist_to_open = Some(pl.clone());
                }
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
            ui.label(RichText::new("Очередь воспроизведения").size(18.0).strong());
            ui.add_space(16.0);
            if ui.button("Перемешать очередь").clicked() {
                self.shuffle_queue();
                self.status_message = Some("Очередь перемешана".to_string());
            }
            ui.add_space(8.0);
            if ui.button("Очистить очередь").clicked() {
                self.queue.clear();
                self.current_index = None;
                self.wave_request = self.wave_request.wrapping_add(1);
                self.wave_busy = false;
            }
        });

        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(
                    &mut self.settings.wave_auto,
                    "Автодополнять из «Своей волны»",
                )
                .changed()
            {
                self.settings.save();
            }
            if ui.button("Настроить свою волну").clicked() {
                self.active_tab = ActiveTab::Wave;
            }
        });
        if self.wave_busy {
            ui.spinner();
        }
        if !self.wave_message.is_empty() {
            ui.label(&self.wave_message);
        }
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

    fn render_downloads_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Локальные загрузки").size(20.0).strong());
            ui.add_space(8.0);
            let count_text = format!("{} треков", self.library.downloaded_tracks.len());
            ui.label(RichText::new(count_text).size(12.0).weak());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Открыть папку").clicked() {
                    let dir = self.settings.download_dir.clone();
                    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                }

                if !self.library.downloaded_tracks.is_empty() {
                    ui.add_space(8.0);
                    if ui.button("Перемешать").clicked() {
                        let mut shuffled = self.library.downloaded_tracks.clone();
                        Self::shuffle_slice(&mut shuffled);
                        self.queue = shuffled;
                        if let Some(first) = self.queue.first().cloned() {
                            self.play_track(first);
                        }
                    }

                    ui.add_space(8.0);
                    let play_all_btn = egui::Button::new(RichText::new("Слушать всё").strong())
                        .fill(accent)
                        .rounding(Rounding::same(6.0));
                    if ui.add(play_all_btn).clicked() {
                        self.queue = self.library.downloaded_tracks.clone();
                        if let Some(first) = self.queue.first().cloned() {
                            self.play_track(first);
                        }
                    }
                }
            });
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

        let mut track_to_play = None;
        let mut track_to_delete: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let rounding = Rounding::same(self.settings.album_art_rounding);

            for track in &self.library.downloaded_tracks {
                let frame = egui::Frame::none()
                    .fill(Color32::from_rgb(26, 26, 34))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 48)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(10.0);

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(cover) = self.track_cover(ui.ctx(), track) {
                            Self::paint_square_cover(ui, cover, 52.0, rounding);
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(52.0, 52.0), egui::Sense::hover());
                            ui.painter()
                                .rect_filled(rect, rounding, Color32::from_rgb(38, 38, 48));
                            Self::draw_vector_logo(
                                ui.painter(),
                                Rect::from_center_size(rect.center(), Vec2::new(26.0, 26.0)),
                                Color32::GRAY,
                            );
                        }

                        ui.add_space(10.0);

                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&track.title)
                                    .size(14.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&track.artist).size(12.0).weak());
                                if !track.duration_text.is_empty() {
                                    ui.label(
                                        RichText::new(format!("• {}", track.duration_text))
                                            .size(11.0)
                                            .weak(),
                                    );
                                }
                            });
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let del_btn = egui::Button::new(
                                RichText::new("Удалить")
                                    .size(12.0)
                                    .color(Color32::from_rgb(244, 63, 94)),
                            )
                            .frame(false);
                            if ui
                                .add(del_btn)
                                .on_hover_text("Удалить из загрузок")
                                .clicked()
                            {
                                track_to_delete = Some(track.id.clone());
                            }

                            ui.add_space(8.0);

                            let play_btn = egui::Button::new(RichText::new("Слушать").strong())
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(play_btn).clicked() {
                                track_to_play = Some(track.clone());
                            }

                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("MP3")
                                    .size(11.0)
                                    .color(Color32::from_rgb(16, 185, 129)),
                            );
                        });
                    });
                });
                ui.add_space(6.0);
            }
        });

        if let Some(del_id) = track_to_delete {
            self.library.downloaded_tracks.retain(|t| t.id != del_id);
            self.library.save();
        }

        if let Some(t) = track_to_play {
            self.play_track(t);
        }
    }

    fn render_now_playing_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();
        let playback_info = self
            .player
            .info
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let current_track = self.current_track().cloned();

        let avail_w = ui.available_width();
        let avail_h = ui.available_height();

        let cover = if self.show_lyrics_in_player || self.settings.hide_player_thumbnail {
            None
        } else {
            current_track.as_ref().and_then(|track| {
                track
                    .thumbnail_url
                    .as_deref()
                    .map(InnertubeClient::upgrade_thumbnail_quality)
                    .map(egui::Image::new)
                    .filter(|cover| {
                        cover
                            .load_for_size(ui.ctx(), Vec2::splat(420.0))
                            .is_ok_and(|texture| texture.size().is_some())
                    })
                    .or_else(|| self.track_cover(ui.ctx(), track))
            })
        };
        let ambient = self.settings.cover_ambient_light.then(|| {
            let painter = ui.painter().clone();
            let shape = painter.add(egui::Shape::Noop);
            (painter, shape)
        });

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

            let center_w = (ui.available_width() - 98.0).max(70.0);
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
                        .button(RichText::new("Скачать").size(12.0))
                        .on_hover_text("Скачать трек")
                        .clicked()
                    {
                        self.download_track(track_clone);
                    }
                }
            });
        });

        ui.add_space(6.0);

        // 2. Responsive dimensions: dynamically scaled with window size
        let scale = ((avail_w / 460.0).min(avail_h / 620.0)).clamp(0.65, 1.15);
        let space_for_art = (avail_h - 260.0).clamp(90.0, 420.0);
        let max_w_allowed = (avail_w * 0.60).max(90.0);
        let art_size = space_for_art.min(max_w_allowed).clamp(90.0, 420.0);
        let flexible_gap = (avail_h - (48.0 + art_size + 210.0)).clamp(2.0, 16.0);

        let surface = ui.visuals().panel_fill.lerp_to_gamma(accent, 0.12);
        let control_fill = surface.lerp_to_gamma(accent, 0.20);
        egui::TopBottomPanel::bottom("now_playing_controls")
            .resizable(false)
            .min_height(136.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(surface)
                    .rounding(24.0)
                    .inner_margin(12.0),
            )
            .show_inside(ui, |ui| {
                let controls_width = ui.available_width().min(480.0);
                let offset = (ui.available_width() - controls_width) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add_space(offset);
                    ui.allocate_ui_with_layout(
                        Vec2::new(controls_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add_space((6.0 * scale).clamp(3.0, 6.0));

                            // Progress Slider across the ENTIRE artwork width
                            self.render_playback_slider(
                                ui,
                                controls_width,
                                (16.0 * scale).clamp(12.0, 18.0),
                                &playback_info,
                            );

                            ui.add_space(2.0);

                            // Time labels flush with slider ends
                            ui.allocate_ui_with_layout(
                                Vec2::new(controls_width, 16.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
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
                                    ui.label(
                                        RichText::new(curr_str)
                                            .size((11.0 * scale).clamp(9.0, 12.0))
                                            .weak(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(dur_str)
                                                    .size((11.0 * scale).clamp(9.0, 12.0))
                                                    .weak(),
                                            );
                                        },
                                    );
                                },
                            );

                            let v_gap = (12.0 * scale).clamp(4.0, 14.0);
                            ui.add_space(v_gap);

                            // Controls row: Dynamically scaled buttons with window resize
                            let side_btn_w = (48.0 * scale).clamp(32.0, 56.0);
                            let side_btn_h = (44.0 * scale).clamp(32.0, 54.0);
                            let side_icon_sz = (19.0 * scale).clamp(13.0, 22.0);

                            let play_btn_w = (130.0 * scale).clamp(80.0, 160.0);
                            let play_font_sz = (15.0 * scale).clamp(11.0, 17.0);
                            let gap_btn = (12.0 * scale).clamp(6.0, 16.0);

                            let playback_w = side_btn_w * 2.0 + play_btn_w + gap_btn * 2.0;
                            let playback_pad = ((controls_width - playback_w) / 2.0).max(0.0);

                            ui.horizontal(|ui| {
                                ui.add_space(playback_pad);

                                let prev_btn = egui::Button::new(
                                    RichText::new("|◀")
                                        .size(side_icon_sz)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(control_fill)
                                .rounding(Rounding::same(18.0));
                                let prev = ui.add_sized([side_btn_w, side_btn_h], prev_btn);
                                if Self::player_button_hint(prev, "Предыдущий трек", false)
                                    .clicked()
                                {
                                    self.play_prev();
                                }

                                ui.add_space(gap_btn);

                                let text = if playback_info.is_playing {
                                    "Пауза"
                                } else {
                                    "Играть"
                                };
                                let (rect, play) = ui.allocate_exact_size(
                                    Vec2::new(play_btn_w, side_btn_h),
                                    egui::Sense::click(),
                                );
                                if ui.is_rect_visible(rect) {
                                    let painter = ui.painter();
                                    painter.rect_filled(rect, side_btn_h / 2.0, accent);
                                    let label = painter.layout_no_wrap(
                                        text.into(),
                                        egui::FontId::proportional(play_font_sz),
                                        Color32::WHITE,
                                    );
                                    let left = rect.center().x - (label.size().x + 22.0) / 2.0;
                                    Self::draw_playback_icon(
                                        painter,
                                        Pos2::new(left + 7.0, rect.center().y),
                                        playback_info.is_playing,
                                    );
                                    let y = rect.center().y - label.size().y / 2.0;
                                    painter.galley(
                                        Pos2::new(left + 22.0, y),
                                        label,
                                        Color32::WHITE,
                                    );
                                }
                                if Self::player_button_hint(play, text, false).clicked() {
                                    self.toggle_playback();
                                }

                                ui.add_space(gap_btn);

                                let next_btn = egui::Button::new(
                                    RichText::new("▶|")
                                        .size(side_icon_sz)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(control_fill)
                                .rounding(Rounding::same(18.0));
                                let next = ui.add_sized([side_btn_w, side_btn_h], next_btn);
                                if Self::player_button_hint(next, "Следующий трек", false).clicked()
                                {
                                    self.play_next();
                                }
                            });

                            ui.add_space(v_gap);

                            // Bottom action row: queue, shuffle, repeat and volume.
                            // Raised higher up directly under playback controls!
                            let act_btn_w = (36.0 * scale).clamp(26.0, 42.0);
                            let act_btn_h = (30.0 * scale).clamp(22.0, 36.0);
                            let act_font_sz = (16.0 * scale).clamp(12.0, 18.0);
                            let act_gap = (10.0 * scale).clamp(5.0, 14.0);
                            let action_w = act_btn_w * 4.0 + act_gap * 3.0;
                            let action_pad = ((controls_width - action_w) / 2.0).max(0.0);

                            ui.horizontal(|ui| {
                                ui.add_space(action_pad);

                                let queue_btn = egui::Button::new(
                                    RichText::new("≡").size(act_font_sz).color(Color32::WHITE),
                                )
                                .frame(false);
                                let queue = ui.add_sized([act_btn_w, act_btn_h], queue_btn);
                                if Self::player_button_hint(queue, "Открыть очередь", false)
                                    .clicked()
                                {
                                    self.active_tab = ActiveTab::Queue;
                                }

                                ui.add_space(act_gap);

                                let shuffle_col = if self.shuffle_state {
                                    accent
                                } else {
                                    Color32::GRAY
                                };
                                let shuffle_btn = egui::Button::new(
                                    RichText::new("⇄").size(act_font_sz).color(shuffle_col),
                                )
                                .frame(false);
                                let shuffle = ui.add_sized([act_btn_w, act_btn_h], shuffle_btn);
                                let shuffle_hint = if self.shuffle_state {
                                    "Выключить перемешивание"
                                } else {
                                    "Перемешать очередь"
                                };
                                if Self::player_button_hint(shuffle, shuffle_hint, false).clicked()
                                {
                                    self.shuffle_state = !self.shuffle_state;
                                    if self.shuffle_state {
                                        self.shuffle_queue();
                                        self.status_message =
                                            Some("Очередь перемешана".to_string());
                                    }
                                }

                                ui.add_space(act_gap);

                                let (rep_icon, rep_col, rep_hint) = match self.repeat_mode {
                                    1 => ("↻", accent, "Повторять текущий трек"),
                                    2 => ("↻1", accent, "Выключить повтор"),
                                    _ => ("↻", Color32::GRAY, "Повторять очередь"),
                                };
                                let rep_btn = egui::Button::new(
                                    RichText::new(rep_icon).size(act_font_sz).color(rep_col),
                                )
                                .frame(false);
                                let repeat = ui.add_sized([act_btn_w, act_btn_h], rep_btn);
                                if Self::player_button_hint(repeat, rep_hint, false).clicked() {
                                    self.repeat_mode = (self.repeat_mode + 1) % 3;
                                }

                                ui.add_space(act_gap);

                                // 5. Громкость: в точности как в нижней панели (иконка + вертикальный ползунок + колесико мыши)
                                let (np_vol_rect, np_vol_resp) = ui.allocate_exact_size(
                                    Vec2::new(act_btn_w, act_btn_h),
                                    egui::Sense::click(),
                                );
                                let is_muted = self.settings.volume <= 0.0;
                                let vol_icon_col = if np_vol_resp.hovered() {
                                    accent
                                } else {
                                    Color32::from_rgb(200, 200, 215)
                                };
                                Self::draw_vector_speaker(
                                    ui.painter(),
                                    np_vol_rect,
                                    self.settings.volume,
                                    is_muted,
                                    vol_icon_col,
                                );

                                // Переключение Mute по клику
                                if np_vol_resp.clicked() {
                                    if self.settings.volume > 0.0 {
                                        self.pre_mute_volume = self.settings.volume;
                                        self.settings.volume = 0.0;
                                    } else {
                                        self.settings.volume = if self.pre_mute_volume > 0.0 {
                                            self.pre_mute_volume
                                        } else {
                                            75.0
                                        };
                                    }
                                    self.player.set_volume(self.settings.volume);
                                    self.settings.save();
                                }

                                // Регулировка колесиком мыши
                                let scroll = ui.input(|i| i.raw_scroll_delta.y);
                                if np_vol_resp.hovered() && scroll.abs() > 0.1 {
                                    let delta = scroll.signum() as f64 * 5.0;
                                    let new_vol = (self.settings.volume + delta).clamp(0.0, 100.0);
                                    self.settings.volume = new_vol;
                                    self.player.set_volume(new_vol);
                                    self.settings.save();
                                }

                                // Всплывающий вертикальный ползунок при наведении
                                let np_popup_id = ui.make_persistent_id("np_vol_popup");
                                let np_popup_rect = Rect::from_min_size(
                                    Pos2::new(
                                        np_vol_rect.center().x - 17.0,
                                        np_vol_rect.min.y - 146.0,
                                    ),
                                    Vec2::new(34.0, 140.0),
                                );
                                let np_pointer_pos = ui.input(|i| i.pointer.hover_pos());
                                let np_over_popup = np_pointer_pos.is_some_and(|p| {
                                    let panel = ui
                                        .ctx()
                                        .memory(|memory| memory.area_rect(np_popup_id))
                                        .unwrap_or(np_popup_rect);
                                    panel.union(np_vol_rect).contains(p)
                                });

                                // Open from the speaker icon, then retain while hovering the slider.
                                if np_vol_resp.hovered()
                                    || (self.np_volume_popup_open && np_over_popup)
                                {
                                    self.np_volume_popup_open = true;
                                    egui::Area::new(np_popup_id)
                                        .order(egui::Order::Foreground)
                                        .fixed_pos(np_popup_rect.min)
                                        .show(ui.ctx(), |ui| {
                                            egui::Frame::popup(ui.style())
                                                .fill(Color32::from_rgb(20, 20, 26))
                                                .stroke(Stroke::new(
                                                    1.0_f32,
                                                    Color32::from_rgb(50, 50, 65),
                                                ))
                                                .rounding(Rounding::same(8.0))
                                                .inner_margin(6.0)
                                                .show(ui, |ui| {
                                                    ui.set_width(22.0);
                                                    ui.set_height(128.0);
                                                    ui.vertical_centered(|ui| {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{:.0}%",
                                                                self.settings.volume
                                                            ))
                                                            .size(9.0)
                                                            .weak(),
                                                        );
                                                        ui.add_space(4.0);
                                                        let mut vol = self.settings.volume;
                                                        let slider = egui::Slider::new(
                                                            &mut vol,
                                                            0.0..=100.0,
                                                        )
                                                        .vertical()
                                                        .show_value(false);
                                                        if ui
                                                            .add_sized([18.0, 95.0], slider)
                                                            .changed()
                                                        {
                                                            self.player.set_volume(vol);
                                                            self.settings.volume = vol;
                                                            self.settings.save();
                                                        }
                                                    });
                                                });
                                        });
                                } else {
                                    self.np_volume_popup_open = false;
                                }
                            });
                        },
                    );
                });
            });
        let art_size = art_size.min((ui.available_height() - 65.0).max(72.0));

        // Center the entire player column inside a vertical scroll area
        // so on smaller windows or splits nothing ever disappears!
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let actual_w = ui.available_width();
                let column_width = art_size.max(230.0).min(actual_w);
                let center_offset = ((actual_w - column_width) / 2.0).max(0.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(center_offset);
                    ui.allocate_ui_with_layout(Vec2::new(column_width, 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.set_width(column_width);
                        let art_rounding = Rounding::same(28.0);

                        // Top: Centered Album Art OR Synced Lyrics in-place
                        if self.show_lyrics_in_player {
                            let lyrics_box_h = (art_size - 16.0).max(150.0);
                            let frame = egui::Frame::none()
                                .fill(Color32::from_rgb(20, 20, 26))
                                .rounding(art_rounding)
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 45, 55)))
                                .inner_margin(12.0);
                            frame.show(ui, |ui| {
                                ui.set_width(art_size - 24.0);
                                ui.set_height(lyrics_box_h);

                                    // Line 1: Header title + buttons on right
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("Текст песни")
                                                .size(13.0)
                                                .strong()
                                                .color(accent),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .button(RichText::new("Обложка").size(11.0))
                                                    .clicked()
                                                {
                                                    self.show_lyrics_in_player = false;
                                                }
                                                if ui
                                                    .button(RichText::new("↻").size(11.0))
                                                    .on_hover_text("Повторить поиск текста")
                                                    .clicked()
                                                {
                                                    self.trigger_lyrics_search(true);
                                                }
                                            },
                                        );
                                    });

                                    // Line 2: Badge showing actual animation mode
                                    if let Some(lyrics) = &self.lyrics {
                                        let anim_mode_label = match self.settings.lyrics_animation_mode {
                                            0 => "По буквам",
                                            1 => "По словам",
                                            2 => "По строчкам",
                                            _ => "Без анимации",
                                        };
                                        let badge_text = format!("{} • {}", lyrics.source_name, anim_mode_label);
                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new(badge_text)
                                                .size(10.0)
                                                .color(Color32::from_rgb(29, 185, 84)),
                                        );
                                    }

                                    // Line 3: Direct timing delay adjustment buttons in fullscreen/player
                                    if let Some(ref mut lyrics) = self.lyrics {
                                        ui.add_space(2.0);
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Сдвиг:").size(10.0).weak());
                                            if ui.small_button("-500").on_hover_text("Замедлить на 500 мс").clicked() {
                                                lyrics.offset_ms -= 500;
                                            }
                                            if ui.small_button("-100").on_hover_text("Замедлить на 100 мс").clicked() {
                                                lyrics.offset_ms -= 100;
                                            }
                                            let off_str = format!("{:+}", lyrics.offset_ms);
                                            ui.label(
                                                RichText::new(off_str)
                                                    .size(10.0)
                                                    .strong()
                                                    .color(if lyrics.offset_ms == 0 {
                                                        Color32::GRAY
                                                    } else {
                                                        accent
                                                    }),
                                            );
                                            if ui.small_button("+100").on_hover_text("Ускорить на 100 мс").clicked() {
                                                lyrics.offset_ms += 100;
                                            }
                                            if ui.small_button("+500").on_hover_text("Ускорить на 500 мс").clicked() {
                                                lyrics.offset_ms += 500;
                                            }
                                            if lyrics.offset_ms != 0 && ui.small_button("0").on_hover_text("Сбросить сдвиг").clicked() {
                                                lyrics.offset_ms = 0;
                                            }
                                            if lyrics.offset_ms != 0
                                                && ui
                                                    .small_button("💾")
                                                    .on_hover_text("Сохранить этот сдвиг в настройки как значение по умолчанию")
                                                    .clicked()
                                            {
                                                self.settings.lyrics_manual_offset_ms += lyrics.offset_ms;
                                                lyrics.offset_ms = 0;
                                                self.settings.save();
                                                self.status_message = Some(format!(
                                                    "Сдвиг текстов по умолчанию сохранён: {} мс",
                                                    self.settings.lyrics_manual_offset_ms
                                                ));
                                            }
                                        });
                                    }

                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(4.0);

                                    let scroll_max_h = (lyrics_box_h - 96.0).max(60.0);

                                    if let Some(lyrics) = &self.lyrics {
                                        if !lyrics.is_synced {
                                            ui.label(
                                                RichText::new("ℹ Доступен только несинхронизированный текст")
                                                    .size(11.0)
                                                    .color(Color32::from_rgb(180, 180, 195)),
                                            );
                                            ui.add_space(4.0);
                                            egui::ScrollArea::vertical()
                                                .max_height(scroll_max_h)
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new(&lyrics.plain)
                                                            .size(14.0)
                                                            .line_height(Some(22.0)),
                                                    );
                                                });
                                        } else {
                                            let current_time_ms = ((playback_info.position_secs * 1000.0) as i64 + lyrics.offset_ms + self.settings.lyrics_manual_offset_ms).max(0);
                                            let current_line_idx = lyrics
                                                .lines
                                                .iter()
                                                .rposition(|l| (l.start_ms as i64) <= current_time_ms);
                                            let mut seek_to_sec: Option<f64> = None;

                                            egui::ScrollArea::vertical()
                                                .max_height(scroll_max_h)
                                                .show(ui, |ui| {
                                                    for (idx, line) in lyrics.lines.iter().enumerate() {
                                                        let is_active = Some(idx) == current_line_idx;
                                                        if is_active {
                                                            let active_frame = egui::Frame::none()
                                                                .fill(Color32::from_rgba_unmultiplied(29, 185, 84, 45))
                                                                .rounding(Rounding::same(6.0))
                                                                .inner_margin(egui::Margin::symmetric(10.0, 6.0));

                                                            let main_align = if self.settings.lyrics_position == 0 {
                                                                egui::Align::Center
                                                            } else {
                                                                egui::Align::Min
                                                            };

                                                            let resp = active_frame
                                                                .show(ui, |ui| {
                                                                    ui.with_layout(
                                                                        egui::Layout::left_to_right(egui::Align::Center)
                                                                            .with_main_align(main_align),
                                                                        |ui| {
                                                                            ui.label(RichText::new("▶").size(13.0).color(accent).strong());
                                                                            self.render_active_lyric_line(ui, line, current_time_ms, 15.0);
                                                                        },
                                                                    );
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
                                                                Color32::from_rgb(120, 120, 130)
                                                            } else {
                                                                Color32::from_rgb(205, 205, 215)
                                                            };
                                                            let label_widget = egui::Label::new(
                                                                RichText::new(&line.text)
                                                                    .size(13.0)
                                                                    .color(text_color),
                                                            );
                                                            let resp = if self.settings.lyrics_position == 0 {
                                                                ui.vertical_centered(|ui| ui.add(label_widget.sense(egui::Sense::click())))
                                                                    .inner
                                                            } else {
                                                                ui.add(label_widget.sense(egui::Sense::click()))
                                                            };
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
                                let artwork = if let Some(cover) = cover.clone() {
                                    let response = Self::paint_square_cover(ui, cover.clone(), art_size, art_rounding);
                                    if let Some((painter, shape)) = &ambient {
                                        if let Some(mesh) = self.cover_ambient_mesh(ui.ctx(), &cover, playback_info.is_playing, response.rect) {
                                            painter.set(*shape, egui::Shape::mesh(mesh));
                                        }
                                    }
                                    response
                                } else {
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::splat(art_size),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, art_rounding, Color32::from_rgb(32, 32, 40));
                                    Self::draw_vector_logo(
                                        ui.painter(),
                                        Rect::from_center_size(rect.center(), Vec2::splat(64.0 * scale)),
                                        Color32::GRAY,
                                    );
                                    response
                                };
                                if current_track.is_some()
                                    && artwork.interact(egui::Sense::click())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                {
                                    self.toggle_playback();
                                }
                            }

                            // Flexible gap pushes all controls comfortably to the bottom
                            ui.add_space(flexible_gap);

                            // --- BOTTOM INTERFACE SECTION ---
                            ui.allocate_ui_with_layout(
                                Vec2::new(column_width, 48.0 * scale),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(track) = &current_track {
                                        let is_liked = self.library.is_liked(&track.id);
                                        let heart = Self::heart_button(ui, &track.id, is_liked);
                                        if Self::player_button_hint(heart, Self::heart_hint(is_liked), false).clicked() {
                                            self.library.toggle_like(track.clone());
                                        }
                                        if Self::add_to_playlist_button(ui, 30.0, accent).clicked() {
                                            self.open_playlist_picker(ui.ctx(), track.clone());
                                        }

                                        let (lyrics_color, lyrics_hint) = if self.show_lyrics_in_player {
                                            (accent, "Скрыть текст песни")
                                        } else {
                                            (Color32::from_rgb(200, 200, 215), "Показать текст песни")
                                        };
                                        let lyrics = ui.add_sized([30.0, 30.0], egui::Button::new(
                                            RichText::new("Aa").size(16.0).color(lyrics_color)
                                        ).frame(false));
                                        if Self::player_button_hint(lyrics, lyrics_hint, false).clicked() {
                                            self.show_lyrics_in_player = !self.show_lyrics_in_player;
                                        }

                                        let share = Self::draw_share_icon(ui, accent);
                                        if Self::player_button_hint(share, "Скопировать ссылку на трек", false).clicked() {
                                            let link = format!("https://music.youtube.com/watch?v={}", track.id);
                                            ui.output_mut(|o| o.copied_text = link);
                                            self.status_message = Some("Ссылка скопирована!".into());
                                        }

                                        ui.add_space(6.0);
                                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                            ui.add(egui::Label::new(RichText::new(&track.title)
                                                .size((18.0 * scale).clamp(14.0, 20.0)).strong()).truncate());
                                            ui.add(egui::Label::new(RichText::new(&track.artist)
                                                .size((13.0 * scale).clamp(10.0, 14.0)).weak()).truncate());
                                        });
                                    } else {
                                        ui.label(RichText::new("Ничего не играет").size(14.0).weak());
                                    }
                                },
                            );

                            ui.add_space(10.0);
                        },
                    );
                });
            });
    }
}
