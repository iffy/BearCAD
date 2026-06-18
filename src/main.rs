//! LE3 — early prototype GUI.
//!
//! First slice (SPEC §11): an egui window with a 2D sketch viewport where you
//! drag to create rectangles, plus Save/Open to a `.le3` SQLite file. Lots of
//! the spec (3D viewport, action DAG, parameters, constraints) is not here yet;
//! this is the smallest thing that draws, persists, and reloads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod model;
mod storage;

use eframe::egui;
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

#[derive(Default)]
struct App {
    doc: Document,
    /// Path of the currently open file, if it has been saved/opened.
    path: Option<String>,
    /// In-progress drag, in sketch coordinates: (start, current).
    drag: Option<(egui::Pos2, egui::Pos2)>,
    /// Transient status line shown at the bottom.
    status: String,
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
                if ui.button("Clear").clicked() {
                    self.doc.rects.clear();
                }
                if ui.button("Undo last").clicked() {
                    self.doc.rects.pop();
                }
                ui.separator();
                ui.label("Drag in the viewport to draw a rectangle.");
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

impl App {
    fn draw_viewport(&mut self, ui: &mut egui::Ui) {
        // Claim the whole panel as an interactive canvas.
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let origin = response.rect.min;

        // Background grid so the sketch plane reads as a workspace.
        painter.rect_filled(response.rect, 0.0, egui::Color32::from_gray(30));
        draw_grid(&painter, response.rect, 20.0);

        // Sketch coordinates are screen position relative to the canvas origin
        // (no pan/zoom yet), so the math here is intentionally trivial.
        let to_sketch = |p: egui::Pos2| egui::pos2(p.x - origin.x, p.y - origin.y);
        let to_screen = |x: f32, y: f32| egui::pos2(x + origin.x, y + origin.y);

        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() {
                self.drag = Some((to_sketch(pos), to_sketch(pos)));
            } else if response.dragged() {
                if let Some((_, cur)) = &mut self.drag {
                    *cur = to_sketch(pos);
                }
            }
        }

        if response.drag_stopped() {
            if let Some((start, end)) = self.drag.take() {
                let rect = Rect::from_corners(start.x, start.y, end.x, end.y);
                // Ignore accidental click-sized rectangles.
                if rect.w > 2.0 && rect.h > 2.0 {
                    self.doc.rects.push(rect);
                    self.status = format!("Added rectangle ({:.0} × {:.0} mm)", rect.w, rect.h);
                }
            }
        }

        // Committed rectangles.
        for r in &self.doc.rects {
            let screen = egui::Rect::from_min_size(to_screen(r.x, r.y), egui::vec2(r.w, r.h));
            painter.rect_filled(screen, 0.0, egui::Color32::from_rgb(70, 110, 180).gamma_multiply(0.4));
            painter.rect_stroke(
                screen,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 170, 240)),
            );
        }

        // Live preview of the in-progress drag.
        if let Some((start, end)) = self.drag {
            let r = Rect::from_corners(start.x, start.y, end.x, end.y);
            let screen = egui::Rect::from_min_size(to_screen(r.x, r.y), egui::vec2(r.w, r.h));
            painter.rect_stroke(
                screen,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(240, 200, 120)),
            );
        }
    }
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, step: f32) {
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(45));
    let mut x = rect.min.x;
    while x <= rect.max.x {
        painter.line_segment(
            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
            stroke,
        );
        x += step;
    }
    let mut y = rect.min.y;
    while y <= rect.max.y {
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            stroke,
        );
        y += step;
    }
}
