use crate::app::MetrolistApp;
use crate::models::LyricsLine;
use eframe::egui::{self, Color32, RichText, Rounding};

impl MetrolistApp {
    pub(crate) fn render_active_lyric_line(
        &self,
        ui: &mut egui::Ui,
        line: &LyricsLine,
        current_ms: i64,
        font_size: f32,
    ) {
        let mut job = egui::text::LayoutJob::default();
        if self.settings.lyrics_position == 0 {
            job.halign = egui::Align::Center;
        }
        job.wrap.max_width = ui.available_width().max(100.0);

        let color_sung = Color32::WHITE;
        let color_unsung = Color32::from_rgb(175, 175, 185); // Solid readable light silver!

        if let Some(ref words) = line.words {
            for word in words {
                let is_fully_sung = (current_ms as u64) >= word.end_ms;
                let is_unsung = (current_ms as u64) < word.start_ms;

                if is_fully_sung {
                    job.append(
                        &word.text,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(font_size),
                            color: color_sung,
                            ..Default::default()
                        },
                    );
                } else if is_unsung {
                    job.append(
                        &word.text,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(font_size),
                            color: color_unsung,
                            ..Default::default()
                        },
                    );
                } else {
                    match self.settings.lyrics_animation_mode {
                        // 0 = По буквам (караоке-заливка внутри текущего слова)
                        0 => {
                            let dur = (word.end_ms - word.start_ms).max(1);
                            let frac = ((current_ms as u64 - word.start_ms) as f32 / dur as f32)
                                .clamp(0.0, 1.0);
                            let chars: Vec<char> = word.text.chars().collect();
                            let sung_len =
                                ((frac * chars.len() as f32).round() as usize).min(chars.len());
                            let sung_part: String = chars[..sung_len].iter().collect();
                            let unsung_part: String = chars[sung_len..].iter().collect();

                            if !sung_part.is_empty() {
                                job.append(
                                    &sung_part,
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::proportional(font_size),
                                        color: color_sung,
                                        ..Default::default()
                                    },
                                );
                            }
                            if !unsung_part.is_empty() {
                                job.append(
                                    &unsung_part,
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::proportional(font_size),
                                        color: color_unsung,
                                        ..Default::default()
                                    },
                                );
                            }
                        }
                        // 1 = По словам (текущее слово сразу белое)
                        _ => {
                            job.append(
                                &word.text,
                                0.0,
                                egui::TextFormat {
                                    font_id: egui::FontId::proportional(font_size),
                                    color: color_sung,
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }
        } else {
            // Построчный LRC: интерполируем прогресс строки по времени,
            // чтобы анимация караоке («По буквам», «По словам», «По строчкам»)
            // работала для всех песен без исключения!
            let line_dur = line
                .end_ms
                .map(|e| e.saturating_sub(line.start_ms))
                .unwrap_or(4000)
                .max(600);
            let elapsed = (current_ms as u64).saturating_sub(line.start_ms);
            let progress = (elapsed as f32 / line_dur as f32).clamp(0.0, 1.0);

            match self.settings.lyrics_animation_mode {
                // 0 = По буквам: прогрессивная подсветка букв строки
                0 => {
                    let chars: Vec<char> = line.text.chars().collect();
                    let sung_count =
                        ((progress * chars.len() as f32).round() as usize).min(chars.len());
                    let sung: String = chars[..sung_count].iter().collect();
                    let unsung: String = chars[sung_count..].iter().collect();

                    if !sung.is_empty() {
                        job.append(
                            &sung,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(font_size),
                                color: color_sung,
                                ..Default::default()
                            },
                        );
                    }
                    if !unsung.is_empty() {
                        job.append(
                            &unsung,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(font_size),
                                color: color_unsung,
                                ..Default::default()
                            },
                        );
                    }
                }
                // 1 = По словам: постепенная подсветка слов строки по очереди
                1 => {
                    let words: Vec<&str> = line.text.split_inclusive(' ').collect();
                    let sung_count =
                        ((progress * words.len() as f32).floor() as usize).min(words.len());
                    let sung: String = words[..sung_count].concat();
                    let unsung: String = words[sung_count..].concat();

                    if !sung.is_empty() {
                        job.append(
                            &sung,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(font_size),
                                color: color_sung,
                                ..Default::default()
                            },
                        );
                    }
                    if !unsung.is_empty() {
                        job.append(
                            &unsung,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(font_size),
                                color: color_unsung,
                                ..Default::default()
                            },
                        );
                    }
                }
                // 2 = По строчкам: вся строка целиком ярко подсвечена
                2 => {
                    job.append(
                        &line.text,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(font_size),
                            color: Color32::WHITE,
                            ..Default::default()
                        },
                    );
                }
                // _ = Без анимации
                _ => {
                    job.append(
                        &line.text,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(font_size),
                            color: color_sung,
                            ..Default::default()
                        },
                    );
                }
            }
        }
        ui.label(job);
    }

    pub(crate) fn render_highlighted_lyric_line(
        &self,
        ui: &mut egui::Ui,
        line: &LyricsLine,
        current_ms: i64,
        font_size: f32,
    ) -> egui::Response {
        let draw = |ui: &mut egui::Ui| {
            let frame = egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(29, 185, 84, 65))
                .rounding(Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(10.0, 4.0));
            let rect = frame
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("▶")
                                .size(font_size - 2.0)
                                .color(self.accent_color()),
                        );
                        self.render_active_lyric_line(ui, line, current_ms, font_size);
                    });
                })
                .response;
            ui.interact(rect.rect, rect.id.with("seek-lyric"), egui::Sense::click())
        };
        if self.settings.lyrics_position == 0 {
            ui.vertical_centered(draw).inner
        } else {
            draw(ui)
        }
    }

    pub(crate) fn scroll_to_active_lyric(
        &self,
        ui: &egui::Ui,
        track_id: Option<&str>,
        index: usize,
        response: &egui::Response,
    ) {
        let id = ui.make_persistent_id("lyrics-last-scroll-target");
        let target = egui::Id::new((track_id, index));
        let changed = ui.ctx().data_mut(|data| {
            if !self.settings.lyrics_auto_scroll {
                data.remove::<egui::Id>(id);
                return false;
            }
            if data.get_temp::<egui::Id>(id) == Some(target) {
                return false;
            }
            data.insert_temp(id, target);
            true
        });
        if changed {
            response.scroll_to_me(Some(egui::Align::Center));
        }
    }

    pub(crate) fn lyric_timing_badge(lyrics: &crate::models::Lyrics, mode: usize) -> String {
        let timing = if !lyrics.is_synced {
            "Без синхронизации"
        } else if lyrics.is_word_synced() {
            match mode {
                0 => "Тайминги слов • подсветка букв",
                1 => "По словам",
                _ => "По строкам",
            }
        } else if mode <= 1 {
            "Тайминги строк • караоке приблизительное"
        } else {
            "По строкам"
        };
        format!("{} • {timing}", lyrics.source_name)
    }

    fn lyric_track_offset(&self) -> i64 {
        self.current_track()
            .and_then(|track| self.library.lyric_track_offsets.get(&track.id))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn lyric_total_offset(&self, lyrics: &crate::models::Lyrics) -> i64 {
        lyrics
            .offset_ms
            .saturating_add(self.settings.lyrics_manual_offset_ms)
            .saturating_add(self.lyric_track_offset())
    }

    pub(crate) fn save_lyric_track_offset(&mut self, offset: i64) {
        let Some(id) = self.current_track().map(|track| track.id.clone()) else {
            return;
        };
        if offset == 0 {
            self.library.lyric_track_offsets.remove(&id);
        } else {
            self.library
                .lyric_track_offsets
                .insert(id, offset.clamp(-300_000, 300_000));
        }
        self.library.save();
    }

    pub(crate) fn render_lyric_timing_controls(&mut self, ui: &mut egui::Ui) {
        if self.current_track().is_none() {
            return;
        }
        let original = self.lyric_track_offset();
        let mut seconds = original as f64 / 1000.0;
        ui.horizontal_wrapped(|ui| {
            ui.label("Поправка трека").on_hover_text(
                "Прибавляется к общему сдвигу и автоматически восстанавливается для этого трека.",
            );
            ui.spacing_mut().button_padding.x = 5.0;
            let control_size = egui::vec2(72.0, 26.0);
            if ui
                .add_sized(control_size, egui::Button::new("−0,5"))
                .clicked()
            {
                seconds -= 0.5;
            }
            ui.add_sized(
                control_size,
                egui::DragValue::new(&mut seconds)
                    .speed(0.1)
                    .range(-300.0..=300.0)
                    .min_decimals(1)
                    .max_decimals(3)
                    .suffix(" с"),
            )
            .on_hover_text("Если текст отстаёт, введите положительный сдвиг. Например, +45 с.");
            if ui
                .add_sized(control_size, egui::Button::new("+0,5"))
                .clicked()
            {
                seconds += 0.5;
            }
            if ui
                .add_sized(control_size, egui::Button::new("Сброс"))
                .on_hover_text("Убрать поправку трека и оставить общий сдвиг.")
                .clicked()
            {
                seconds = 0.0;
            }
        });
        let offset = ((seconds * 1000.0).round() as i64).clamp(-300_000, 300_000);
        if offset != original {
            self.save_lyric_track_offset(offset);
        }
        let base = self.settings.lyrics_manual_offset_ms;
        ui.label(
            RichText::new(format!(
                "Для всех: {base:+} мс  ·  Итог: {:+} мс",
                base.saturating_add(offset)
            )).small(),
        ).on_hover_text("Сумма общего сдвига и поправки трека. Метка [offset] в файле текста учитывается отдельно.");
    }

    pub(crate) fn render_lyrics_tab(&mut self, ui: &mut egui::Ui, current_time: f64) {
        let accent = self.accent_color();

        let is_narrow = ui.available_width() < 620.0;

        if is_narrow {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Текст песни").size(18.0).strong());
                if let Some(lyrics) = &self.lyrics {
                    let badge_text =
                        Self::lyric_timing_badge(lyrics, self.settings.lyrics_animation_mode);
                    ui.label(
                        RichText::new(badge_text)
                            .size(11.0)
                            .color(accent)
                            .background_color(Color32::from_rgba_unmultiplied(29, 185, 84, 25)),
                    );
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let current_source_name = match self.settings.lyrics_source_mode {
                    0 => "Авто",
                    1 => "LRCLIB",
                    2 => "QQ",
                    3 => "TTML",
                    4 => "KuGou",
                    5 => "LrcMux",
                    _ => "Авто",
                };
                egui::ComboBox::from_id_salt("lyrics_source_combo_tab")
                    .selected_text(current_source_name)
                    .show_ui(ui, |ui| {
                        let options = [
                            (0, "Автоматически"),
                            (1, "LRCLIB"),
                            (2, "Better Lyrics (QQ)"),
                            (3, "Better Lyrics (TTML)"),
                            (4, "Better Lyrics (KuGou)"),
                            (5, "LrcMux"),
                        ];
                        for (val, label) in options {
                            if ui
                                .selectable_label(self.settings.lyrics_source_mode == val, label)
                                .clicked()
                            {
                                self.settings.lyrics_source_mode = val;
                                self.settings.save();
                                self.trigger_lyrics_search(false);
                            }
                        }
                    });

                if ui.button(RichText::new("↻ Повторить").size(11.0)).clicked() {
                    self.trigger_lyrics_search(true);
                }

                if ui
                    .button(RichText::new("Загрузить файл").size(11.0))
                    .clicked()
                {
                    self.pick_and_load_local_lyrics_file();
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Текст песни").size(18.0).strong());
                if let Some(lyrics) = &self.lyrics {
                    let badge_text =
                        Self::lyric_timing_badge(lyrics, self.settings.lyrics_animation_mode);
                    ui.label(
                        RichText::new(badge_text)
                            .size(11.0)
                            .color(accent)
                            .background_color(Color32::from_rgba_unmultiplied(29, 185, 84, 25)),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let current_source_name = match self.settings.lyrics_source_mode {
                        0 => "Авто",
                        1 => "LRCLIB",
                        2 => "QQ",
                        3 => "TTML",
                        4 => "KuGou",
                        5 => "LrcMux",
                        _ => "Авто",
                    };
                    egui::ComboBox::from_id_salt("lyrics_source_combo_tab")
                        .selected_text(current_source_name)
                        .show_ui(ui, |ui| {
                            let options = [
                                (0, "Автоматически"),
                                (1, "LRCLIB"),
                                (2, "Better Lyrics (QQ)"),
                                (3, "Better Lyrics (TTML)"),
                                (4, "Better Lyrics (KuGou)"),
                                (5, "LrcMux"),
                            ];
                            for (val, label) in options {
                                if ui
                                    .selectable_label(
                                        self.settings.lyrics_source_mode == val,
                                        label,
                                    )
                                    .clicked()
                                {
                                    self.settings.lyrics_source_mode = val;
                                    self.settings.save();
                                    self.trigger_lyrics_search(false);
                                }
                            }
                        });

                    if ui.button(RichText::new("↻ Повторить").size(11.0)).clicked() {
                        self.trigger_lyrics_search(true);
                    }

                    if ui
                        .button(RichText::new("Загрузить файл").size(11.0))
                        .clicked()
                    {
                        self.pick_and_load_local_lyrics_file();
                    }
                });
            });
        }

        self.render_lyric_timing_controls(ui);

        ui.add_space(10.0);

        let Some(lyrics) = &self.lyrics else {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    RichText::new("Текст песни не найден или трек ещё не воспроизводится").weak(),
                );
            });
            return;
        };

        crate::pet::register_surface(
            ui.ctx(),
            egui::Id::new("pet-lyrics-panel"),
            ui.available_rect_before_wrap().intersect(ui.clip_rect()),
            false,
        );
        if !lyrics.is_synced {
            ui.label(
                RichText::new("ℹ Для этого трека доступен только несинхронизированный текст")
                    .size(12.0)
                    .color(Color32::from_rgb(180, 180, 195)),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(
                    RichText::new(&lyrics.plain)
                        .size(15.0)
                        .line_height(Some(24.0)),
                );
            });
            return;
        }

        let offset = self.lyric_total_offset(lyrics);
        let current_time_ms = ((current_time * 1000.0) as i64).saturating_add(offset);
        let current_line_idx = lyrics
            .lines
            .iter()
            .rposition(|l| (l.start_ms as i64) <= current_time_ms);
        let mut seek_to_sec: Option<f64> = None;
        let track_id = self.current_track().map(|track| track.id.as_str());

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (idx, line) in lyrics.lines.iter().enumerate() {
                let is_active = Some(idx) == current_line_idx;
                let response = if is_active {
                    let resp = self.render_highlighted_lyric_line(ui, line, current_time_ms, 18.0);

                    if resp.clicked() {
                        seek_to_sec = Some((line.seconds - offset as f64 / 1000.0).max(0.0));
                    }
                    self.scroll_to_active_lyric(ui, track_id, idx, &resp);
                    resp
                } else {
                    let is_past = current_line_idx.is_some_and(|cur| idx < cur);
                    let text_color = if is_past {
                        Color32::from_rgb(130, 130, 140)
                    } else {
                        Color32::from_rgb(215, 215, 225)
                    };
                    let label_widget =
                        egui::Label::new(RichText::new(&line.text).size(15.0).color(text_color));
                    let resp = if self.settings.lyrics_position == 0 {
                        ui.vertical_centered(|ui| ui.add(label_widget.sense(egui::Sense::click())))
                            .inner
                    } else {
                        ui.add(label_widget.sense(egui::Sense::click()))
                    };
                    if resp.clicked() {
                        seek_to_sec = Some((line.seconds - offset as f64 / 1000.0).max(0.0));
                    }
                    resp
                };
                crate::pet::register_lyric_row(ui, track_id, idx, &response, is_active, None);
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
}
