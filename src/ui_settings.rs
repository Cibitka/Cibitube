use crate::app::{AppEvent, MetrolistApp};
use crate::browser_auth::BrowserAuth;
use crate::discord::DiscordStatus;
use eframe::egui::{self, pos2, Color32, Rect, RichText, Rounding, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlagKind {
    Russia,
    UK,
}

impl MetrolistApp {
    fn draw_flag_button(
        ui: &mut egui::Ui,
        label: &str,
        is_selected: bool,
        flag: FlagKind,
    ) -> egui::Response {
        let desired_size = Vec2::new(76.0, 26.0);
        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        ui.ctx()
            .data_mut(|data| data.insert_temp(resp.id.with("no_button_feedback"), true));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter().with_clip_rect(rect);
            let rounding_r = 6.0_f32;

            match flag {
                FlagKind::Russia => {
                    let h_third = rect.height() / 3.0;
                    // White stripe (top rounded)
                    let r1 = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + h_third));
                    painter.rect_filled(
                        r1,
                        Rounding {
                            nw: rounding_r,
                            ne: rounding_r,
                            sw: 0.0,
                            se: 0.0,
                        },
                        Color32::from_rgb(255, 255, 255),
                    );
                    // Blue stripe (#0039A6, middle square)
                    let r2 = Rect::from_min_max(
                        pos2(rect.min.x, rect.min.y + h_third),
                        pos2(rect.max.x, rect.min.y + 2.0 * h_third),
                    );
                    painter.rect_filled(r2, Rounding::ZERO, Color32::from_rgb(0, 57, 166));
                    // Red stripe (#D52B1E, bottom rounded)
                    let r3 =
                        Rect::from_min_max(pos2(rect.min.x, rect.min.y + 2.0 * h_third), rect.max);
                    painter.rect_filled(
                        r3,
                        Rounding {
                            nw: 0.0,
                            ne: 0.0,
                            sw: rounding_r,
                            se: rounding_r,
                        },
                        Color32::from_rgb(213, 43, 30),
                    );
                }
                FlagKind::UK => {
                    painter.rect_filled(rect, Rounding::ZERO, Color32::from_rgb(1, 33, 105));

                    let p_tl = rect.left_top();
                    let p_br = rect.right_bottom();
                    let p_bl = rect.left_bottom();
                    let p_tr = rect.right_top();

                    // White diagonals
                    let stroke_white_diag = Stroke::new(3.2_f32, Color32::WHITE);
                    painter.line_segment([p_tl, p_br], stroke_white_diag);
                    painter.line_segment([p_bl, p_tr], stroke_white_diag);

                    // Red diagonals
                    let stroke_red_diag = Stroke::new(1.6_f32, Color32::from_rgb(200, 16, 46));
                    painter.line_segment([p_tl, p_br], stroke_red_diag);
                    painter.line_segment([p_bl, p_tr], stroke_red_diag);

                    // White Cross
                    let white_h =
                        Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 6.0));
                    painter.rect_filled(white_h, Rounding::ZERO, Color32::WHITE);
                    let white_v =
                        Rect::from_center_size(rect.center(), Vec2::new(8.0, rect.height()));
                    painter.rect_filled(white_v, Rounding::ZERO, Color32::WHITE);

                    // Red Cross
                    let red_h = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 3.2));
                    painter.rect_filled(red_h, Rounding::ZERO, Color32::from_rgb(200, 16, 46));
                    let red_v =
                        Rect::from_center_size(rect.center(), Vec2::new(4.5, rect.height()));
                    painter.rect_filled(red_v, Rounding::ZERO, Color32::from_rgb(200, 16, 46));
                }
            }

            let pill_rect = Rect::from_center_size(rect.center(), Vec2::new(34.0, 17.0));
            let pill_fill = if is_selected || resp.hovered() {
                Color32::from_rgb(12, 12, 18)
            } else {
                Color32::from_rgba_unmultiplied(12, 12, 18, 190)
            };
            painter.rect_filled(pill_rect, Rounding::same(4.0), pill_fill);

            // Text: RU or EN
            painter.text(
                pill_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        resp
    }

    pub(crate) fn render_welcome_tour(&mut self, ctx: &egui::Context) {
        let Some(step) = self.welcome_step else {
            return;
        };
        let en = self.settings.ui_language == "en";
        let steps = if en {
            [
                ("Welcome to Cibitube", "Find a track in |Home|, press |Play|, and add tracks to the |Queue|. Expand the mini player to see the cover, lyrics and playback controls.", None),
                ("Connect your account", "Sign in to |YouTube Music| in |Firefox| first. Then open |Settings → Accounts| and choose the |Firefox| sign-in button. Once connected, you can select your profile and sync playlists.", Some(1)),
                ("Make the player yours", "In |Navigation|, choose which sidebar tabs to show. In |Audio|, you can |normalize loudness| and |skip long silences|. Use |Downloads| to choose where audio and cover art are saved.", Some(13)),
            ]
        } else {
            [
                (crate::i18n::text("Добро пожаловать в Cibitube"), crate::i18n::text("Найди трек на вкладке |«Главная»|, нажми |«Играть»| и добавь другие песни в |очередь|. Разверни мини-плеер, чтобы увидеть обложку, текст песни и управление воспроизведением."), None),
                (crate::i18n::text("Вход в аккаунт"), crate::i18n::text("Сначала войди в |YouTube Music| в |Firefox|. Затем открой |«Настройки → Аккаунты»| и нажми |«Войти через Firefox в один клик»|. После подключения можно выбрать профиль и синхронизировать плейлисты."), Some(1)),
                (crate::i18n::text("Настрой плеер под себя"), crate::i18n::text("В разделе |«Навигация»| можно скрыть лишние вкладки. В разделе |«Звук»| можно включить |выравнивание громкости| и |пропуск тишины|. Папка для аудио и обложек выбирается в разделе |«Хранилище»|."), Some(13)),
            ]
        };
        let (title, description, section) = steps[step];
        egui::Modal::new(egui::Id::new("welcome_tour"))
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(23, 26, 29))
                    .rounding(20.0)
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                ui.set_width(440.0);
                *ui.visuals_mut() = egui::Visuals::dark();
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                let green = Color32::from_rgb(78, 224, 167);
                ui.label(RichText::new(format!("{} / {}", step + 1, steps.len())).color(green));
                ui.heading(title);
                ui.add_space(8.0);
                let mut text = egui::text::LayoutJob::default();
                for (index, segment) in description.split('|').enumerate() {
                    let color = if index % 2 == 0 {
                        Color32::WHITE
                    } else {
                        green
                    };
                    text.append(
                        segment,
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(15.0), color),
                    );
                }
                ui.label(text);
                ui.add_space(14.0);

                if let Some(section) = section {
                    let label = match (en, section) {
                        (true, 1) => "Open Accounts",
                        (false, 1) => crate::i18n::text("Открыть аккаунты"),
                        (true, _) => "Open Navigation",
                        (false, _) => crate::i18n::text("Открыть навигацию"),
                    };
                    if ui.button(label).clicked() {
                        self.active_tab = crate::models::ActiveTab::Settings;
                        self.settings_menu_tab = section;
                        self.finish_welcome_tour();
                        return;
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if step > 0
                        && ui
                            .button(if en {
                                "Back"
                            } else {
                                crate::i18n::text("Назад")
                            })
                            .clicked()
                    {
                        self.welcome_step = Some(step - 1);
                    }
                    if step + 1 < steps.len() {
                        if ui
                            .button(if en {
                                "Next"
                            } else {
                                crate::i18n::text("Далее")
                            })
                            .clicked()
                        {
                            self.welcome_step = Some(step + 1);
                        }
                        if ui
                            .button(if en {
                                "Skip"
                            } else {
                                crate::i18n::text("Пропустить")
                            })
                            .clicked()
                        {
                            self.finish_welcome_tour();
                        }
                    } else if ui
                        .button(if en {
                            "Done"
                        } else {
                            crate::i18n::text("Готово")
                        })
                        .clicked()
                    {
                        self.finish_welcome_tour();
                    }
                });
            });
    }

    fn finish_welcome_tour(&mut self) {
        self.welcome_step = None;
        if !self.settings.welcome_tour_seen {
            self.settings.welcome_tour_seen = true;
            self.settings.save();
        }
    }

    pub(crate) fn render_settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let accent = self.accent_color();
        let strings = crate::i18n::tr(&self.settings.ui_language);

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(strings.nav_settings).size(22.0).strong());
            ui.label(
                RichText::new(concat!("Cibitube v", env!("CARGO_PKG_VERSION")))
                    .size(12.0)
                    .weak(),
            );

            let tour_label = if self.settings.ui_language == "en" {
                "Quick tour"
            } else {
                crate::i18n::text("Знакомство с Cibitube")
            };
            if ui.button(tour_label).clicked() {
                self.welcome_step = Some(0);
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(strings.settings_language_label).size(12.0));
            let is_ru = self.settings.ui_language == "ru";
            if Self::draw_flag_button(ui, "RU", is_ru, FlagKind::Russia).clicked() {
                self.settings.ui_language = "ru".to_string();
                crate::i18n::set_language("ru");
                self.settings.save();
            }
            let is_en = self.settings.ui_language == "en";
            if Self::draw_flag_button(ui, "EN", is_en, FlagKind::UK).clicked() {
                self.settings.ui_language = "en".to_string();
                crate::i18n::set_language("en");
                self.settings.save();
            }
        });
        ui.add_space(10.0);

        // 1. Быстрые фильтры меню (разбивка на удобные категории)
        let categories = [
            (
                0,
                if self.settings.ui_language == "en" {
                    "All"
                } else {
                    crate::i18n::text("Все меню")
                },
            ),
            (
                1,
                if self.settings.ui_language == "en" {
                    "Accounts"
                } else {
                    crate::i18n::text("Аккаунты")
                },
            ),
            (
                2,
                if self.settings.ui_language == "en" {
                    "Library"
                } else {
                    crate::i18n::text("Библиотека")
                },
            ),
            (
                3,
                if self.settings.ui_language == "en" {
                    "Theme"
                } else {
                    crate::i18n::text("Оформление")
                },
            ),
            (
                4,
                if self.settings.ui_language == "en" {
                    "Player"
                } else {
                    crate::i18n::text("Плеер")
                },
            ),
            (
                5,
                if self.settings.ui_language == "en" {
                    "Lyrics"
                } else {
                    crate::i18n::text("Текст песни")
                },
            ),
            (
                6,
                if self.settings.ui_language == "en" {
                    "Window"
                } else {
                    crate::i18n::text("Окно и Вид")
                },
            ),
            (
                13,
                if self.settings.ui_language == "en" {
                    "Navigation"
                } else {
                    crate::i18n::text("Навигация")
                },
            ),
            (
                7,
                if self.settings.ui_language == "en" {
                    "Audio"
                } else {
                    crate::i18n::text("Звук")
                },
            ),
            (8, "Discord"),
            (
                14,
                if self.settings.ui_language == "en" {
                    "Pet"
                } else {
                    crate::i18n::text("Питомец")
                },
            ),
            (
                9,
                if self.settings.ui_language == "en" {
                    "Downloads"
                } else {
                    crate::i18n::text("Хранилище")
                },
            ),
            (
                10,
                if self.settings.ui_language == "en" {
                    "Updates"
                } else {
                    crate::i18n::text("Обновления")
                },
            ),
            (
                11,
                if self.settings.ui_language == "en" {
                    "Sync"
                } else {
                    crate::i18n::text("Синхронизация")
                },
            ),
            (
                12,
                if self.settings.ui_language == "en" {
                    "Support"
                } else {
                    crate::i18n::text("Поддержать")
                },
            ),
        ];

        let visuals = ui.visuals();
        let surface = visuals.widgets.inactive.weak_bg_fill;
        let text_color = visuals.text_color();
        let order = [7, 4, 3, 13, 5, 2, 9, 6, 1, 11, 8, 14, 10, 12, 0];
        ui.horizontal_wrapped(|ui| {
            for cat_id in order {
                let Some((_, title)) = categories.iter().find(|(id, _)| *id == cat_id) else {
                    continue;
                };
                let selected = self.settings_menu_tab == cat_id;
                let label = RichText::new(*title).size(13.0).color(if selected {
                    Color32::WHITE
                } else {
                    text_color
                });
                let button = egui::Button::new(label)
                    .fill(if selected { accent } else { surface })
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(18.0))
                    .min_size(Vec2::new(0.0, 36.0));
                if ui.add(button).clicked() {
                    self.settings_menu_tab = cat_id;
                }
            }
        });

        ui.add_space(14.0);

        egui::ScrollArea::vertical().id_salt(("settings_section", self.settings_menu_tab)).show(ui, |ui| {
            // 1. АККАУНТ GOOGLE / YOUTUBE MUSIC
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 1 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Аккаунты Google / YouTube Music")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let is_logged_in = !self.settings.google_cookie.is_empty();

                        if is_logged_in {
                            let user_title = self.settings.account_name.as_deref().unwrap_or(crate::i18n::text("Пользователь YouTube"));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(16, 185, 129)));
                                ui.label(RichText::new(crate::localized_format!("Текущий активный аккаунт: {}", "Active account: {}", user_title)).size(13.0).strong().color(Color32::WHITE));
                            });
                            ui.label(RichText::new(crate::i18n::text("Синхронизация библиотеки, плейлистов и любимых треков с выбранным профилем активна.")).size(11.0).weak());
                            ui.add_space(10.0);

                            // ACCOUNT SWITCHER
                            if !self.settings.available_accounts.is_empty() {
                                ui.label(RichText::new(crate::i18n::text("Выберите, в какой аккаунт войти:")).size(12.0).strong());
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
                                                ui.label(RichText::new(crate::i18n::text("[Активен]")).size(11.0).color(Color32::from_rgb(16, 185, 129)));
                                            } else {
                                                if ui.button(crate::i18n::text("Выбрать этот")).clicked() {
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
                                    self.status_message = Some(crate::localized_format!("Переключено на аккаунт: {}", "Switched to account: {}", acc.name));
                                    self.sync_playlists();
                                }

                                ui.add_space(8.0);
                            }

                            if ui.button(crate::i18n::text("Переподключить через Firefox")).clicked() {
                                self.connect_youtube(None, BrowserAuth::firefox_sessions);
                            }

                            ui.horizontal(|ui| {
                                if ui.button(crate::i18n::text("Обновить список аккаунтов")).clicked() {
                                    self.refresh_youtube_accounts();
                                }

                                if ui.button(RichText::new(crate::i18n::text("Выйти из аккаунта")).color(Color32::from_rgb(244, 63, 94))).clicked() {
                                    self.reset_youtube_library();
                                    self.settings.google_cookie.clear();
                                    self.youtube_cookie_draft.clear();
                                    self.youtube_login_error = None;
                                    self.settings.account_name = None;
                                    self.settings.available_accounts.clear();
                                    self.settings.save();
                                    self.status_message = Some(crate::i18n::text("Вы вышли из аккаунта").to_string());
                                }
                            });
                        } else {
                            ui.label(RichText::new(crate::i18n::text("Вход позволяет слушать ваши плейлисты, синхронизировать лайки и рекомендации с YouTube Music.")).size(12.0).weak());
                            ui.add_space(8.0);

                            let ff_btn = egui::Button::new(RichText::new(crate::i18n::text("Войти через Firefox в один клик")).size(13.0).strong().color(Color32::WHITE))
                                .fill(Color32::from_rgb(234, 88, 12))
                                .rounding(Rounding::same(8.0));

                            if ui.add_sized([260.0, 36.0], ff_btn).clicked() {
                                self.connect_youtube(None, BrowserAuth::firefox_sessions);
                            }
                            ui.add_space(4.0);
                            ui.label(RichText::new(crate::i18n::text("Использует активную сессию YouTube Music в Firefox и предлагает доступные аккаунты.")).size(11.0).weak());

                        }

                        ui.add_space(8.0);
                        ui.collapsing(crate::i18n::text("Войти вручную через Cookie"), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(crate::i18n::text("1. Открой"));
                                ui.hyperlink_to("YouTube Music", "https://music.youtube.com");
                                ui.label(crate::i18n::text("в браузере и войди в нужный аккаунт."));
                            });
                            ui.label(crate::i18n::text("2. Нажми F12, открой вкладку «Сеть», введи browse в фильтр и нажми «Библиотека» на сайте."));
                            ui.label(crate::i18n::text("3. Выбери запрос POST browse со статусом 200 и доменом music.youtube.com."));
                            ui.label(crate::i18n::text("4. В Firefox нажми правой кнопкой по запросу → «Копировать» → «Копировать заголовки запроса». Можно также открыть «Заголовки запроса» и скопировать значение Cookie."));
                            ui.label(crate::i18n::text("5. Вставь скопированное в поле ниже и нажми «Войти»."));
                            ui.label(RichText::new(crate::i18n::text("Cookie даёт доступ к сессии. Вставляй его только сюда.")).weak());
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::multiline(&mut self.youtube_cookie_draft)
                                .password(true)
                                .desired_rows(2)
                                .hint_text(crate::i18n::text("Cookie или заголовки запроса"))
                                .desired_width(ui.available_width()));
                            if ui.add_enabled(!self.youtube_cookie_draft.trim().is_empty(), egui::Button::new(crate::i18n::text("Войти"))).clicked() {
                                match BrowserAuth::manual_session(&self.youtube_cookie_draft) {
                                    Ok((cookie, account)) => {
                                        self.youtube_cookie_draft.clear();
                                        self.connect_youtube(account, move || Ok(vec![cookie]));
                                    }
                                    Err(error) => self.youtube_login_error = Some(error),
                                }
                            }
                        });
                        if let Some(error) = &self.youtube_login_error {
                            ui.label(RichText::new(crate::i18n::message(error)).color(Color32::from_rgb(248, 113, 113)));
                        }
                    });
                ui.add_space(12.0);
            }

            // 2. АВТОСОЗДАННЫЕ ПЛЕЙЛИСТЫ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 2 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Автосозданные плейлисты (Библиотека)")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new(crate::i18n::text("Отображать автосозданные плейлисты в библиотеке:")).size(12.0).weak());
                        ui.add_space(6.0);

                        if ui.checkbox(&mut self.settings.show_liked_playlist, crate::i18n::text("Понравившиеся")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_downloaded_playlist, crate::i18n::text("Скачанные")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_top_playlist, crate::i18n::text("Топ")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_cached_playlist, crate::i18n::text("Кэшированные")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.show_uploaded_playlist, crate::i18n::text("Загруженные")).changed() {
                            self.settings.save();
                        }
                    });
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Плейлисты YouTube Music")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(crate::i18n::text("Сними галочку, чтобы скрыть плейлист из медиатеки и меню выбора."));
                        if self.playlists.is_empty()
                            && ui.add_enabled(!self.is_loading_playlists, egui::Button::new(crate::i18n::text("Загрузить плейлисты"))).clicked()
                        {
                            self.sync_playlists();
                        }
                        for playlist in &self.playlists {
                            let mut visible = !self.settings.youtube_playlist_hidden(&playlist.id);
                            if ui.checkbox(&mut visible, &playlist.title).changed() {
                                self.settings.set_youtube_playlist_hidden(&playlist.id, !visible);
                                self.settings.save();
                            }
                        }
                    });
                ui.add_space(12.0);
            }

            // 3. ВНЕШНИЙ ВИД И ТЕМА
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 3 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Внешний вид и Тема")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        let en = self.settings.ui_language == "en";
                        if ui.checkbox(&mut self.settings.cover_ambient_light, if en {
                            "Ambient cover lighting"
                        } else {
                            crate::i18n::text("Профессиональное освещение обложки")
                        }).changed() {
                            self.settings.save();
                        }

                        let ambient_enabled = self.settings.cover_ambient_light;
                        let mut ambient_changed = false;
                        for (value, label) in [
                            (&mut self.settings.ambient_pulse_strength, if en { "Pulse strength" } else { crate::i18n::text("Сила импульсов") }),
                            (&mut self.settings.ambient_density, if en { "Density" } else { crate::i18n::text("Плотность") }),
                            (&mut self.settings.ambient_blur, if en { "Blur" } else { crate::i18n::text("Размытие") }),
                        ] {
                            ambient_changed |= ui.add_enabled(
                                ambient_enabled,
                                egui::Slider::new(value, 0..=100).suffix("%").text(label),
                            ).changed();
                        }
                        if ambient_changed {
                            self.settings.save();
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Режим темы:"));
                            let themes = [crate::i18n::text("Чисто чёрный (AMOLED)"), crate::i18n::text("Тёмный сланец"), crate::i18n::text("Светлая")];
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
                            ui.label(crate::i18n::text("Акцентный цвет:"));
                            let color_names = [crate::i18n::text("Изумруд (Metrolist)"), crate::i18n::text("Рубин"), crate::i18n::text("Аметист"), crate::i18n::text("Кибер-синий"), crate::i18n::text("Янтарь")];
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
                            ui.label(crate::i18n::text("Кастомный цвет:"));
                            let mut custom_c = Color32::from_rgb(
                                self.settings.custom_accent_color[0],
                                self.settings.custom_accent_color[1],
                                self.settings.custom_accent_color[2],
                            );
                            let is_custom = self.settings.use_custom_accent;
                            let custom_btn = egui::Button::new(RichText::new(crate::i18n::text("Свой цвет")).color(Color32::WHITE))
                                .fill(if is_custom { custom_c } else { Color32::from_rgb(35, 35, 45) });
                            if ui.add(custom_btn).on_hover_text(crate::i18n::text("Активировать свой цвет")).clicked() {
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
                        if ui.checkbox(&mut self.settings.dynamic_icon_colors, crate::i18n::text("Динамические цвета значка под обложку")).changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 4. МИНИ-ПЛЕЕР И ПЛЕЕР
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 4 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Мини-плеер и Плеер")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.new_mini_player_design, crate::i18n::text("Новый дизайн мини-плеера")).changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Стиль фона мини-плеера:"));
                            let bg_styles = [crate::i18n::text("Градиент"), crate::i18n::text("Чистый черный"), crate::i18n::text("Сплошной")];
                            for (i, name) in bg_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.mini_player_bg_style == i, *name).clicked() {
                                    self.settings.mini_player_bg_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.new_player_design, crate::i18n::text("Новый дизайн плеера")).changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Стиль фона плеера:"));
                            let bg_styles = [crate::i18n::text("Градиент"), crate::i18n::text("Чистый черный"), crate::i18n::text("Сплошной")];
                            for (i, name) in bg_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.player_bg_style == i, *name).clicked() {
                                    self.settings.player_bg_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.hide_player_thumbnail, crate::i18n::text("Скрыть миниатюру плеера")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.crop_album_art, crate::i18n::text("Обрезать обложку альбома (1:1)")).changed() {
                            self.settings.save();
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Цветовой стиль кнопок плеера:"));
                            let btn_styles = [crate::i18n::text("Акцентный"), crate::i18n::text("Монохромный"), crate::i18n::text("Адаптивный")];
                            for (i, name) in btn_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.player_buttons_color_style == i, *name).clicked() {
                                    self.settings.player_buttons_color_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Стиль ползунка плеера:"));
                            let slider_styles = [crate::i18n::text("Прямой"), crate::i18n::text("Волнистый (Змейка)"), crate::i18n::text("Тонкий")];
                            for (i, name) in slider_styles.iter().enumerate() {
                                if ui.selectable_label(self.settings.slider_style == i, *name).clicked() {
                                    self.settings.slider_style = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.experimental_lyrics, crate::i18n::text("Экспериментальный текст")).changed() {
                            self.settings.save();
                        }

                        if ui.checkbox(&mut self.settings.show_remaining_time, crate::i18n::text("Отображать оставшееся время трека (-01:23)")).changed() {
                            self.settings.save();
                        }

                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Скругление обложек:"));
                            let roundings = [(4.0, "4 px"), (8.0, "8 px (Metrolist)"), (16.0, "16 px"), (24.0, crate::i18n::text("Круглые"))];
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
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Текст песни (Lyrics)")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Расположение текста песни:"));
                            let pos_names = [crate::i18n::text("По центру"), crate::i18n::text("По левому краю")];
                            for (i, name) in pos_names.iter().enumerate() {
                                if ui.selectable_label(self.settings.lyrics_position == i, *name).clicked() {
                                    self.settings.lyrics_position = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::i18n::text("Сдвиг текста для всех треков"));
                            if ui
                                .add(
                                    egui::DragValue::new(&mut self.settings.lyrics_manual_offset_ms)
                                        .speed(25)
                                        .range(-300_000..=300_000)
                                        .suffix(crate::i18n::text(" мс")),
                                )
                                .on_hover_text(crate::i18n::text("Этот сдвиг применяется ко всем трекам с таймингами. Поправка отдельного трека прибавляется к нему."))
                                .changed()
                            {
                                self.settings.save();
                            }
                            if ui.selectable_label(self.settings.lyrics_manual_offset_ms == 0, crate::i18n::text("0 мс"))
                                .on_hover_text(crate::i18n::text("Сбросить сдвиг")).clicked()
                            {
                                self.settings.lyrics_manual_offset_ms = 0;
                                self.settings.save();
                            }
                        });

                        if let Some(track) = self.current_track() {
                            ui.label(crate::localized_format!("Текущий трек · {}", "Current track · {}", track.title));
                            self.render_lyric_timing_controls(ui);
                        }

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::i18n::text("Анимация подсветки текста:"));
                            let anim_modes = [crate::i18n::text("По буквам (бета)"), crate::i18n::text("По словам"), crate::i18n::text("По строчкам"), crate::i18n::text("Без анимации")];
                            for (i, name) in anim_modes.iter().enumerate() {
                                if ui.selectable_label(self.settings.lyrics_animation_mode == i, *name).clicked() {
                                    self.settings.lyrics_animation_mode = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::i18n::text("Источник текстов по умолчанию:"));
                            let sources = [
                                (0, crate::i18n::text("Автоматически")),
                                (1, "LRCLIB"),
                                (2, "QQ"),
                                (3, "TTML"),
                                (4, "KuGou"),
                                (5, "LrcMux"),
                            ];
                            for (val, name) in sources {
                                if ui
                                    .selectable_label(self.settings.lyrics_source_mode == val, name)
                                    .clicked()
                                {
                                    self.settings.lyrics_source_mode = val;
                                    self.settings.save();
                                    self.trigger_lyrics_search(false);
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.lyrics_respect_artist_position, crate::i18n::text("Учитывать позиционирование исполнителя")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.lyrics_change_on_tap, crate::i18n::text("Перемотка трека по клику на строку текста")).changed() {
                            self.settings.save();
                        }
                        if ui.checkbox(&mut self.settings.lyrics_auto_scroll, crate::i18n::text("Автоматическая плавная прокрутка текста")).changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 6. ОКНО И НАВИГАЦИЯ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 6 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Окно и Навигация")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Вкладка навигации по умолчанию:"));
                            let nav_tabs = [crate::i18n::text("Главная"), crate::i18n::text("Медиатека"), crate::i18n::text("Любимые треки")];
                            for (i, name) in nav_tabs.iter().enumerate() {
                                if ui.selectable_label(self.settings.default_nav_tab == i, *name).clicked() {
                                    self.settings.default_nav_tab = i;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.slim_nav_bar, crate::i18n::text("Компактная боковая панель навигации")).changed() {
                            self.settings.save();
                        }

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Размер ячейки сетки библиотеки:"));
                            let grid_sizes = [crate::i18n::text("Маленький"), crate::i18n::text("Средний"), crate::i18n::text("Большой")];
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

            if self.settings_menu_tab == 0 || self.settings_menu_tab == 13 {
                let en = self.settings.ui_language == "en";
                let strings = crate::i18n::tr(&self.settings.ui_language);
                ui.heading(if en { "Sidebar tabs" } else { crate::i18n::text("Вкладки бокового меню") });
                ui.label(if en {
                    "Choose which tabs to show. Settings is always visible."
                } else {
                    crate::i18n::text("Выбери, какие вкладки показывать. «Настройки» видны всегда.")
                });
                let tabs = [
                    (&mut self.settings.show_nav_search, strings.nav_search),
                    (&mut self.settings.show_nav_liked, if en { "Liked Songs" } else { crate::i18n::text("Любимые треки") }),
                    (&mut self.settings.show_nav_playlists, strings.nav_library),
                    (&mut self.settings.show_nav_queue, if en { "Queue" } else { crate::i18n::text("Очередь") }),
                    (&mut self.settings.show_nav_wave, if en { "My wave" } else { crate::i18n::text("Своя волна") }),
                    (&mut self.settings.show_nav_lyrics, strings.player_lyrics_tab),
                    (&mut self.settings.show_nav_downloads, strings.nav_downloads),
                ];
                let mut changed = false;
                for (visible, label) in tabs {
                    changed |= ui.checkbox(visible, label).changed();
                }
                if changed {
                    self.settings.save();
                }
                ui.add_space(12.0);
            }

            // 7. ЗВУК И КАЧЕСТВО
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 7 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Звук и Воспроизведение")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Качество потока:"));
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
                        if ui.checkbox(&mut self.settings.sponsorblock_enabled, crate::i18n::text("SponsorBlock (автоматический пропуск немузыкальных фрагментов)")).changed() {
                            self.settings.save();
                        }

                        let en = self.settings.ui_language == "en";
                        let normalize_changed = ui.checkbox(
                            &mut self.settings.audio_normalization,
                            if en { "Normalize loudness" } else { crate::i18n::text("Выравнивать громкость") },
                        ).changed();
                        let silence_changed = ui.checkbox(
                            &mut self.settings.skip_silence,
                            if en { "Skip long silences" } else { crate::i18n::text("Пропускать долгую тишину") },
                        ).on_hover_text(if en {
                            "Shortens quiet gaps longer than 0.8 seconds, including at the end."
                        } else {
                            crate::i18n::text("Сокращает паузы тише −55 дБ длительностью от 0,8 с, в том числе в конце трека.")
                        }).changed();
                        if normalize_changed || silence_changed {
                            self.settings.save();
                        }

                        let mut fades_changed = false;
                        for (seconds, title) in [
                            (&mut self.settings.fade_in_seconds, if en { "Fade in" } else { crate::i18n::text("Нарастание в начале") }),
                            (&mut self.settings.fade_out_seconds, if en { "Fade out" } else { crate::i18n::text("Затухание в конце") }),
                        ] {
                            fades_changed |= ui.add(egui::Slider::new(seconds, 0.0..=15.0)
                                .step_by(0.5).suffix(if en { " s" } else { crate::i18n::text(" с") }).text(title)).changed();
                        }
                        if fades_changed {
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
                        if ui.checkbox(&mut self.settings.preserve_pitch, crate::i18n::text("Сохранять тональность звука при изменении скорости")).changed() {
                            self.player.set_speed(self.settings.playback_speed, self.settings.preserve_pitch);
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 8. DISCORD RPC
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 8 {
                egui::CollapsingHeader::new(RichText::new("Discord Rich Presence").size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.discord_rpc_enabled, crate::i18n::text("Отображать статус в Discord")).changed() {
                            self.settings.save();
                        }
                        if self.settings.discord_rpc_enabled {
                            let message = match self.discord.status() {
                                DiscordStatus::Idle => crate::i18n::text("Статус появится при воспроизведении трека.").into(),
                                DiscordStatus::Connecting => crate::i18n::text("Подключение к Discord…").into(),
                                DiscordStatus::Unavailable => crate::i18n::text("Нет ответа от настольного Discord. Проверь, что он запущен.").into(),
                                DiscordStatus::Active => crate::i18n::text("Discord принял статус.").into(),
                                DiscordStatus::Rejected(code) => crate::localized_format!("Discord отклонил статус. Код {code}.", "Discord rejected your activity. Code {code}."),
                            };
                            ui.label(message);
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Тип активности:"));
                            let types = [crate::i18n::text("Слушает (Listening)"), crate::i18n::text("Играет (Playing)")];
                            for (idx, name) in types.iter().enumerate() {
                                let is_sel = self.settings.discord_activity_type == idx;
                                if ui.selectable_label(is_sel, *name).clicked() {
                                    self.settings.discord_activity_type = idx;
                                    self.settings.save();
                                }
                            }
                        });

                        ui.add_space(4.0);
                        if ui.checkbox(&mut self.settings.discord_show_buttons, crate::i18n::text("Отображать кнопку «Слушать в Cibitube» в профиле Discord")).changed() {
                            self.settings.save();
                        }
                    });
                ui.add_space(12.0);
            }

            // 9. ХРАНИЛИЩЕ И ЗАГРУЗКИ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 9 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Загрузки и Хранилище")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::i18n::text("Папка для сохранения треков:"));
                            ui.add(egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width((ui.available_width() - 150.0).max(100.0)));
                            if ui.button(crate::i18n::text("Сохранить")).clicked() {
                                self.settings.save();
                                self.status_message = Some(crate::i18n::text("Папка загрузки обновлена").to_string());
                            }
                        });

                        ui.add_space(6.0);
                        let dir = self.settings.download_dir.clone();
                        if ui.button(crate::i18n::text("Открыть папку загрузок в файловом менеджере")).clicked() {
                            if let Err(error) = crate::local_media::open_folder(&dir) {
                                self.status_message = Some(error);
                            }
                        }

                        ui.add_space(8.0);
                        ui.label(RichText::new(crate::i18n::text("Опции умного загрузчика:")).strong());
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
                            ui.label(crate::i18n::text("Формат аудио:"));
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

            if self.settings_menu_tab == 14 {
                self.render_pet_settings(ui);
            }

            // 10. ОБНОВЛЕНИЯ ПРОГРАММЫ
            if self.settings_menu_tab == 0 || self.settings_menu_tab == 10 {
                egui::CollapsingHeader::new(RichText::new(crate::i18n::text("Обновления программы (GitHub)")).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        egui::CollapsingHeader::new(crate::i18n::text("Что обновилось")).show(ui, |ui| {
                            for note in include_str!("../CHANGELOG.md").lines().skip_while(|line| !line.starts_with("## ")).skip(1).filter(|line| line.starts_with("- ")) {
                                ui.label(format!("• {}", crate::i18n::text(&note[2..])));
                            }
                        });
                        let cur_ver = crate::updater::Updater::CURRENT_VERSION;
                        ui.label(RichText::new(crate::localized_format!("Текущая версия Cibitube: v{}", "Current Cibitube version: v{}", cur_ver)).size(13.0).strong());
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let check_label = if cfg!(target_os = "linux") { crate::i18n::text("Проверить и обновить") } else { strings.updater_check_btn };
                            let check_btn = egui::Button::new(RichText::new(check_label).strong())
                                .fill(Color32::from_rgb(35, 45, 55))
                                .rounding(Rounding::same(6.0));
                            if ui.add_enabled(!self.is_checking_update && !self.is_applying_update, check_btn).clicked() {
                                self.is_checking_update = true;
                                self.install_after_check = cfg!(target_os = "linux");
                                self.update_status = Some(crate::i18n::text("Проверка новых релизов на GitHub...").to_string());
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
                                ui.label(RichText::new(crate::localized_format!("★ Доступна новая версия: {}!", "★ A new version is available: {}!", info.version)).size(15.0).strong().color(Color32::from_rgb(52, 211, 153)));
                                if !info.notes.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(&info.notes).size(12.0).weak());
                                }
                                ui.add_space(8.0);

                                if self.is_applying_update {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new(crate::i18n::text("Скачивание и замена файла программы...")).color(Color32::from_rgb(56, 189, 248)));
                                    });
                                } else if self.show_update_confirm {
                                    ui.label(RichText::new(crate::i18n::text("Вы уверены, что хотите обновить приложение? Текущая версия будет автоматически заменена на новую.")).size(12.0).color(Color32::YELLOW));
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        let confirm_btn = egui::Button::new(RichText::new(crate::i18n::text("✓ Да, установить обновление")).strong().color(Color32::WHITE))
                                            .fill(Color32::from_rgb(16, 185, 129))
                                            .rounding(Rounding::same(6.0));
                                        if ui.add(confirm_btn).clicked() {
                                            self.begin_update(info.clone());
                                        }

                                        if ui.button(crate::i18n::text("Отмена")).clicked() {
                                            self.show_update_confirm = false;
                                        }
                                    });
                                } else {
                                    let update_btn = egui::Button::new(RichText::new(crate::localized_format!("Скачать и обновить до {}", "Download and update to {}", info.version)).strong().color(Color32::WHITE))
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
                                ui.label(RichText::new(crate::i18n::message(status)).size(12.0).color(Color32::from_rgb(147, 197, 253)));
                                if status.contains("✓") || status.contains("успешно") {
                                    let restart_btn = egui::Button::new(RichText::new(crate::i18n::text("Перезапустить Cibitube")).strong().color(Color32::WHITE))
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
                egui::CollapsingHeader::new(RichText::new(strings.sync_title).size(16.0).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        let folder_path = std::path::Path::new(&self.settings.google_drive_sync_folder);
                        let folder_exists = folder_path.exists();
                        let sync_file_exists = folder_path.join(crate::sync::SyncManager::SYNC_FILE_NAME).exists();

                        ui.horizontal(|ui| {
                            if folder_exists {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(16, 185, 129)));
                                ui.label(RichText::new(crate::i18n::text("Папка синхронизации доступна")).size(13.0).strong().color(Color32::WHITE));
                            } else {
                                ui.label(RichText::new("●").size(12.0).color(Color32::from_rgb(239, 68, 68)));
                                ui.label(RichText::new(crate::i18n::text("Папка синхронизации ещё не создана на диске")).size(13.0).weak());
                            }
                        });
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text("Папка синхронизации:"));
                            ui.add(egui::TextEdit::singleline(&mut self.settings.google_drive_sync_folder).desired_width(ui.available_width() - 140.0));
                            if ui.button(crate::i18n::text("Сохранить")).clicked() {
                                self.settings.save();
                                self.sync_status_message = Some(crate::i18n::text("Папка синхронизации сохранена").to_string());
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let push_btn = egui::Button::new(RichText::new(strings.sync_push_btn).strong().color(Color32::WHITE))
                                .fill(accent)
                                .rounding(Rounding::same(6.0));
                            if ui.add(push_btn).clicked() {
                                let info = self.player.info.lock().map(|info| info.clone()).unwrap_or_default();
                                self.pending_sync_save = Some(self.trigger_auto_sync(&info));
                                self.sync_status_message = Some(crate::i18n::text("Сохраняется состояние…").into());
                            }

                            let pull_btn = egui::Button::new(RichText::new(strings.sync_pull_btn).strong())
                                .fill(Color32::from_rgb(35, 45, 55))
                                .rounding(Rounding::same(6.0));
                            if ui.add(pull_btn).clicked() {
                                if let Some(state) = crate::sync::SyncManager::load_state(&self.settings.google_drive_sync_folder) {
                                    let title = state.title.clone();
                                    self.restore_session(state, true);
                                    self.sync_status_message = Some(if title.is_empty() {
                                        crate::i18n::text("Загружена пустая очередь.").into()
                                    } else {
                                        crate::localized_format!("Загружается «{title}»…", "Loading “{title}”…")
                                    });
                                } else {
                                    self.sync_status_message = Some(crate::localized_format!("Файл {} не найден в указанной папке", "File {} was not found in the selected folder", crate::sync::SyncManager::SYNC_FILE_NAME));
                                }
                            }
                        });

                        if let Some(msg) = &self.sync_status_message {
                            ui.add_space(6.0);
                            ui.label(RichText::new(crate::i18n::message(msg)).size(12.0).color(Color32::from_rgb(147, 197, 253)));
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
                egui::CollapsingHeader::new(RichText::new(strings.donate_title).size(16.0).strong())
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
                                self.status_message = Some(crate::i18n::text("TON/USDT адрес скопирован в буфер обмена!").to_string());
                            }
                        });
                    });
                ui.add_space(12.0);
            }
        });
    }
}
