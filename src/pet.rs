use crate::app::{AppEvent, MetrolistApp};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SIDE: f32 = 58.0;
const LEGACY_ROWS: u32 = 8;
const SEATED_SINGING_ROWS: u32 = 9;
const ROWS: u32 = 10;
const LEGACY_COLUMNS: u32 = 16;
const COLUMNS: u32 = 36;
const PROMPT: &str = "Сделай или дополни лист кадров питомца для Cibitube. Если я приложил PNG, используй того же персонажа и сохрани стиль, пропорции, размер клетки и точку опоры. PNG должен быть прозрачным, с десятью строками и 36 столбцами. Для нового персонажа используй клетку 96×96 пикселей (итого 3456×960). Если я приложил лист на 36 столбцов и 9 строк, сохрани его и дорисуй только десятую строку.\n\nВ каждой строке девять групп по четыре кадра. Порядок групп: вправо, вправо-вниз, вниз, влево-вниз, влево, влево-вверх, вверх, вправо-вверх, прямо. В старом листе на 16 столбцов группы идут в порядке влево, вправо, вверх, вниз. Перенеси этих четыре группы как есть на соответствующие места и дорисуй для каждой позы отдельные кадры для четырёх диагоналей и взгляда прямо. Поворачивай голову и глаза, сохраняя пропорции персонажа и положение тела.\n\nПорядок строк: появление, стояние, сидение, залезание на кнопку, нет интернета, пение стоя, бег, исчезание, пение сидя, падение. В 10-й строке покажи четыре кадра падения для каждого направления. Сначала персонаж теряет опору, затем поджимает ноги, разводит руки для равновесия, а в последнем кадре готовится к приземлению. Держи персонажа внутри клетки. Перемещение вниз выполняет приложение. Последний кадр сидения — неподвижная поза. В строке пения сидя меняется только рот относительно последнего кадра сидения в том же направлении. Если пение сидя уже есть, сохрани готовые кадры. Верни готовый PNG без разделителей и подписей.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct PetSettings {
    enabled: bool,
    follow: bool,
    follow_lyrics: bool,
    sing: bool,
    sitting: bool,
    position: [f32; 2],
    custom: bool,
    size: f32,
}

impl Default for PetSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            follow: true,
            follow_lyrics: true,
            sing: true,
            sitting: false,
            position: [0.15, 0.82],
            custom: false,
            size: SIDE,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pose {
    Appear,
    Stand,
    Sit,
    Climb,
    Offline,
    Sing,
    Run,
    Disappear,
    SitSing,
    Fall,
}

#[derive(Default)]
pub(crate) struct Pet {
    pos: Option<Pos2>,
    drag: Option<(Pos2, Vec2)>,
    perch: Option<(Id, f32)>,
    jump: Option<Jump>,
    recovery: Option<Recovery>,
    game_resume_at: f64,
    game: bool,
    menu: bool,
    menu_rect: Option<Rect>,
    animation: Option<(Pose, f64)>,
    was_enabled: bool,
    pose: Option<(Pose, f64)>,
    offline: bool,
    next_probe: f64,
    probe: Option<tokio::sync::oneshot::Receiver<bool>>,
    texture: Option<egui::TextureHandle>,
    sprite_attempted: bool,
    message: Option<String>,
}

#[derive(Clone, Copy)]
struct Surface {
    id: Id,
    rect: Rect,
    button: bool,
    parent: Option<Id>,
}

struct Jump {
    from: Pos2,
    to: Pos2,
    perch: (Id, f32),
    start: f64,
}

struct Recovery {
    origin: Pos2,
    previous: (Id, f32),
    phase: RecoveryPhase,
}

enum RecoveryPhase {
    Falling { speed: f32 },
    Returning { landed_at: f64 },
}

impl Recovery {
    fn new(origin: Pos2, previous: (Id, f32)) -> Self {
        Self {
            origin,
            previous,
            phase: RecoveryPhase::Falling { speed: 0.0 },
        }
    }
}

fn surfaces_id() -> Id {
    Id::new("pet_perch_surfaces")
}

pub(crate) fn clear_surfaces(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<Surface>>(surfaces_id())
            .clear();
        data.remove::<Rect>(Id::new("pet-active-lyric"));
    });
}

pub(crate) fn register_surface(ctx: &egui::Context, id: Id, rect: Rect, button: bool) {
    register_platform(
        ctx,
        Surface {
            id,
            rect,
            button,
            parent: None,
        },
    );
}

fn register_platform(ctx: &egui::Context, surface: Surface) {
    if surface.rect.is_positive() {
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Vec<Surface>>(surfaces_id())
                .push(surface);
        });
    }
}

pub(crate) fn register_lyric_row(
    ui: &egui::Ui,
    track_id: Option<&str>,
    index: usize,
    response: &egui::Response,
    active: bool,
    parent: Option<Id>,
) {
    let visible = response.interact_rect.intersect(ui.clip_rect());
    if active && visible.is_positive() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::new("pet-active-lyric"), visible));
    }
    register_platform(
        ui.ctx(),
        Surface {
            id: ui.make_persistent_id(("pet-lyric-row", track_id, index)),
            rect: visible,
            button: false,
            parent,
        },
    );
}

impl Surface {
    fn anchor(self, x: f32) -> f32 {
        if self.button {
            0.5
        } else {
            ((x - self.rect.left()) / self.rect.width()).clamp(0.0, 1.0)
        }
    }

    fn supports(self, bounds: Rect, side: f32) -> bool {
        bounds.y_range().contains(self.landing(0.5, side).y)
    }

    fn landing(self, anchor: f32, side: f32) -> Pos2 {
        Pos2::new(
            self.rect.left() + self.rect.width() * anchor - side / 2.0,
            self.rect.top() - side + side * 0.08,
        )
    }
}

fn sprite_path() -> PathBuf {
    crate::settings::AppSettings::config_path().with_file_name("pet-sprites.png")
}

fn sprite_grid(width: u32, height: u32) -> Option<(u32, u32)> {
    [(COLUMNS, 128), (LEGACY_COLUMNS, 256)]
        .into_iter()
        .find_map(|(columns, max_cell)| {
            let cell = width / columns;
            if width != columns * cell || !(16..=max_cell).contains(&cell) {
                return None;
            }
            [ROWS, SEATED_SINGING_ROWS, LEGACY_ROWS]
                .into_iter()
                .find(|rows| height == rows * cell)
                .map(|rows| (columns, rows))
        })
}

fn decode_sprite(path: &Path) -> Result<(Vec<u8>, egui::ColorImage), String> {
    const MAX_FILE: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(path).map_or(true, |meta| !meta.is_file() || meta.len() > MAX_FILE) {
        return Err("Выберите PNG размером до 8 МБ.".into());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&bytes))
        .map_err(|_| "Файл должен быть PNG.")?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    if sprite_grid(width, height).is_none() {
        return Err(
            "Нужен PNG с квадратными кадрами: 36 столбцов с 8–10 строками (16–128 пикселей) или 16 столбцов с 8–10 строками (16–256 пикселей)."
                .into(),
        );
    }
    let rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok((
        bytes,
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], rgba.as_raw()),
    ))
}

fn target_button(ctx: &egui::Context) -> Option<egui::Response> {
    let ids = ctx.interaction_snapshot(|hit| hit.contains_pointer.clone());
    ids.iter()
        .filter_map(|id| ctx.read_response(*id))
        .filter(|response| {
            response.enabled()
                && response.sense.click
                && !response.sense.drag
                && response.layer_id.order <= egui::Order::Middle
                && response.interact_rect.is_positive()
        })
        .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
}

fn sprite_frame(pose: Pose, age: f64) -> u32 {
    let frame = (age * 8.0) as u32;
    match pose {
        Pose::Appear | Pose::Disappear | Pose::Sit | Pose::Fall => frame.min(3),
        _ => frame % 4,
    }
}

fn sprite_direction(look: Vec2, columns: u32) -> u32 {
    if columns == LEGACY_COLUMNS {
        if look.y.abs() > look.x.abs() {
            return if look.y < 0.0 { 2 } else { 3 };
        }
        return if look.x < 0.0 {
            0
        } else if look.x > 0.0 {
            1
        } else {
            3
        };
    }
    if look == Vec2::ZERO {
        return 8;
    }
    let sector = (look.y.atan2(look.x) / std::f32::consts::FRAC_PI_4).round() as i32;
    sector.rem_euclid(8) as u32
}

fn paint_builtin(ui: &egui::Ui, rect: Rect, pose: Pose, time: f64, look: Vec2) {
    let t = time as f32;
    let scale = rect.width() / SIDE;
    let v = |x, y| Vec2::new(x, y) * scale;
    let opacity = match pose {
        Pose::Appear => (t / 0.45).clamp(0.0, 1.0),
        Pose::Disappear => (1.0 - t / 0.45).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let hop = match pose {
        Pose::Climb => (t * 9.0).sin().abs() * 4.0,
        Pose::Run => (t * 14.0).sin().abs() * 3.0,
        _ => 0.0,
    };
    let seated = matches!(pose, Pose::Sit | Pose::SitSing | Pose::Offline);
    let center = rect.center() + v(0.0, 5.0 - hop + if seated { 5.0 } else { 0.0 });
    let coat = if pose == Pose::Offline {
        Color32::from_rgb(142, 155, 174)
    } else {
        Color32::from_rgb(255, 181, 104)
    }
    .gamma_multiply(opacity);
    let dark = Color32::from_rgb(53, 37, 57).gamma_multiply(opacity);
    let pale = Color32::from_rgb(255, 231, 199).gamma_multiply(opacity);
    let painter = ui.painter();
    painter.circle_filled(
        center + v(0.0, 9.0),
        (if seated { 13.0 } else { 11.0 }) * scale,
        coat,
    );
    for side in [-1.0, 1.0] {
        let (stride, lift) = match pose {
            Pose::Run => ((t * 16.0).sin() * side * 4.0, 0.0),
            Pose::Fall => (side * (t * 14.0).sin() * 2.0, 4.0),
            _ => (0.0, 0.0),
        };
        painter.circle_filled(
            center + v(side * 9.0 + stride, 17.0 - lift),
            4.0 * scale,
            coat,
        );
        if pose == Pose::Fall {
            painter.line_segment(
                [
                    center + v(side * 9.0, 10.0),
                    center + v(side * 17.0, 6.0 + stride),
                ],
                Stroke::new(3.0 * scale, coat),
            );
        }
    }
    let head = center + v(0.0, -8.0) + look * (4.0 * scale);
    for side in [-1.0, 1.0] {
        painter.add(egui::Shape::convex_polygon(
            vec![
                head + v(side * 14.0, -3.0),
                head + v(side * 12.0, -18.0),
                head + v(side * 4.0, -8.0),
            ],
            coat,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(head, 15.0 * scale, coat);
    painter.circle_filled(head + v(0.0, 5.0), 7.0 * scale, pale);
    for side in [-1.0, 1.0] {
        painter.circle_filled(
            head + v(side * 6.0, -1.0) + look * (1.5 * scale),
            1.8 * scale,
            dark,
        );
    }
    if matches!(pose, Pose::Sing | Pose::SitSing) {
        painter.circle_filled(
            head + v(0.0, 6.0),
            (2.0 + (t * 13.0).sin().abs()) * scale,
            dark,
        );
    } else {
        painter.line_segment(
            [head + v(-2.0, 6.0), head + v(2.0, 6.0)],
            Stroke::new(1.2 * scale, dark),
        );
    }
}

impl Pet {
    fn check_connection(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(probe) = &mut self.probe {
            match probe.try_recv() {
                Ok(online) => {
                    self.offline = !online;
                    self.probe = None;
                    self.next_probe = now + if online { 30.0 } else { 8.0 };
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.probe = None;
                    self.next_probe = now + 8.0;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if self.probe.is_none() && now >= self.next_probe {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.probe = Some(rx);
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let online = reqwest::Client::new()
                    .head("https://music.youtube.com/")
                    .timeout(std::time::Duration::from_secs(4))
                    .send()
                    .await
                    .is_ok();
                let _ = tx.send(online);
                ctx.request_repaint();
            });
        }
    }

    pub(crate) fn keep_on_replaced_panel(&mut self, ctx: &egui::Context, panel_id: Id) {
        ctx.data(|data| {
            let Some(surfaces) = data.get_temp::<Vec<Surface>>(surfaces_id()) else {
                return;
            };
            let Some(panel) = surfaces.iter().find(|surface| surface.id == panel_id) else {
                return;
            };
            let reanchor = |perch: &mut (Id, f32)| {
                if let Some(row) = surfaces
                    .iter()
                    .find(|row| row.id == perch.0 && row.parent == Some(panel_id))
                {
                    let x = row.rect.left() + row.rect.width() * perch.1;
                    *perch = (panel_id, panel.anchor(x));
                }
            };
            if let Some(perch) = &mut self.perch {
                reanchor(perch);
            }
            if let Some(jump) = &mut self.jump {
                reanchor(&mut jump.perch);
            }
        });
    }

    fn move_on_platforms(
        &mut self,
        ctx: &egui::Context,
        surfaces: &[Surface],
        pos: &mut Pos2,
        bounds: Rect,
        side: f32,
    ) -> Option<Pose> {
        const REACH: f32 = 160.0;
        let (now, pointer, dt) =
            ctx.input(|input| (input.time, input.pointer.hover_pos(), input.unstable_dt));
        let dt = dt.clamp(0.0, 0.08);
        let feet = *pos + Vec2::new(side / 2.0, side * 0.92);
        let clamp = |point: Pos2| point.clamp(bounds.min, bounds.max);
        let visible = |surface: &Surface| surface.supports(bounds, side);
        let surface_for = |id| {
            surfaces
                .iter()
                .find(|surface| surface.id == id)
                .copied()
                .or_else(|| {
                    // Dragging can also select a regular egui button under the pointer.
                    ctx.read_response(id)
                        .filter(|response| {
                            response.enabled()
                                && response.interact_rect.is_positive()
                                && response.layer_id.order <= egui::Order::Middle
                        })
                        .map(|response| Surface {
                            id,
                            rect: response.interact_rect,
                            button: true,
                            parent: None,
                        })
                })
                .filter(visible)
        };

        if let Some(jump) = &mut self.jump {
            if let Some(surface) = surface_for(jump.perch.0) {
                jump.to = clamp(surface.landing(jump.perch.1, side));
                let duration = (f64::from(jump.from.distance(jump.to)) / 320.0).clamp(0.7, 1.6);
                let progress = ((now - jump.start) / duration).clamp(0.0, 1.0) as f32;
                let eased = progress * progress * (3.0 - 2.0 * progress);
                let lift = 4.0 * progress * (1.0 - progress) * (side * 0.45 + 12.0);
                *pos = clamp(jump.from.lerp(jump.to, eased) - Vec2::new(0.0, lift));
                if progress >= 1.0 {
                    self.perch = Some(jump.perch);
                    self.jump = None;
                    self.recovery = None;
                    self.game_resume_at = now + 0.6;
                    return None;
                }
                return Some(Pose::Climb);
            }
            let recovery = self
                .recovery
                .get_or_insert_with(|| Recovery::new(jump.to, jump.perch));
            recovery.phase = RecoveryPhase::Falling { speed: 0.0 };
            self.jump = None;
        }

        let support = self.perch.and_then(|(id, _)| surface_for(id));
        if let Some(previous) = self.perch {
            if let Some(surface) = support {
                if self.recovery.is_none() {
                    *pos = clamp(surface.landing(previous.1, side));
                } else {
                    pos.y = surface.landing(previous.1, side).y;
                }
            } else {
                self.perch = None;
                let recovery = self
                    .recovery
                    .get_or_insert_with(|| Recovery::new(*pos, previous));
                recovery.phase = RecoveryPhase::Falling { speed: 0.0 };
            }
        }

        if let Some(recovery) = &mut self.recovery {
            match &mut recovery.phase {
                RecoveryPhase::Falling { speed } => {
                    *speed = (*speed + 660.0 * dt).min(380.0);
                    let next_y = (pos.y + *speed * dt).min(bounds.bottom());
                    let landing = surfaces
                        .iter()
                        .copied()
                        .filter(visible)
                        .filter(|surface| {
                            let y = surface.landing(0.5, side).y;
                            surface.rect.x_range().contains(feet.x)
                                && y >= pos.y - 0.5
                                && y <= next_y
                        })
                        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
                    if let Some(surface) = landing {
                        let anchor = surface.anchor(feet.x);
                        *pos = clamp(surface.landing(anchor, side));
                        self.perch = Some((surface.id, anchor));
                    } else {
                        pos.y = next_y;
                        if next_y < bounds.bottom() {
                            return Some(Pose::Fall);
                        }
                    }
                    recovery.phase = RecoveryPhase::Returning { landed_at: now };
                    return Some(Pose::Stand);
                }
                RecoveryPhase::Returning { landed_at } if now - *landed_at < 0.22 => {
                    return Some(Pose::Stand);
                }
                RecoveryPhase::Returning { .. } => {}
            }

            let origin_x = recovery.origin.x + side / 2.0;
            let target = surface_for(recovery.previous.0)
                .map(|surface| (surface, recovery.previous.1))
                .or_else(|| {
                    surfaces
                        .iter()
                        .copied()
                        .filter(visible)
                        .filter(|surface| support.is_none_or(|support| surface.id != support.id))
                        .map(|surface| (surface, surface.anchor(origin_x)))
                        .min_by(|(a, a_anchor), (b, b_anchor)| {
                            a.landing(*a_anchor, side)
                                .distance_sq(recovery.origin)
                                .total_cmp(&b.landing(*b_anchor, side).distance_sq(recovery.origin))
                        })
                });
            let (surface, anchor) = target?;
            let destination = clamp(surface.landing(anchor, side));
            let walk_x = support
                .map_or(destination.x, |platform| {
                    let mut range = platform.rect.x_range().shrink(5.0);
                    if range.min > range.max {
                        range = egui::emath::Rangef::point(platform.rect.center().x);
                    }
                    range.clamp(destination.x + side / 2.0) - side / 2.0
                })
                .clamp(bounds.left(), bounds.right());
            let delta = walk_x - pos.x;
            if delta.abs() > 1.0 {
                pos.x += delta.clamp(-170.0 * dt, 170.0 * dt);
                return Some(Pose::Run);
            }

            if pos.distance(destination) <= 2.0 {
                self.recovery = None;
                *pos = destination;
                self.perch = Some((surface.id, anchor));
                self.game_resume_at = now + 0.6;
                return None;
            }
            self.perch = None;
            self.jump = Some(Jump {
                from: *pos,
                to: destination,
                perch: (surface.id, anchor),
                start: now,
            });
            return Some(Pose::Climb);
        }

        if !self.game || self.menu || now < self.game_resume_at {
            return None;
        }
        let near_cursor = pointer.and_then(|cursor| {
            surfaces
                .iter()
                .copied()
                .filter(visible)
                .filter(|surface| surface.rect.distance_to_pos(cursor) <= REACH)
                .min_by(|a, b| {
                    a.rect
                        .distance_to_pos(cursor)
                        .total_cmp(&b.rect.distance_to_pos(cursor))
                        .then_with(|| a.rect.area().total_cmp(&b.rect.area()))
                })
                .map(|surface| (surface, cursor.x))
        });
        let target = near_cursor.or_else(|| {
            if self.perch.is_some() {
                return None;
            }
            surfaces
                .iter()
                .copied()
                .filter(visible)
                .min_by(|a, b| {
                    a.rect
                        .distance_to_pos(feet)
                        .total_cmp(&b.rect.distance_to_pos(feet))
                })
                .map(|surface| (surface, feet.x))
        });
        if let Some((surface, x)) =
            target.filter(|(surface, _)| !self.perch.is_some_and(|(id, _)| id == surface.id))
        {
            let anchor = surface.anchor(x);
            self.perch = None;
            self.jump = Some(Jump {
                from: *pos,
                to: clamp(surface.landing(anchor, side)),
                perch: (surface.id, anchor),
                start: now,
            });
            return Some(Pose::Climb);
        }
        None
    }

    fn load_custom_sprite(&mut self, ctx: &egui::Context, custom: bool) {
        if custom && !self.sprite_attempted {
            self.sprite_attempted = true;
            match decode_sprite(&sprite_path()) {
                Ok((_, image)) => {
                    self.texture =
                        Some(ctx.load_texture("cibitube-pet", image, egui::TextureOptions::LINEAR))
                }
                Err(error) => self.message = Some(error),
            }
        }
    }

    fn custom_sprite_grid(&self) -> Option<(u32, u32)> {
        let [width, height] = self.texture.as_ref()?.size();
        sprite_grid(width as u32, height as u32)
    }

    fn render(&mut self, ctx: &egui::Context, settings: &mut PetSettings, singing: bool) -> bool {
        let (now, pointer) = ctx.input(|input| (input.time, input.pointer.hover_pos()));
        let mut changed = false;
        if settings.enabled != self.was_enabled {
            self.animation = Some((
                if settings.enabled {
                    Pose::Appear
                } else {
                    Pose::Disappear
                },
                now,
            ));
            self.was_enabled = settings.enabled;
            if !settings.enabled {
                self.menu = false;
                self.game = false;
            }
        }
        if self.animation.is_some_and(|(_, start)| now - start >= 0.5) {
            self.animation = None;
        }
        if !settings.enabled && self.animation.is_none() {
            return false;
        }
        if settings.enabled {
            self.check_connection(ctx, now);
        }

        self.load_custom_sprite(ctx, settings.custom);

        let screen = ctx.screen_rect();
        let side = settings
            .size
            .clamp(36.0, 128.0)
            .min(screen.width().min(screen.height()).max(1.0));
        let travel = (screen.size() - Vec2::splat(side)).max(Vec2::splat(1.0));
        let bounds = Rect::from_min_size(
            screen.min,
            (screen.size() - Vec2::splat(side)).max(Vec2::ZERO),
        );
        let clamp = |pos: Pos2| pos.clamp(bounds.min, bounds.max);
        let home = screen.min
            + Vec2::new(
                travel.x * settings.position[0],
                travel.y * settings.position[1],
            );
        let mut pos = clamp(self.pos.unwrap_or(home));
        let mut surfaces = ctx
            .data(|data| data.get_temp::<Vec<Surface>>(surfaces_id()))
            .unwrap_or_default();
        let hovered_button = target_button(ctx).map(|response| Surface {
            id: response.id,
            rect: response.interact_rect,
            button: true,
            parent: None,
        });
        if let Some(button) =
            hovered_button.filter(|button| !surfaces.iter().any(|surface| surface.id == button.id))
        {
            surfaces.push(button);
        }
        let drop_target = pointer.and_then(|pointer| {
            surfaces
                .iter()
                .copied()
                .filter(|surface| surface.rect.contains(pointer) && surface.supports(bounds, side))
                .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
        });
        let previous_pos = pos;
        let motion = if settings.enabled && self.drag.is_none() {
            self.move_on_platforms(ctx, &surfaces, &mut pos, bounds, side)
        } else {
            None
        };
        if let (Some((_, offset)), Some(pointer)) = (self.drag, pointer) {
            pos = clamp(pointer - offset);
        }
        self.pos = Some(pos);

        let pose = self.animation.map(|(pose, _)| pose).unwrap_or_else(|| {
            if self.drag.is_some() {
                Pose::Climb
            } else if let Some(motion) = motion {
                motion
            } else if self.offline {
                Pose::Offline
            } else if settings.sitting || (self.game && self.perch.is_some()) {
                if settings.sing && singing {
                    Pose::SitSing
                } else {
                    Pose::Sit
                }
            } else if settings.sing && singing {
                Pose::Sing
            } else {
                Pose::Stand
            }
        });
        let (_, started) = *self.pose.get_or_insert((pose, now));
        if let Some((previous, _)) = self.pose.filter(|(previous, _)| *previous != pose) {
            let already_seated =
                pose == Pose::Sit && matches!(previous, Pose::SitSing | Pose::Offline);
            self.pose = Some((pose, if already_seated { now - 0.5 } else { now }));
        }
        let age = now - self.pose.map_or(started, |(_, at)| at);
        let enabled = settings.enabled;
        let area = egui::Area::new(Id::new("cibitube-pet"))
            .order(egui::Order::Foreground)
            // Keep the interactive hitbox at the starting position while dragging.
            // The pointer can then see the controls behind the painted pet.
            .fixed_pos(self.drag.map_or(pos, |(origin, _)| origin))
            .interactable(enabled)
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
                let rect = Rect::from_min_size(pos, rect.size());
                let lyric = if settings.follow_lyrics {
                    ctx.data(|data| data.get_temp::<Rect>(Id::new("pet-active-lyric")))
                } else {
                    None
                };
                let focus = lyric
                    .map(|row| row.center())
                    .or(pointer.filter(|_| settings.follow));
                let look = match pose {
                    Pose::Run => Vec2::new((pos.x - previous_pos.x).signum(), 0.0),
                    Pose::Fall => Vec2::new(0.0, 1.0),
                    _ => focus.map_or(Vec2::ZERO, |point| (point - rect.center()).normalized()),
                };
                if let Some(texture) = &self.texture {
                    let [width, height] = texture.size();
                    let (columns, rows) = sprite_grid(width as u32, height as u32)
                        .expect("imported sprite grid was validated");
                    let direction = sprite_direction(look, columns);
                    let (sprite_pose, frame) = match pose {
                        Pose::SitSing if rows == LEGACY_ROWS => (Pose::Sit, 3),
                        Pose::Fall if rows < ROWS => (Pose::Stand, sprite_frame(Pose::Stand, age)),
                        _ => (pose, sprite_frame(pose, age)),
                    };
                    let column = (direction * 4 + frame) as f32;
                    let row = sprite_pose as u8 as f32;
                    let uv = Rect::from_min_max(
                        Pos2::new(column / columns as f32, row / rows as f32),
                        Pos2::new((column + 1.0) / columns as f32, (row + 1.0) / rows as f32),
                    );
                    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
                } else {
                    paint_builtin(ui, rect, pose, age, look);
                }
                response
            });
        let response = area.inner;
        if response.drag_started() {
            self.drag = Some((pos, pointer.map_or(Vec2::splat(side / 2.0), |p| p - pos)));
            self.menu = false;
            self.game = false;
            self.perch = None;
            self.jump = None;
            self.recovery = None;
        }
        if response.drag_stopped() {
            self.drag = None;
            let landing = if let Some(surface) = drop_target {
                let anchor = surface.anchor(pointer.map_or(surface.rect.center().x, |p| p.x));
                self.perch = Some((surface.id, anchor));
                clamp(surface.landing(anchor, side))
            } else {
                self.pos.unwrap_or(pos)
            };
            self.pos = Some(landing);
            settings.position = [
                (landing.x - screen.left()) / travel.x,
                (landing.y - screen.top()) / travel.y,
            ];
            changed = true;
        }

        if motion.is_some() {
            self.menu = false;
        }
        if settings.enabled && self.drag.is_none() && motion.is_none() {
            let over_menu = self.menu
                && pointer.is_some_and(|p| {
                    self.menu_rect
                        .is_some_and(|rect| rect.union(response.rect).expand(5.0).contains(p))
                });
            self.menu = response.hovered() || response.clicked() || over_menu;
            if self.menu {
                let x = if pos.x + side + 190.0 <= screen.right() {
                    pos.x + side + 3.0
                } else {
                    pos.x - 187.0
                };
                let menu = egui::Area::new(Id::new("cibitube-pet-menu"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(Pos2::new(x, pos.y - 25.0))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_width(172.0);
                            changed |= ui
                                .checkbox(&mut settings.follow, "Следить за курсором")
                                .changed();
                            changed |= ui
                                .checkbox(&mut settings.follow_lyrics, "Следить за строкой песни")
                                .changed();
                            changed |= ui
                                .checkbox(&mut settings.sing, "Рот под текст песни")
                                .changed();
                            changed |= ui.checkbox(&mut settings.sitting, "Посадить").changed();
                            ui.checkbox(&mut self.game, "Режим игры");
                            if ui.button("Спрятать").clicked() {
                                settings.enabled = false;
                                changed = true;
                            }
                        });
                    });
                self.menu_rect = Some(menu.response.rect);
            }
        }
        let (focused, minimized) =
            ctx.input(|input| (input.focused, input.viewport().minimized == Some(true)));
        if !minimized && motion.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else if focused
            && !minimized
            && (self.animation.is_some()
                || matches!(pose, Pose::Sing | Pose::SitSing | Pose::Offline)
                || (pose == Pose::Sit && age < 0.4)
                || (pose == Pose::Stand && self.texture.is_some()))
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(65));
        }
        changed
    }
}

impl MetrolistApp {
    pub(crate) fn render_pet(
        &mut self,
        ctx: &egui::Context,
        playback: &crate::models::PlaybackInfo,
    ) {
        let singing = playback.is_playing
            && self.lyrics.as_ref().is_some_and(|lyrics| {
                let ms = ((playback.position_secs * 1000.0) as i64)
                    .saturating_add(self.lyric_total_offset(lyrics));
                let Ok(ms) = u64::try_from(ms) else {
                    return false;
                };
                if !lyrics.is_synced {
                    return false;
                }
                let Some(index) = lyrics.lines.iter().rposition(|line| line.start_ms <= ms) else {
                    return false;
                };
                let line = &lyrics.lines[index];
                if let Some(words) = line.words.as_ref().filter(|words| !words.is_empty()) {
                    words.iter().any(|word| {
                        word.start_ms <= ms && ms < word.end_ms.max(word.start_ms + 100)
                    })
                } else {
                    let end = line
                        .end_ms
                        .or_else(|| lyrics.lines.get(index + 1).map(|line| line.start_ms))
                        .unwrap_or(line.start_ms + 2_000);
                    !line.text.trim().is_empty() && ms < end.min(line.start_ms + 6_000)
                }
            });
        if self.pet.render(ctx, &mut self.settings.pet, singing) {
            self.settings.save();
        }
    }

    pub(crate) fn import_pet_sprite(&mut self, path: &Path, ctx: &egui::Context) {
        self.pet.message = Some(
            match decode_sprite(path).and_then(|(bytes, image)| {
                let destination = sprite_path();
                let temporary = destination.with_extension("tmp");
                std::fs::write(&temporary, bytes)
                    .and_then(|_| std::fs::rename(temporary, destination))
                    .map_err(|error| error.to_string())?;
                self.pet.texture =
                    Some(ctx.load_texture("cibitube-pet", image, egui::TextureOptions::LINEAR));
                self.pet.sprite_attempted = true;
                self.settings.pet.custom = true;
                self.settings.pet.enabled = true;
                self.settings.save();
                Ok(())
            }) {
                Ok(()) => "Питомец загружен.".into(),
                Err(error) => error,
            },
        );
    }

    pub(crate) fn pet_import_error(&mut self, error: String) {
        self.pet.message = Some(error);
    }

    pub(crate) fn pet_sprite_update_needed(&mut self, ctx: &egui::Context) -> bool {
        let custom = self.settings.pet.custom;
        self.pet.load_custom_sprite(ctx, custom);
        custom
            && self
                .pet
                .custom_sprite_grid()
                .is_some_and(|(columns, rows)| columns < COLUMNS || rows < ROWS)
    }

    pub(crate) fn render_pet_settings(&mut self, ui: &mut egui::Ui) {
        let mut changed = ui
            .checkbox(&mut self.settings.pet.enabled, "Показывать питомца")
            .changed();
        changed |= ui
            .checkbox(&mut self.settings.pet.follow, "Следить головой за курсором")
            .changed();
        changed |= ui
            .checkbox(
                &mut self.settings.pet.follow_lyrics,
                "Следить за подсвеченной строкой песни",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.settings.pet.sing,
                "Открывать рот по таймингам текста песни",
            )
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut self.settings.pet.size, 36.0..=128.0).text("Размер питомца"),
            )
            .changed();
        ui.label("Перетащите питомца на кнопку, строку песни, заголовок или панель. В режиме игры питомец прыгает к кнопкам рядом с курсором и остаётся на опоре, когда курсор уходит.");
        if let Some((columns, rows)) = self.pet.custom_sprite_grid() {
            if columns == LEGACY_COLUMNS {
                ui.label("В этом PNG четыре направления. Для диагоналей нужны отдельные кадры в листе на 36 столбцов.");
            }
            if rows == LEGACY_ROWS {
                ui.label("Для пения сидя нужна девятая строка.");
            }
            if rows < ROWS {
                ui.label("Для отдельной анимации падения нужна десятая строка PNG.");
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Скопировать промпт для ChatGPT").clicked() {
                ui.output_mut(|out| out.copied_text = PROMPT.to_owned());
                self.pet.message = Some("Промпт скопирован.".into());
            }
            if ui.button("Загрузить файл питомца").clicked() {
                let tx = self.event_tx.clone();
                std::thread::spawn(move || {
                    let result =
                        crate::local_media::pick_file("Выберите спрайт питомца", "PNG | *.png");
                    let _ = tx.send(AppEvent::PetSpriteSelected(result));
                });
            }
            if self.settings.pet.custom && ui.button("Вернуть встроенного питомца").clicked()
            {
                self.settings.pet.custom = false;
                self.pet.texture = None;
                self.pet.sprite_attempted = false;
                changed = true;
            }
        });
        for file in ui.input(|input| input.raw.dropped_files.clone()) {
            if let Some(path) = file
                .path
                .filter(|p| crate::local_media::extension(p) == "png")
            {
                self.import_pet_sprite(&path, ui.ctx());
            }
        }
        if let Some(message) = &self.pet.message {
            ui.label(message);
        }
        if changed {
            self.settings.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, RawInput};

    #[test]
    fn sitting_stops_on_last_frame_and_diagonals_are_distinct() {
        assert_eq!(sprite_frame(Pose::Sit, 0.0), 0);
        assert_eq!(sprite_frame(Pose::Sit, 1.0), 3);
        assert_eq!(sprite_frame(Pose::Sit, 10.0), 3);
        assert_eq!(sprite_direction(Vec2::ZERO, COLUMNS), 8);
        for (look, expected) in [
            ((1.0, 1.0), 1),
            ((-1.0, 1.0), 3),
            ((-1.0, -1.0), 5),
            ((1.0, -1.0), 7),
        ] {
            assert_eq!(
                sprite_direction(Vec2::new(look.0, look.1), COLUMNS),
                expected
            );
        }
    }

    #[test]
    fn sprite_grid_supports_older_sheets_and_the_seated_singing_row() {
        assert_eq!(sprite_grid(3456, 864), Some((36, 9)));
        assert_eq!(sprite_grid(3456, 768), Some((36, 8)));
        assert_eq!(sprite_grid(1536, 768), Some((16, 8)));
        assert_eq!(sprite_grid(1536, 864), Some((16, 9)));
        assert_eq!(sprite_grid(3456, 863), None);
        assert_eq!(sprite_grid(3456, 960), Some((36, 10)));
        assert_eq!(Pose::SitSing as u32, SEATED_SINGING_ROWS - 1);
        assert_eq!(Pose::Fall as u32, ROWS - 1);
        assert_eq!(sprite_frame(Pose::Fall, 0.25), 2);
        assert_eq!(sprite_frame(Pose::Fall, 1.0), 3);
        assert_eq!(sprite_frame(Pose::SitSing, 0.25), 2);
        assert_eq!(sprite_frame(Pose::SitSing, 0.5), 0);
    }

    #[test]
    fn game_jumps_onto_a_nearby_control() {
        let ctx = egui::Context::default();
        let mut pet = Pet {
            game: true,
            next_probe: f64::INFINITY,
            ..Pet::default()
        };
        let mut settings = PetSettings {
            enabled: true,
            ..PetSettings::default()
        };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let surface = Rect::from_min_size(Pos2::new(400.0, 240.0), Vec2::new(65.0, 30.0));
        let target = Id::new("game-test-control");
        let cursor = surface.right_center() + Vec2::new(35.0, 0.0);
        let mut time = 0.0;
        let mut render = |pet: &mut Pet, settings: &mut PetSettings, cursor| {
            time += 0.1;
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events: vec![Event::PointerMoved(cursor)],
                    ..Default::default()
                },
                |ctx| {
                    clear_surfaces(ctx);
                    register_surface(ctx, target, surface, true);
                    pet.render(ctx, settings, false);
                },
            );
        };
        render(&mut pet, &mut settings, cursor);
        let landing = pet.jump.as_ref().expect("starts jump towards button").to;
        for _ in 0..18 {
            render(&mut pet, &mut settings, cursor);
        }
        for _ in 0..8 {
            render(&mut pet, &mut settings, Pos2::new(15.0, 70.0));
        }
        assert_eq!(pet.perch.map(|(id, _)| id), Some(target));
        assert!(pet.pos.unwrap().distance(landing) < 0.1);
        assert!(pet.jump.is_none());
    }

    #[test]
    fn pet_falls_onto_first_support_below_or_climbs_back_from_floor() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 500.0));
        let missing = Id::new("removed control");
        let surface = |name, x, y| Surface {
            id: Id::new(name),
            rect: Rect::from_min_size(Pos2::new(x, y), Vec2::new(60.0, 24.0)),
            button: true,
            parent: None,
        };
        let nearest_to_old = Surface {
            button: false,
            ..surface("replacement platform", 245.0, 150.0)
        };
        let nearest_to_floor = surface("lower side control", 245.0, 350.0);
        let directly_below = surface("directly under pet", 170.0, 270.0);
        let second_below = surface("farther down", 170.0, 410.0);
        for catches in [true, false] {
            let ctx = egui::Context::default();
            let bounds = Rect::from_min_max(screen.min, screen.max - Vec2::splat(SIDE));
            let start = Pos2::new(171.0, 120.0);
            let mut pos = start;
            let mut pet = Pet {
                perch: Some((missing, 0.5)),
                ..Pet::default()
            };
            let original = Surface {
                id: missing,
                button: false,
                ..surface("previous platform", 345.0, 230.0)
            };
            let mut surfaces = if catches {
                vec![nearest_to_old, second_below, directly_below]
            } else {
                vec![nearest_to_floor, nearest_to_old]
            };
            let mut lowest = start.y;
            let mut caught = false;
            let mut animated_fall = false;
            let mut ran = false;
            for frame in 1..=180 {
                let _ = ctx.run(
                    RawInput {
                        screen_rect: Some(screen),
                        time: Some(f64::from(frame) / 30.0),
                        focused: false,
                        ..Default::default()
                    },
                    |ctx| {
                        let pose = pet.move_on_platforms(ctx, &surfaces, &mut pos, bounds, SIDE);
                        animated_fall |= pose == Some(Pose::Fall);
                        ran |= pose == Some(Pose::Run);
                    },
                );
                lowest = lowest.max(pos.y);
                if catches && !caught && pet.perch.is_some_and(|(id, _)| id == directly_below.id) {
                    caught = true;
                    surfaces.push(original);
                }
            }
            let expected = if catches { original } else { nearest_to_old };
            assert!(animated_fall && ran);
            assert_eq!(caught, catches);
            assert_eq!(pet.perch.map(|(id, _)| id), Some(expected.id));
            let expected_anchor = if catches {
                0.5
            } else {
                expected.anchor(start.x + SIDE / 2.0)
            };
            assert_eq!(pos, expected.landing(expected_anchor, SIDE));
            let lowest_expected = if catches {
                directly_below.landing(0.5, SIDE).y
            } else {
                bounds.bottom()
            };
            assert_eq!(lowest, lowest_expected);
        }
    }

    #[test]
    fn closing_full_player_lyrics_keeps_the_pet_on_the_replacement_cover() {
        let ctx = egui::Context::default();
        let panel_id = Id::new("player content");
        let row_id = Id::new("player lyric row");
        let mut pet = Pet {
            perch: Some((row_id, 0.5)),
            ..Default::default()
        };
        let panel = Rect::from_min_size(Pos2::new(100.0, 120.0), Vec2::new(200.0, 180.0));
        register_surface(&ctx, panel_id, panel, false);
        register_platform(
            &ctx,
            Surface {
                id: row_id,
                rect: Rect::from_min_size(Pos2::new(140.0, 200.0), Vec2::new(80.0, 24.0)),
                button: false,
                parent: Some(panel_id),
            },
        );
        pet.keep_on_replaced_panel(&ctx, panel_id);
        assert_eq!(pet.perch, Some((panel_id, 0.4)));

        let cover = Surface {
            id: panel_id,
            rect: panel.translate(Vec2::new(0.0, 15.0)),
            button: false,
            parent: None,
        };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let mut pos = Pos2::new(150.0, 150.0);
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                let bounds = Rect::from_min_max(screen.min, screen.max - Vec2::splat(SIDE));
                assert!(pet
                    .move_on_platforms(ctx, &[cover], &mut pos, bounds, SIDE)
                    .is_none());
            },
        );
        assert_eq!(pos, cover.landing(0.4, SIDE));
        assert!(pet.recovery.is_none());
    }

    #[test]
    fn falling_pet_requests_motion_frames_without_focus() {
        let ctx = egui::Context::default();
        let mut pet = Pet {
            pos: Some(Pos2::new(120.0, 80.0)),
            perch: Some((Id::new("removed"), 0.5)),
            was_enabled: true,
            next_probe: f64::INFINITY,
            ..Pet::default()
        };
        let mut settings = PetSettings {
            enabled: true,
            ..PetSettings::default()
        };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 800.0));
        for frame in 1..=5 {
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(screen),
                    time: Some(f64::from(frame) / 30.0),
                    focused: false,
                    events: vec![Event::PointerGone],
                    ..Default::default()
                },
                |ctx| {
                    pet.render(ctx, &mut settings, false);
                },
            );
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                    <= std::time::Duration::from_millis(33)
            );
        }
        assert!(pet.pose.is_some_and(|(pose, _)| pose == Pose::Fall));
    }

    #[test]
    fn dragging_pet_onto_button_preserves_standing_without_clicking_button() {
        check_pet_drop(false);
    }

    #[test]
    fn dragging_seated_pet_onto_a_lyric_anchors_it_to_the_line() {
        check_pet_drop(true);
    }

    fn check_pet_drop(onto_lyric: bool) {
        let ctx = egui::Context::default();
        let mut pet = Pet {
            next_probe: f64::INFINITY,
            ..Pet::default()
        };
        let mut settings = PetSettings {
            enabled: true,
            sitting: onto_lyric,
            ..PetSettings::default()
        };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let mut time = 0.0;
        let mut target = Rect::NOTHING;
        let mut target_id = Id::NULL;
        let mut button_clicked = false;
        let mut frame = |pet: &mut Pet, settings: &mut PetSettings, events: Vec<Event>| {
            time += 0.1;
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..RawInput::default()
                },
                |ctx| {
                    clear_surfaces(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.add_space(70.0);
                        if onto_lyric {
                            register_surface(
                                ctx,
                                Id::new("pet-lyrics-panel"),
                                ui.max_rect(),
                                false,
                            );
                            let response = egui::Frame::none()
                                .inner_margin(egui::Margin::same(6.0))
                                .show(ui, |ui| ui.label("A highlighted lyric"))
                                .response;
                            register_lyric_row(ui, Some("track"), 0, &response, true, None);
                            target_id =
                                ui.make_persistent_id(("pet-lyric-row", Some("track"), 0usize));
                            target = response.rect;
                        } else {
                            let response = ui.button("Perch");
                            target_id = response.id;
                            target = response.rect;
                            button_clicked |= response.clicked();
                        }
                    });
                    pet.render(ctx, settings, false);
                },
            );
            target.center() + Vec2::new(target.width() * 0.25, 0.0)
        };
        let mut destination = Pos2::ZERO;
        for _ in 0..7 {
            destination = frame(&mut pet, &mut settings, vec![]);
        }
        let start = pet.pos.unwrap() + Vec2::splat(SIDE / 2.0);
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(
            &mut pet,
            &mut settings,
            vec![Event::PointerMoved(start), press(start, true)],
        );
        frame(
            &mut pet,
            &mut settings,
            vec![Event::PointerMoved(start + Vec2::new(-30.0, -15.0))],
        );
        frame(
            &mut pet,
            &mut settings,
            vec![Event::PointerMoved(destination)],
        );
        frame(&mut pet, &mut settings, vec![press(destination, false)]);
        assert!(!button_clicked);
        assert_eq!(settings.sitting, onto_lyric);
        let (id, anchor) = pet.perch.expect("pet attached to the drop target");
        assert_eq!(id, target_id);
        let expected_anchor = if onto_lyric { 0.75 } else { 0.5 };
        assert!((anchor - expected_anchor).abs() < 0.001);
    }
}
