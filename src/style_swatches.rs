//! Docs style-swatch generator (#160): renders each viewport style (line kinds × states,
//! points, faces, bodies) directly into PNGs for the documentation's Styles page, using the
//! real color constants from the rendering code — the swatches can never drift from the app.
//! Screenshots can't capture hover states deterministically (scripted pointer moves don't
//! reach egui, #130), so these are drawn synthetically instead.
//!
//! Regenerate with `cargo test generate_style_swatches -- --ignored`
//! (scripts/gen-doc-screenshots.sh runs it alongside the screenshot scripts).

#![cfg_attr(not(test), allow(dead_code))]

use eframe::egui::Color32;

const SWATCH_W: u32 = 240;
const SWATCH_H: u32 = 56;

struct Canvas {
    img: image::RgbaImage,
}

impl Canvas {
    fn new() -> Self {
        let bg = crate::theme::VIEWPORT_BG;
        Self {
            img: image::RgbaImage::from_pixel(
                SWATCH_W,
                SWATCH_H,
                image::Rgba([bg.r(), bg.g(), bg.b(), 255]),
            ),
        }
    }

    /// Alpha-blend one pixel.
    fn blend(&mut self, x: i32, y: i32, color: Color32) {
        if x < 0 || y < 0 || x >= SWATCH_W as i32 || y >= SWATCH_H as i32 {
            return;
        }
        let dst = self.img.get_pixel_mut(x as u32, y as u32);
        let a = color.a() as f32 / 255.0;
        for (d, s) in dst.0.iter_mut().take(3).zip([color.r(), color.g(), color.b()]) {
            *d = (*d as f32 * (1.0 - a) + s as f32 * a) as u8;
        }
    }

    /// Horizontal stroke centered at `y`, `thickness` px tall, spanning `x0..x1`; dashes of
    /// `dash` px separated by `gap` px (dash = 0 draws solid).
    fn hline(&mut self, x0: i32, x1: i32, y: i32, thickness: i32, color: Color32, dash: i32, gap: i32) {
        for x in x0..x1 {
            if dash > 0 && ((x - x0) % (dash + gap)) >= dash {
                continue;
            }
            for dy in -(thickness / 2)..=(thickness / 2) {
                self.blend(x, y + dy, color);
            }
        }
    }

    fn disc(&mut self, cx: i32, cy: i32, radius: f32, color: Color32) {
        let r = radius.ceil() as i32 + 1;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= radius {
                    self.blend(cx + dx, cy + dy, color);
                } else if d <= radius + 1.0 {
                    let mut edge = color;
                    edge[3] = (color.a() as f32 * (radius + 1.0 - d)) as u8;
                    self.blend(cx + dx, cy + dy, edge);
                }
            }
        }
    }

    fn ring(&mut self, cx: i32, cy: i32, radius: f32, thickness: f32, color: Color32) {
        let r = (radius + thickness).ceil() as i32 + 1;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if (d - radius).abs() <= thickness / 2.0 {
                    self.blend(cx + dx, cy + dy, color);
                }
            }
        }
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color32) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, color);
            }
        }
    }

    fn rect_outline(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thickness: i32, color: Color32) {
        self.fill_rect(x0, y0, x1, y0 + thickness, color);
        self.fill_rect(x0, y1 - thickness, x1, y1, color);
        self.fill_rect(x0, y0, x0 + thickness, y1, color);
        self.fill_rect(x1 - thickness, y0, x1, y1, color);
    }

    fn save(self, dir: &std::path::Path, name: &str) {
        self.img
            .save(dir.join(format!("{name}.png")))
            .expect("write swatch png");
    }
}

/// How a line swatch's state decorates the base stroke.
enum LineState {
    Normal,
    /// Pick hover: the highlight redraws the segment in the hover color (4 px) with
    /// endpoint discs, over the base stroke — mirrors `push_pick_target_highlight`.
    Hovered,
    /// Selection redraws the segment in the selection-highlight color (3 px) — mirrors
    /// `push_selection`.
    Selected,
}

fn line_swatch(dir: &std::path::Path, name: &str, base: Color32, dashed: bool, state: LineState) {
    let palette = crate::gpu_viewport::ViewportPalette::default();
    let (x0, x1, y) = (24, SWATCH_W as i32 - 24, SWATCH_H as i32 / 2);
    let (dash, gap) = if dashed { (8, 6) } else { (0, 0) };
    let mut canvas = Canvas::new();
    canvas.hline(x0, x1, y, 2, base, dash, gap);
    match state {
        LineState::Normal => {}
        LineState::Hovered => {
            let hover = crate::construction::PICK_HOVER_RGBA;
            canvas.hline(x0, x1, y, 4, hover, dash, gap);
            canvas.disc(x0, y, 5.0, hover);
            canvas.disc(x1, y, 5.0, hover);
        }
        LineState::Selected => {
            canvas.hline(x0, x1, y, 3, palette.dim_edge_highlight, dash, gap);
        }
    }
    canvas.save(dir, name);
}

pub fn generate(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create swatch dir");
    let palette = crate::gpu_viewport::ViewportPalette::default();

    // Lines: 4 kinds x 3 states.
    let kinds: [(&str, Color32, bool); 4] = [
        ("line-normal", palette.rect_line, false),
        ("line-constrained", palette.rect_line_constrained, false),
        ("line-construction", palette.construction, true),
        ("line-projected", palette.projection, true),
    ];
    for (name, color, dashed) in kinds {
        line_swatch(dir, name, color, dashed, LineState::Normal);
        line_swatch(dir, &format!("{name}-hovered"), color, dashed, LineState::Hovered);
        line_swatch(dir, &format!("{name}-selected"), color, dashed, LineState::Selected);
    }

    // Points (line endpoints / circle centers): hover ring+disc, selected disc.
    let (cx, cy) = (SWATCH_W as i32 / 2, SWATCH_H as i32 / 2);
    let mut canvas = Canvas::new();
    canvas.hline(cx - 60, cx + 60, cy, 2, palette.rect_line, 0, 0);
    canvas.disc(cx, cy, 3.0, palette.rect_line);
    canvas.save(dir, "point-normal");
    let mut canvas = Canvas::new();
    canvas.hline(cx - 60, cx + 60, cy, 2, palette.rect_line, 0, 0);
    canvas.disc(cx, cy, 6.0, crate::construction::PICK_HOVER_RGBA);
    canvas.ring(cx, cy, 6.0, 2.0, crate::construction::PICK_HOVER_RGBA);
    canvas.save(dir, "point-hovered");
    let mut canvas = Canvas::new();
    canvas.hline(cx - 60, cx + 60, cy, 2, palette.rect_line, 0, 0);
    canvas.disc(cx, cy, 6.0, palette.dim_edge_highlight);
    canvas.save(dir, "point-selected");

    // Faces: the hover fill tint over a body face (mirrors FACE_HOVER_FILL_MULTIPLIER).
    let face = crate::gpu_viewport::SOLID_FILL;
    let mut canvas = Canvas::new();
    canvas.fill_rect(24, 10, SWATCH_W as i32 - 24, SWATCH_H as i32 - 10, face);
    canvas.save(dir, "face-normal");
    let mut canvas = Canvas::new();
    canvas.fill_rect(24, 10, SWATCH_W as i32 - 24, SWATCH_H as i32 - 10, face);
    let mut tint = crate::construction::PICK_HOVER_RGBA;
    tint[3] = (255.0 * crate::construction::FACE_HOVER_FILL_MULTIPLIER) as u8;
    canvas.fill_rect(24, 10, SWATCH_W as i32 - 24, SWATCH_H as i32 - 10, tint);
    canvas.rect_outline(24, 10, SWATCH_W as i32 - 24, SWATCH_H as i32 - 10, 2, crate::construction::PICK_HOVER_RGBA);
    canvas.save(dir, "face-hovered");

    // Bodies: normal fill, hovered aura (pane hover, hover color), selected aura (blue).
    let body_rect = (60, 14, SWATCH_W as i32 - 60, SWATCH_H as i32 - 14);
    let mut canvas = Canvas::new();
    canvas.fill_rect(body_rect.0, body_rect.1, body_rect.2, body_rect.3, face);
    canvas.save(dir, "body-normal");
    for (name, aura) in [
        ("body-hovered", crate::construction::PICK_HOVER_RGBA),
        ("body-selected", crate::gpu_viewport::BODY_SILHOUETTE_COLOR),
    ] {
        let mut canvas = Canvas::new();
        canvas.fill_rect(body_rect.0, body_rect.1, body_rect.2, body_rect.3, face);
        // The aura sits offset *outside* the silhouette (#145).
        canvas.rect_outline(
            body_rect.0 - 6,
            body_rect.1 - 6,
            body_rect.2 + 6,
            body_rect.3 + 6,
            4,
            aura,
        );
        canvas.save(dir, name);
    }
}

#[cfg(test)]
mod tests {
    /// Regenerates the documentation style swatches (#160). Ignored by default: it writes
    /// into docs-site/, so it runs from scripts/gen-doc-screenshots.sh (and by hand), not CI
    /// unit-test runs.
    #[test]
    #[ignore]
    fn generate_style_swatches() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs-site/static/img/screenshots/styles");
        super::generate(&dir);
        assert!(dir.join("line-normal.png").exists());
    }
}
