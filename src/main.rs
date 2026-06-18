//! LE3 — early prototype GUI.
//!
//! Rectangle tool: click to fix first corner, move mouse for second, with live
//! dimension inputs on the sides. Type to constrain a side, Tab to cycle,
//! Enter to commit. Right-drag orbit, wheel zoom. Save/Open .le3. (prototype)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod camera;
mod model;
mod storage;

use camera::Camera;
use eframe::egui;
use glam::Vec3;
use model::{Document, Rect};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("LE3")
            // Pass an explicitly-empty icon. Otherwise eframe installs a default
            // egui icon via AppKit's NSImage, which faults in ImageIO (SIGBUS)
            // on macOS here. An empty IconData makes eframe skip that path.
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("LE3", options, Box::new(|_cc| Ok(Box::<App>::default())))
}

/// The active viewport tool.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tool {
    /// Orbit/zoom only; no drawing.
    #[default]
    Select,
    /// Click to fix first corner of rectangle; move to position opposite corner;
    /// on-screen number inputs allow typing constraints; Enter commits.
    Rectangle,
}

/// State for the in-progress (pre-Enter) rectangle creation.
#[derive(Clone, Debug)]
struct CreatingRect {
    /// Fixed first corner in ground coords.
    origin: Vec3,
    /// Text content of the two dimension inputs (width, height).
    /// A parseable positive number locks/constrains that side.
    texts: [String; 2],
    /// 0 = width (horiz side), 1 = height (vert side)
    focused: usize,
    /// Current mouse projected ground point (drives free dimension + signs).
    last_mouse: Vec3,
    /// Tracks whether user has typed into each field; prevents live-mouse overwrite
    /// of user input (including mid-edit partials like "12.").
    user_edited: [bool; 2],
}

struct App {
    doc: Document,
    /// Path of the currently open file, if it has been saved/opened.
    path: Option<String>,
    tool: Tool,
    cam: Camera,
    /// In-progress rectangle creation (pre-commit with dimension inputs).
    creating_rect: Option<CreatingRect>,
    /// Transient status line shown at the bottom.
    status: String,
}

impl Default for App {
    fn default() -> Self {
        App {
            doc: Document::default(),
            path: None,
            tool: Tool::default(),
            cam: Camera::default(),
            creating_rect: None,
            status: String::new(),
        }
    }
}

impl App {
    fn save_as(&mut self) {
        let start = rfd::FileDialog::new()
            .add_filter("LE3 document", &["le3"])
            .set_file_name("untitled.le3");
        if let Some(path) = start.save_file() {
            let path = path.to_string_lossy().to_string();
            self.write_to(&path);
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(path) => self.write_to(&path),
            None => self.save_as(),
        }
    }

    fn write_to(&mut self, path: &str) {
        match storage::save(path, &self.doc) {
            Ok(()) => {
                self.path = Some(path.to_string());
                self.status = format!("Saved {} rectangle(s) to {}", self.doc.rects.len(), path);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn open(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("LE3 document", &["le3"])
            .pick_file();
        if let Some(path) = picked {
            let path = path.to_string_lossy().to_string();
            match storage::open(&path) {
                Ok(doc) => {
                    self.doc = doc;
                    self.status = format!("Opened {} ({} rectangle(s))", path, self.doc.rects.len());
                    self.path = Some(path);
                }
                Err(e) => self.status = format!("Open failed: {e}"),
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Escape: first cancel an in-progress operation; if none, fall back to
        // the Select tool.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.creating_rect.take().is_some() {
                self.status = "Cancelled".to_string();
            } else if self.tool != Tool::Select {
                self.tool = Tool::Select;
                self.status = "Select tool".to_string();
            }
        }

        // Tool shortcuts (only when not mid-creation, so 'r' doesn't interfere with typing dimensions)
        if self.creating_rect.is_none() && ctx.input(|i| i.key_pressed(egui::Key::R)) {
            if self.tool != Tool::Rectangle {
                self.tool = Tool::Rectangle;
                self.status = "Rectangle tool".to_string();
            }
        }

        // While Rectangle tool is not active, discard any in-progress creation.
        if self.tool != Tool::Rectangle {
            self.creating_rect = None;
        }

        // Consume navigation keys while creating so TextEdits don't act on Tab/Enter.
        let (enter_pressed, tab_pressed) = if self.creating_rect.is_some() {
            (
                ctx.input(|i| i.key_pressed(egui::Key::Enter)),
                ctx.input(|i| i.key_pressed(egui::Key::Tab)),
            )
        } else {
            (false, false)
        };
        if enter_pressed {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        if tab_pressed {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        }

        // Apply Tab/Enter for rectangle creation (after consume).
        if let Some(cr) = &mut self.creating_rect {
            if tab_pressed {
                cr.focused = 1 - cr.focused;
                let target_id = if cr.focused == 0 {
                    egui::Id::new("cr_width")
                } else {
                    egui::Id::new("cr_height")
                };
                ctx.memory_mut(|m| m.request_focus(target_id));
            }
            if enter_pressed {
                let end = cr.end_point();
                let rect = Rect::from_corners(cr.origin.x, cr.origin.y, end.x, end.y);
                if rect.w > 0.5 && rect.h > 0.5 {
                    self.doc.rects.push(rect);
                    self.status =
                        format!("Added rectangle ({:.1} × {:.1} mm)", rect.w, rect.h);
                } else {
                    self.status = "Rectangle too small".to_string();
                }
                self.creating_rect = None;
            }
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("New").clicked() {
                    self.doc = Document::default();
                    self.path = None;
                    self.status = "New document".to_string();
                }
                if ui.button("Open…").clicked() {
                    self.open();
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
                if ui.button("Save As…").clicked() {
                    self.save_as();
                }
                ui.separator();
                // Tool selection.
                ui.selectable_value(&mut self.tool, Tool::Select, "Select");
                ui.selectable_value(&mut self.tool, Tool::Rectangle, "Rectangle");
                ui.separator();
                if ui.button("Clear").clicked() {
                    self.doc.rects.clear();
                }
                if ui.button("Undo last").clicked() {
                    self.doc.rects.pop();
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let name = self.path.as_deref().unwrap_or("(unsaved)");
            ui.horizontal(|ui| {
                ui.label(name);
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_viewport(ui);
        });
    }
}

/// Colours used in the viewport.
mod col {
    use egui::Color32;
    pub const BG: Color32 = Color32::from_gray(28);
    pub const GRID: Color32 = Color32::from_gray(55);
    pub const GRID_AXIS: Color32 = Color32::from_gray(90);
    pub const X_AXIS: Color32 = Color32::from_rgb(200, 70, 70);
    pub const Y_AXIS: Color32 = Color32::from_rgb(70, 190, 90);
    pub const RECT_LINE: Color32 = Color32::from_rgb(120, 170, 240);
    pub const PREVIEW: Color32 = Color32::from_rgb(240, 200, 120);
}

const GRID_EXTENT: f32 = 200.0;
const GRID_STEP: f32 = 20.0;

impl App {
    fn draw_viewport(&mut self, ui: &mut egui::Ui) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let viewport = response.rect;
        painter.rect_filled(viewport, 0.0, col::BG);

        // --- Camera input (orbit / zoom) ---------------------------------
        if response.dragged_by(egui::PointerButton::Secondary) {
            if ui.input(|i| i.modifiers.shift) {
                self.cam.pan(response.drag_delta(), viewport.height());
            } else {
                self.cam.orbit(response.drag_delta());
            }
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                self.cam.zoom(scroll);
            }
        }

        // Project with the camera as it stands this frame (after input).
        let vp = self.cam.view_proj(viewport);
        let project = |w: Vec3| self.cam.project(w, viewport, &vp);

        // --- Rectangle creation (click first corner, live preview + inputs, Enter to commit)
        if self.tool == Tool::Rectangle {
            let ground = |p: egui::Pos2| self.cam.ground_point(p, viewport, &vp);

            // Use hover (or interact) pos so the preview follows the mouse continuously
            // after the initial click, even without holding the button.
            let pointer_screen = response.hover_pos().or(response.interact_pointer_pos());

            if let Some(pp) = pointer_screen {
                if let Some(gp) = ground(pp) {
                    // On primary press, fix the first corner (origin).
                    if self.creating_rect.is_none() && ui.input(|i| i.pointer.primary_pressed()) {
                        self.creating_rect = Some(CreatingRect {
                            origin: gp,
                            texts: ["".to_string(), "".to_string()],
                            focused: 0,
                            last_mouse: gp,
                            user_edited: [false, false],
                        });
                        self.status = "Move mouse • type to lock dim • Tab cycle • Enter commit • Esc cancel".to_string();
                        // Seed focus request early (widget will be created later this frame).
                        ui.ctx().memory_mut(|m| m.request_focus(egui::Id::new("cr_width")));
                    }

                    // Continuously update the live mouse position for preview + free dimension,
                    // *unless* the pointer is over one of the dimension input fields. Freezing
                    // the preview when over an input lets the user click/focus/type in the
                    // (now-stationary) fields.
                    if let Some(cr) = &mut self.creating_rect {
                        let cur_end = cr.end_point();
                        let x0 = cr.origin.x.min(cur_end.x);
                        let y0 = cr.origin.y.min(cur_end.y);
                        let x1 = cr.origin.x.max(cur_end.x);
                        let y1 = cr.origin.y.max(cur_end.y);
                        let mid_w = Vec3::new((x0 + x1) * 0.5, y0, 0.0);
                        let mid_h = Vec3::new(x0, (y0 + y1) * 0.5, 0.0);
                        let pw = project(mid_w);
                        let ph = project(mid_h);

                        let mut over_input = false;
                        if let (Some(pw), Some(ph)) = (pw, ph) {
                            let r_w = egui::Rect::from_min_size(
                                pw + egui::vec2(-20.0, 14.0),
                                egui::vec2(55.0, 20.0),
                            );
                            let r_h = egui::Rect::from_min_size(
                                ph + egui::vec2(-48.0, -4.0),
                                egui::vec2(55.0, 20.0),
                            );
                            if r_w.contains(pp) || r_h.contains(pp) {
                                over_input = true;
                            }
                        }

                        if !over_input {
                            cr.last_mouse = gp;
                            let rw = (gp.x - cr.origin.x).abs();
                            let rh = (gp.y - cr.origin.y).abs();
                            let fm = |v: f32| -> String {
                                if v < 0.1 { "0".to_string() } else { format!("{:.1}", v) }
                            };
                            if !cr.user_edited[0] {
                                cr.texts[0] = fm(rw);
                            }
                            if !cr.user_edited[1] {
                                cr.texts[1] = fm(rh);
                            }
                        }
                        // else: do not change last_mouse; inputs stay put for interaction
                    }
                }
            }
        }

        // --- Draw scene ---------------------------------------------------
        draw_ground(&painter, &project);

        for r in &self.doc.rects {
            draw_rect(&painter, &project, *r, col::RECT_LINE, true);
        }
        // Live preview while creating (uses constrained or mouse values).
        if let Some(cr) = &self.creating_rect {
            let end = cr.end_point();
            let preview = Rect::from_corners(cr.origin.x, cr.origin.y, end.x, end.y);
            draw_rect(&painter, &project, preview, col::PREVIEW, false);
            if let Some(sp) = project(cr.origin) {
                painter.circle_filled(sp, 3.5, col::PREVIEW);
            }
        }

        // --- Dimension inputs on the sides (only while creating) ------------
        // Two number inputs placed near perpendicular sides of the preview rect.
        // One is auto-focused; typing constrains that side; mouse affects free side(s).
        if let Some(cr) = &mut self.creating_rect {
            let end = cr.end_point();
            // Use axis-aligned bounding box of current rect for side placement.
            let x0 = cr.origin.x.min(end.x);
            let y0 = cr.origin.y.min(end.y);
            let x1 = cr.origin.x.max(end.x);
            let y1 = cr.origin.y.max(end.y);
            let mid_w = Vec3::new((x0 + x1) * 0.5, y0, 0.0); // on a horizontal side
            let mid_h = Vec3::new(x0, (y0 + y1) * 0.5, 0.0); // on a vertical side
            let pw = project(mid_w);
            let ph = project(mid_h);
            if let (Some(pw), Some(ph)) = (pw, ph) {
                let ctx = ui.ctx();

                // Explicit ids for the *TextEdit widgets* (so request_focus targets the right thing).
                let id_w = egui::Id::new("cr_width");
                let id_h = egui::Id::new("cr_height");

                // Width input (near horizontal side)
                egui::Area::new(egui::Id::new("cr_width_area"))
                    .fixed_pos(pw + egui::vec2(-20.0, 14.0))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        ui.style_mut().spacing.text_edit_width = 48.0;
                        // Make the floating inputs readable on the dark viewport.
                        ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_gray(32);
                        ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(230));
                        ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_gray(50);
                        ui.visuals_mut().widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 220, 150));
                        let te = egui::TextEdit::singleline(&mut cr.texts[0])
                            .id_source(id_w)
                            .desired_width(48.0)
                            .font(egui::FontId::proportional(11.0))
                            .margin(egui::vec2(2.0, 1.0));
                        let resp = ui.add(te);
                        if resp.changed() {
                            cr.user_edited[0] = true;
                        }
                    });

                // Height input (near vertical side)
                egui::Area::new(egui::Id::new("cr_height_area"))
                    .fixed_pos(ph + egui::vec2(-48.0, -4.0))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        ui.style_mut().spacing.text_edit_width = 48.0;
                        // Make the floating inputs readable on the dark viewport.
                        ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_gray(32);
                        ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(230));
                        ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_gray(50);
                        ui.visuals_mut().widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 220, 150));
                        let te = egui::TextEdit::singleline(&mut cr.texts[1])
                            .id_source(id_h)
                            .desired_width(48.0)
                            .font(egui::FontId::proportional(11.0))
                            .margin(egui::vec2(2.0, 1.0));
                        let resp = ui.add(te);
                        if resp.changed() {
                            cr.user_edited[1] = true;
                        }
                    });

                // Sync logical focused index if user clicked one of the fields (so Tab continues from there).
                let current = ctx.memory(|m| m.focused());
                if current == Some(id_w) {
                    cr.focused = 0;
                } else if current == Some(id_h) {
                    cr.focused = 1;
                }

                // Only force our logical focus if neither input currently has it (e.g. just started,
                // or after Tab, or focus was lost). This prevents us from stealing focus from a click.
                if current != Some(id_w) && current != Some(id_h) {
                    let target_id = if cr.focused == 0 { id_w } else { id_h };
                    ctx.memory_mut(|m| m.request_focus(target_id));
                }
            }
        }

        // --- Help overlay -------------------------------------------------
        let hint = match self.tool {
            Tool::Select => "Right-drag: orbit  •  Shift+right-drag: pan  •  Wheel: zoom  •  r: rectangle",
            Tool::Rectangle => {
                if self.creating_rect.is_some() {
                    "Move mouse (free dim) • Type in focused input to constrain • Tab: switch dims • Enter: create rect • Esc: cancel"
                } else {
                    "r: rectangle  •  Left-click to set corner • move to size • Right-drag: orbit  • Shift+right-drag: pan  •  Wheel: zoom"
                }
            }
        };
        painter.text(
            viewport.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(150),
        );
    }
}

/// The four ground-plane corners of a rectangle (z = 0), CCW.
fn rect_corners(r: Rect) -> [Vec3; 4] {
    [
        Vec3::new(r.x, r.y, 0.0),
        Vec3::new(r.x + r.w, r.y, 0.0),
        Vec3::new(r.x + r.w, r.y + r.h, 0.0),
        Vec3::new(r.x, r.y + r.h, 0.0),
    ]
}

impl CreatingRect {
    /// Current opposite corner, respecting any locked dimensions from texts.
    /// Signs (which quadrant) follow the last_mouse direction.
    fn end_point(&self) -> Vec3 {
        let dx = self.last_mouse.x - self.origin.x;
        let dy = self.last_mouse.y - self.origin.y;
        let w = if let Ok(v) = self.texts[0].trim().parse::<f32>() {
            if v > 0.0 { v } else { dx.abs() }
        } else {
            dx.abs()
        };
        let h = if let Ok(v) = self.texts[1].trim().parse::<f32>() {
            if v > 0.0 { v } else { dy.abs() }
        } else {
            dy.abs()
        };
        let sx = if dx < 0.0 { -1.0 } else { 1.0 };
        let sy = if dy < 0.0 { -1.0 } else { 1.0 };
        Vec3::new(self.origin.x + sx * w, self.origin.y + sy * h, 0.0)
    }
}

fn draw_rect(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    r: Rect,
    color: egui::Color32,
    fill: bool,
) {
    let pts: Option<Vec<egui::Pos2>> = rect_corners(r).iter().map(|&c| project(c)).collect();
    let Some(pts) = pts else { return };
    if fill {
        painter.add(egui::Shape::convex_polygon(
            pts.clone(),
            color.gamma_multiply(0.25),
            egui::Stroke::new(1.5, color),
        ));
    } else {
        painter.add(egui::Shape::closed_line(
            pts,
            egui::Stroke::new(1.5, color),
        ));
    }
}

/// Draw the XY ground grid plus highlighted X and Y axes.
fn draw_ground(painter: &egui::Painter, project: &impl Fn(Vec3) -> Option<egui::Pos2>) {
    let e = GRID_EXTENT;
    let line = |a: Vec3, b: Vec3, color: egui::Color32, w: f32| {
        if let (Some(pa), Some(pb)) = (project(a), project(b)) {
            painter.line_segment([pa, pb], egui::Stroke::new(w, color));
        }
    };

    let mut t = -e;
    while t <= e + 0.001 {
        // Lines parallel to X (varying y) and to Y (varying x).
        let color = if t.abs() < 0.001 { col::GRID_AXIS } else { col::GRID };
        line(Vec3::new(-e, t, 0.0), Vec3::new(e, t, 0.0), color, 1.0);
        line(Vec3::new(t, -e, 0.0), Vec3::new(t, e, 0.0), color, 1.0);
        t += GRID_STEP;
    }

    // Coloured world axes through the origin.
    line(Vec3::ZERO, Vec3::new(e, 0.0, 0.0), col::X_AXIS, 2.0);
    line(Vec3::ZERO, Vec3::new(0.0, e, 0.0), col::Y_AXIS, 2.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn make_cr(origin: (f32, f32), texts: [&str; 2], mouse: (f32, f32)) -> CreatingRect {
        CreatingRect {
            origin: Vec3::new(origin.0, origin.1, 0.0),
            texts: [texts[0].to_string(), texts[1].to_string()],
            focused: 0,
            last_mouse: Vec3::new(mouse.0, mouse.1, 0.0),
            user_edited: [true, true], // don't care for end_point
        }
    }

    #[test]
    fn end_point_free_follows_mouse() {
        let cr = make_cr((0., 0.), ["", ""], (10., 4.));
        let e = cr.end_point();
        assert!((e.x - 10.0).abs() < 1e-4);
        assert!((e.y - 4.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_one_constrained() {
        // width locked to 5, mouse suggests 12 x 3 -> should be 5 x 3, sign from mouse
        let cr = make_cr((0., 0.), ["5", ""], (12., 3.));
        let e = cr.end_point();
        assert!((e.x - 5.0).abs() < 1e-4 && (e.y - 3.0).abs() < 1e-4);

        // negative quadrant
        let cr2 = make_cr((10., 20.), ["5", ""], (3., 15.));
        let e2 = cr2.end_point();
        // w=5 so x = 10 -5 =5 , h from mouse |15-20|=5? wait dy=15-20=-5, h=5, y=20-5=15
        assert!((e2.x - 5.0).abs() < 1e-4);
        assert!((e2.y - 15.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_both_constrained() {
        let cr = make_cr((0., 0.), ["3", "7"], (99., -4.));
        let e = cr.end_point();
        // signs from mouse: x+, y-
        assert!((e.x - 3.0).abs() < 1e-4);
        assert!((e.y + 7.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_invalid_text_falls_back_to_mouse() {
        let cr = make_cr((0., 0.), ["abc", "12x"], (8., 9.));
        let e = cr.end_point();
        assert!((e.x - 8.0).abs() < 1e-4);
        assert!((e.y - 9.0).abs() < 1e-4);
    }
}
