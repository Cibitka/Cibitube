use crate::media_controls::LyricsSnapshot;
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use std::time::Instant;

const TRANSITION_SECONDS: f32 = 0.45;

/// Rasterize only when the lyric or terminal size changes; animation uses the cached pixels.
pub(crate) struct Animation {
    font: FontArc,
    key: Option<(String, String, u64, usize, usize)>,
    pixels: Vec<bool>,
    previous: Vec<bool>,
    changed: Instant,
}

impl Animation {
    pub(crate) fn new() -> Self {
        let font = [
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ]
        .into_iter()
        .find_map(|path| {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| FontArc::try_from_vec(bytes).ok())
        })
        .unwrap_or_else(|| {
            let definitions = eframe::egui::FontDefinitions::default();
            FontArc::try_from_vec(definitions.font_data["Ubuntu-Light"].font.to_vec())
                .expect("the embedded interface font is valid")
        });
        Self {
            font,
            key: None,
            pixels: Vec::new(),
            previous: Vec::new(),
            changed: Instant::now(),
        }
    }

    fn prepare(&mut self, snapshot: &LyricsSnapshot, line: &str, width: usize, height: usize) {
        let key = (
            snapshot.track_id.clone(),
            line.to_owned(),
            snapshot.revision,
            width,
            height,
        );
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.previous = if self
            .key
            .as_ref()
            .is_some_and(|old| old.3 == width && old.4 == height)
        {
            std::mem::take(&mut self.pixels)
        } else {
            Vec::new()
        };
        self.pixels = rasterize(&self.font, line, width, height);
        self.key = Some(key);
        self.changed = Instant::now();
    }

    pub(crate) fn render(
        &mut self,
        snapshot: &LyricsSnapshot,
        lines: &[String],
        width: usize,
        height: usize,
    ) -> Option<String> {
        if !snapshot.available || !snapshot.synced || width < 16 || height < 8 {
            return None;
        }
        let line = snapshot
            .active
            .and_then(|index| lines.get(index))
            .map(String::as_str)
            .unwrap_or("");
        // Bound work even if a terminal reports an unusually large window.
        let columns = width.saturating_sub(2).min(320);
        let rows = height.saturating_sub(5).min(100);
        self.prepare(snapshot, line, columns, rows * 2);
        let progress = (self.changed.elapsed().as_secs_f32() / TRANSITION_SECONDS).min(1.0);
        let mut output = String::from("\x1b[H");
        output.push_str(&super::terminal_lyrics::clean_line(
            &format!("{} — {}", snapshot.title, snapshot.artist),
            width.saturating_sub(2),
        ));
        output.push_str("\x1b[K\r\n\x1b[K\r\n\x1b[1;38;2;100;220;175m");
        let left = width.saturating_sub(columns) / 2;
        for row in 0..rows {
            output.push_str(&" ".repeat(left));
            for column in 0..columns {
                let top = self.pixel(column, row * 2, columns, progress);
                let bottom = self.pixel(column, row * 2 + 1, columns, progress);
                output.push(match (top, bottom) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
            output.push_str("\x1b[K\r\n");
        }
        output.push_str("\x1b[0m\x1b[K\r\n");
        output.push_str(&super::terminal_lyrics::clean_line(
            crate::i18n::text("Ctrl+C — закрыть текст, музыка продолжит играть"),
            width.saturating_sub(2),
        ));
        output.push_str("\x1b[K\x1b[J");
        Some(output)
    }

    pub(crate) fn is_transitioning(&self) -> bool {
        self.changed.elapsed().as_secs_f32() < TRANSITION_SECONDS
    }

    fn pixel(&self, x: usize, y: usize, width: usize, progress: f32) -> bool {
        let index = y * width + x;
        // Ordered, deterministic dissolve; no random generator or extra repaint of the player.
        let noise = ((x * 73 + y * 139 + x * y * 7) % 251) as f32 / 251.0;
        let reveal = progress * 1.35 - x as f32 / width as f32 * 0.35;
        let new =
            self.pixels.get(index).copied().unwrap_or(false) && (progress >= 1.0 || noise < reveal);
        let old =
            self.previous.get(index).copied().unwrap_or(false) && noise > (progress * 2.5).min(1.0);
        new || old
    }
}

fn measure(font: &ab_glyph::PxScaleFont<&FontArc>, text: &str) -> f32 {
    let mut previous = None;
    text.chars().fold(0.0, |width, ch| {
        let id = font.glyph_id(ch);
        let kern = previous.map_or(0.0, |previous| font.kern(previous, id));
        previous = Some(id);
        width + kern + font.h_advance(id)
    })
}

fn wrap(font: &ab_glyph::PxScaleFont<&FontArc>, text: &str, width: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if measure(font, &candidate) > width && !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn rasterize(font: &FontArc, text: &str, width: usize, height: usize) -> Vec<bool> {
    let text = super::terminal_lyrics::clean_line(text, 4096).to_uppercase();
    let mut size = (height as f32 * 0.7).clamp(3.0, 32.0);
    let (scaled, rows) = loop {
        let scaled = font.as_scaled(PxScale::from(size));
        let rows = wrap(&scaled, &text, width.saturating_sub(4) as f32);
        let fits = rows.len() as f32 * scaled.height() <= height as f32
            && rows.iter().all(|row| measure(&scaled, row) <= width as f32);
        if fits || size <= 2.0 {
            break (scaled, rows);
        }
        size -= 1.0;
    };
    let mut pixels = vec![false; width * height];
    let top = (height as f32 - rows.len() as f32 * scaled.height()).max(0.0) / 2.0;
    for (row, text) in rows.iter().enumerate() {
        let mut x = (width as f32 - measure(&scaled, text)).max(0.0) / 2.0;
        let y = top + row as f32 * scaled.height() + scaled.ascent();
        let mut previous = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(previous) = previous {
                x += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(scaled.scale(), point(x, y));
            if let Some(outline) = scaled.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if coverage >= 0.3
                        && px >= 0
                        && py >= 0
                        && px < width as i32
                        && py < height as i32
                    {
                        pixels[py as usize * width + px as usize] = true;
                    }
                });
            }
            x += scaled.h_advance(id);
            previous = Some(id);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animated_text_supports_cyrillic_and_resizes_without_stale_pixels() {
        let mut animation = Animation::new();
        let snapshot = LyricsSnapshot {
            track_id: "fixture".into(),
            available: true,
            synced: true,
            active: Some(0),
            ..Default::default()
        };
        animation
            .render(&snapshot, &["Привет, мир".into()], 80, 24)
            .unwrap();
        assert!(animation.pixels.iter().any(|pixel| *pixel));
        animation
            .render(&snapshot, &["Привет, мир".into()], 40, 12)
            .unwrap();
        assert_eq!(animation.pixels.len(), 38 * 14);
        assert!(animation.previous.is_empty());
        assert!(animation.render(&snapshot, &[], 10, 4).is_none());
    }

    #[test]
    fn dissolve_finishes_with_exactly_the_new_lyric() {
        let mut animation = Animation::new();
        animation.pixels = vec![true, false, true, false];
        animation.previous = vec![false, true, false, true];
        for x in 0..4 {
            assert_eq!(animation.pixel(x, 0, 4, 1.0), animation.pixels[x]);
        }
    }
}
