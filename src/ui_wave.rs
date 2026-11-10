use crate::app::{AppEvent, MetrolistApp};
use crate::models::Track;
use eframe::egui::{self, Color32, RichText};
use std::sync::Arc;
use std::time::Duration;

impl MetrolistApp {
    pub(crate) fn request_wave(&mut self, resume: bool) {
        if self.wave_busy {
            return;
        }
        let mode = self.settings.wave_mode;
        let keyword = self.settings.wave_keyword.trim().to_owned();
        let playlist = self.settings.wave_playlist.clone();
        let track = self.settings.wave_track.clone();
        if mode == 1 && self.settings.youtube_playlist_hidden(&playlist) {
            self.wave_message =
                "Этот плейлист скрыт. Выбери другой или верни его в настройках библиотеки.".into();
            return;
        }
        if (mode == 0 && keyword.is_empty())
            || (mode == 1 && playlist.is_empty())
            || (mode == 2 && track.is_none())
        {
            self.wave_message = "Выбери слово, плейлист или трек для волны.".into();
            return;
        }
        self.wave_request = self.wave_request.wrapping_add(1);
        let request = self.wave_request;
        self.wave_busy = true;
        self.wave_message = "Собираю музыкальное радио…".into();
        let current = self.current_track().cloned();
        let resume_after = if resume {
            current.as_ref().map(|t| t.id.clone())
        } else {
            None
        };
        let client = Arc::clone(&self.innertube);
        let country = self.settings.content_country.clone();
        let cookie = self.settings.google_cookie.clone();
        let auth = self.settings.auth_user_index;
        let liked = self.library.liked_tracks.clone();
        let history = self.library.history.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let work = async {
                let mut seeds = match mode {
                    1 if playlist == "__liked" => liked,
                    1 => {
                        client
                            .fetch_playlist_tracks(&playlist, &cookie, auth)
                            .await?
                    }
                    2 if resume => current.into_iter().collect(),
                    2 => track.into_iter().collect(),
                    _ if keyword == "Для меня" => {
                        if liked.is_empty() {
                            history
                        } else {
                            liked
                        }
                    }
                    _ => {
                        let query = match keyword.as_str() {
                            "Спокойствие" => "спокойная музыка",
                            "Энергия" => "энергичная музыка",
                            "Для работы" => "музыка для работы",
                            "Ночь" => "ночной плейлист музыка",
                            other => other,
                        };
                        client.search(query, &country, &cookie, auth).await?
                    }
                };
                if seeds.is_empty() {
                    return Err("Не найдены треки для основы волны. Выбери другой запрос, трек или непустой плейлист.".into());
                }
                let offset = request.wrapping_sub(1) as usize % seeds.len();
                if mode == 1 {
                    seeds.rotate_left(offset);
                }
                let mut tracks = Vec::new();
                let mut failures = Vec::new();
                for seed in seeds.iter().take(if mode == 2 { 1 } else { 2 }) {
                    match client.fetch_radio(&seed.id, &country).await {
                        Ok(radio) => tracks.extend(radio),
                        Err(error) => failures.push(error),
                    }
                }
                if tracks.is_empty() {
                    return Err(failures
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| "Станция не вернула треков.".into()));
                }
                let mut seen = std::collections::HashSet::new();
                // Keep the seed first for an explicit start; skip it for continuation.
                tracks.retain(|t| {
                    (!resume || resume_after.as_ref() != Some(&t.id)) && seen.insert(t.id.clone())
                });
                Ok(tracks)
            };
            let result: Result<Vec<Track>, String> =
                tokio::time::timeout(Duration::from_secs(65), work)
                    .await
                    .unwrap_or_else(|_| {
                        Err("YouTube Music долго не отвечает. Попробуй снова.".into())
                    });
            let _ = tx.send(AppEvent::WaveLoaded {
                request,
                result,
                resume_after,
            });
        });
    }

    fn search_wave_seed(&mut self) {
        let query = self.wave_search_query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.wave_search_request = self.wave_search_request.wrapping_add(1);
        let request = self.wave_search_request;
        self.wave_search_busy = true;
        self.wave_search_results.clear();
        self.wave_search_message.clear();
        let client = Arc::clone(&self.innertube);
        let country = self.settings.content_country.clone();
        let cookie = self.settings.google_cookie.clone();
        let auth = self.settings.auth_user_index;
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(25),
                client.search(&query, &country, &cookie, auth),
            )
            .await
            .unwrap_or_else(|_| Err("Поиск не ответил вовремя. Попробуй ещё раз.".into()));
            let _ = tx.send(AppEvent::WaveSearchLoaded { request, result });
        });
    }

    pub(crate) fn render_wave_tab(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent_color();
        let mut changed = false;
        let mut launch = false;
        egui::ScrollArea::vertical()
            .id_salt("wave_page")
            .show(ui, |ui| {
                egui::Frame::none()
                    .fill(accent.gamma_multiply(0.18))
                    .rounding(22.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.heading("Своя волна");
                        ui.label("Музыка под твой настрой");
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            for (mode, label) in ["Настроение и слово", "Мой плейлист", "Один трек"]
                                .iter()
                                .enumerate()
                            {
                                changed |= ui
                                    .selectable_value(&mut self.settings.wave_mode, mode, *label)
                                    .changed();
                            }
                        });
                    });
                ui.add_space(14.0);
                match self.settings.wave_mode {
                    1 => {
                        ui.label("Выбери основу — радио подберёт музыку по песням плейлиста.");
                        egui::ComboBox::from_id_salt("wave_playlist")
                            .width(ui.available_width().min(320.0))
                            .selected_text(if self.settings.wave_playlist == "__liked" {
                                "Любимые треки".to_string()
                            } else {
                                self.playlists
                                    .iter()
                                    .find(|p| {
                                        p.id == self.settings.wave_playlist
                                            && !self.settings.youtube_playlist_hidden(&p.id)
                                    })
                                    .map(|p| p.title.clone())
                                    .unwrap_or_else(|| "Выбрать плейлист".into())
                            })
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.settings.wave_playlist,
                                        "__liked".into(),
                                        "Любимые треки",
                                    )
                                    .changed();
                                for playlist in &self.playlists {
                                    if self.settings.youtube_playlist_hidden(&playlist.id) {
                                        continue;
                                    }
                                    changed |= ui
                                        .selectable_value(
                                            &mut self.settings.wave_playlist,
                                            playlist.id.clone(),
                                            &playlist.title,
                                        )
                                        .changed();
                                }
                            });
                        if ui.button("Обновить мои плейлисты").clicked() {
                            self.sync_playlists();
                        }
                        if self.is_loading_playlists {
                            ui.spinner();
                        }
                    }
                    2 => {
                        ui.label("Найди песню и выбери её для запуска радио.");
                        ui.horizontal_wrapped(|ui| {
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.wave_search_query)
                                    .hint_text("Название трека или исполнитель")
                                    .desired_width((ui.available_width() - 90.0).max(120.0)),
                            );
                            let enter =
                                edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui.button("Найти").clicked() || enter {
                                self.search_wave_seed();
                            }
                        });
                        if self.wave_search_busy {
                            ui.spinner();
                        }
                        if !self.wave_search_message.is_empty() {
                            ui.label(&self.wave_search_message);
                        }
                        let mut selected = None;
                        egui::ScrollArea::vertical()
                            .id_salt("wave_search_results")
                            .max_height(230.0)
                            .show(ui, |ui| {
                                for track in &self.wave_search_results {
                                    let is_selected = self
                                        .settings
                                        .wave_track
                                        .as_ref()
                                        .is_some_and(|t| t.id == track.id);
                                    let response = ui.add(
                                        egui::Button::new(format!(
                                            "{} — {}",
                                            track.artist, track.title
                                        ))
                                        .selected(is_selected)
                                        .wrap(),
                                    );
                                    if response.clicked() {
                                        selected = Some(track.clone());
                                    }
                                }
                            });
                        if ui.button("Использовать текущую песню").clicked()
                        {
                            selected = self.current_track().cloned();
                        }
                        if let Some(track) = selected {
                            self.settings.wave_track = Some(track);
                            changed = true;
                        }
                        if let Some(track) = &self.settings.wave_track {
                            ui.label(
                                RichText::new(format!(
                                    "Основа волны: {} — {}",
                                    track.artist, track.title
                                ))
                                .color(accent),
                            );
                        }
                    }
                    _ => {
                        ui.label("Выбери настроение, жанр или введи своё слово.");
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.wave_keyword)
                                    .hint_text("Например: инди, прогулка, спокойствие")
                                    .desired_width(ui.available_width().min(400.0)),
                            )
                            .changed();
                        ui.horizontal_wrapped(|ui| {
                            for word in [
                                "Для меня",
                                "Рок",
                                "Электроника",
                                "Джаз",
                                "Спокойствие",
                                "Энергия",
                                "Для работы",
                                "Ночь",
                            ] {
                                if ui
                                    .add(
                                        egui::Button::new(word)
                                            .selected(self.settings.wave_keyword == word)
                                            .rounding(16.0),
                                    )
                                    .clicked()
                                {
                                    self.settings.wave_keyword = word.into();
                                    changed = true;
                                }
                            }
                        });
                    }
                }
                ui.add_space(12.0);
                launch = ui
                    .add_enabled(
                        !self.wave_busy,
                        egui::Button::new(
                            RichText::new("▶ Запустить волну")
                                .color(Color32::WHITE)
                                .size(17.0),
                        )
                        .fill(accent)
                        .rounding(24.0)
                        .min_size(egui::vec2(210.0, 46.0)),
                    )
                    .clicked();
                ui.label("Запустит первый трек волны. Остальные добавятся в очередь.");
                if ui
                    .checkbox(
                        &mut self.settings.wave_auto,
                        "Продолжать волну после конца очереди",
                    )
                    .changed()
                {
                    self.settings.save();
                }
                if self.wave_busy {
                    ui.spinner();
                }
                if !self.wave_message.is_empty() {
                    ui.label(&self.wave_message);
                }
            });
        if changed {
            self.wave_request = self.wave_request.wrapping_add(1);
            self.wave_busy = false;
            self.wave_message.clear();
            self.settings.save();
        }
        if launch {
            self.request_wave(false);
        }
    }
}
