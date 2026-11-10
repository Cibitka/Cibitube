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

    pub(crate) fn render_lyrics_tab(&mut self, ui: &mut egui::Ui, current_time: f64) {
        let accent = self.accent_color();

        let is_narrow = ui.available_width() < 620.0;
        let anim_mode_label = match self.settings.lyrics_animation_mode {
            0 => "По буквам",
            1 => "По словам",
            2 => "По строчкам",
            _ => "Без анимации",
        };

        if is_narrow {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Текст песни").size(18.0).strong());
                if let Some(lyrics) = &self.lyrics {
                    let badge_text = format!("{} • {}", lyrics.source_name, anim_mode_label);
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
                    let badge_text = format!("{} • {}", lyrics.source_name, anim_mode_label);
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

        // Блок ручной регулировки задержки синхронизации
        if let Some(ref mut lyrics) = self.lyrics {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Задержка текста:").size(11.0).weak());
                if ui.small_button("-500мс").clicked() {
                    lyrics.offset_ms -= 500;
                }
                if ui.small_button("-100мс").clicked() {
                    lyrics.offset_ms -= 100;
                }
                let off_str = format!("{:+}", lyrics.offset_ms);
                ui.label(RichText::new(off_str).size(11.0).strong().color(
                    if lyrics.offset_ms == 0 {
                        Color32::GRAY
                    } else {
                        accent
                    },
                ));
                if ui.small_button("+100мс").clicked() {
                    lyrics.offset_ms += 100;
                }
                if ui.small_button("+500мс").clicked() {
                    lyrics.offset_ms += 500;
                }
                if lyrics.offset_ms != 0 && ui.small_button("0 (сброс)").clicked() {
                    lyrics.offset_ms = 0;
                }
                if lyrics.offset_ms != 0
                    && ui
                        .small_button("💾 Сохранить по умолч.")
                        .on_hover_text("Сохранить этот сдвиг в настройки как значение по умолчанию для всех песен")
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

        let current_time_ms = ((current_time * 1000.0) as i64
            + lyrics.offset_ms
            + self.settings.lyrics_manual_offset_ms)
            .max(0);
        let current_line_idx = lyrics
            .lines
            .iter()
            .rposition(|l| (l.start_ms as i64) <= current_time_ms);
        let mut seek_to_sec: Option<f64> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (idx, line) in lyrics.lines.iter().enumerate() {
                let is_active = Some(idx) == current_line_idx;
                if is_active {
                    let active_frame = egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(29, 185, 84, 45))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0));

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
                                    ui.label(RichText::new("▶").size(15.0).color(accent).strong());
                                    self.render_active_lyric_line(ui, line, current_time_ms, 18.0);
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
}
