//! LE3 — early prototype GUI.
//!
//! Current slice (SPEC §11): an egui window with a 3D viewport. Pick the
//! Rectangle tool and left-drag to draw rectangles on the ground plane (XY,
//! z = 0); orbit with right-drag and zoom with the mouse wheel. Save/Open to a
//! `.le3` SQLite file. The 3D scene is projected with egui's painter for now;
//! the wgpu/OCCT pipeline (SPEC §1, §10) comes later.

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
    /// Left-drag draws a rectangle on the ground plane.
    Rectangle,
}

struct App {
    doc: Document,
    /// Path of the currently open file, if it has been saved/opened.
    path: Option<String>,
    tool: Tool,
    cam: Camera,
    /// In-progress rectangle drag, in ground-plane coords: (start, current).
    drag: Option<(Vec3, Vec3)>,
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
            drag: None,
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
            if self.drag.take().is_some() {
                self.status = "Cancelled".to_string();
            } else if self.tool != Tool::Select {
                self.tool = Tool::Select;
                self.status = "Select tool".to_string();
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

        // --- Rectangle drawing on the ground plane -----------------------
        if self.tool == Tool::Rectangle {
            let ground = |p: egui::Pos2| self.cam.ground_point(p, viewport, &vp);

            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(p) = response.interact_pointer_pos().and_then(ground) {
                    self.drag = Some((p, p));
                }
            } else if response.dragged_by(egui::PointerButton::Primary) {
                if let (Some((start, _)), Some(cur)) =
                    (self.drag, response.interact_pointer_pos().and_then(ground))
                {
                    self.drag = Some((start, cur));
                }
            }

            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let Some((start, end)) = self.drag.take() {
                    let rect = Rect::from_corners(start.x, start.y, end.x, end.y);
                    if rect.w > 0.5 && rect.h > 0.5 {
                        self.doc.rects.push(rect);
                        self.status =
                            format!("Added rectangle ({:.1} × {:.1} mm)", rect.w, rect.h);
                    }
                }
            }
        }

        // --- Draw scene ---------------------------------------------------
        draw_ground(&painter, &project);

        for r in &self.doc.rects {
            draw_rect(&painter, &project, *r, col::RECT_LINE, true);
        }
        if let Some((start, end)) = self.drag {
            let preview = Rect::from_corners(start.x, start.y, end.x, end.y);
            draw_rect(&painter, &project, preview, col::PREVIEW, false);
        }

        // --- Help overlay -------------------------------------------------
        let hint = match self.tool {
            Tool::Select => "Right-drag: orbit  •  Shift+right-drag: pan  •  Wheel: zoom",
            Tool::Rectangle => {
                "Left-drag: draw rectangle  •  Right-drag: orbit  •  Shift+right-drag: pan  •  Wheel: zoom"
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
