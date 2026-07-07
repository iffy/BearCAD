//! In-window menu bar for the web build (wasm32): the browser has no OS menu bar, so
//! everything pertinent from the native menus ([`crate::native_menu`]) renders as an egui
//! menu strip across the top of the window instead. Items emit the same
//! [`MenuCommand`](crate::menu_command::MenuCommand)s the OS menus do; `App::
//! handle_menu_command` dispatches both.
//!
//! Deliberately omitted on web: Quit (close the tab), Install CLI (no filesystem), and
//! the native Save-vs-Save-As split (both are a download; one **Save…** covers it).

use crate::menu_command::MenuCommand;
use crate::actions::Pane;
use eframe::egui;

/// Draw the menu bar; returns the command the user picked, if any.
pub fn bar(ctx: &egui::Context, pane_visible: impl Fn(Pane) -> bool) -> Option<MenuCommand> {
    let mut picked: Option<MenuCommand> = None;
    egui::TopBottomPanel::top("web_menu_bar").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    picked = Some(MenuCommand::NewDocument);
                    ui.close();
                }
                if ui.button("Open…").clicked() {
                    picked = Some(MenuCommand::Open);
                    ui.close();
                }
                if ui.button("Save…").clicked() {
                    picked = Some(MenuCommand::Save);
                    ui.close();
                }
                if ui.button("Load Script…").clicked() {
                    picked = Some(MenuCommand::LoadScript);
                    ui.close();
                }
                ui.separator();
                if ui.button("Import STL…").clicked() {
                    picked = Some(MenuCommand::ImportStl);
                    ui.close();
                }
                if ui.button("Import STEP…").clicked() {
                    picked = Some(MenuCommand::ImportStep);
                    ui.close();
                }
                if ui.button("Import Image…").clicked() {
                    picked = Some(MenuCommand::ImportImage);
                    ui.close();
                }
                ui.separator();
                if ui.button("Export STL…").clicked() {
                    picked = Some(MenuCommand::ExportStl);
                    ui.close();
                }
                if ui.button("Export STEP…").clicked() {
                    picked = Some(MenuCommand::ExportStep);
                    ui.close();
                }
                ui.separator();
                if ui.button("Document JSON…").clicked() {
                    picked = Some(MenuCommand::DocumentJson);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Undo").clicked() {
                    picked = Some(MenuCommand::UndoLast);
                    ui.close();
                }
                if ui.button("Clear document").clicked() {
                    picked = Some(MenuCommand::Clear);
                    ui.close();
                }
            });
            ui.menu_button("CAD", |ui| {
                if ui.button("New Drawing").clicked() {
                    picked = Some(MenuCommand::NewDrawing);
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom to fit").clicked() {
                    picked = Some(MenuCommand::ZoomToFit);
                    ui.close();
                }
                if ui.button("Command palette").clicked() {
                    picked = Some(MenuCommand::ToggleCommandPalette);
                    ui.close();
                }
                if ui.button("First-person mode").clicked() {
                    picked = Some(MenuCommand::ToggleFpsMode);
                    ui.close();
                }
                ui.separator();
                for &pane in Pane::ALL {
                    let label = pane.label();
                    let mut visible = pane_visible(pane);
                    if ui.checkbox(&mut visible, label).changed() {
                        picked = Some(MenuCommand::SetPaneVisible { pane, visible });
                        ui.close();
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Documentation").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab("https://www.iffycan.com/BearCAD/docs/intro"));
                    ui.close();
                }
                if ui.button("About").clicked() {
                    picked = Some(MenuCommand::About);
                    ui.close();
                }
                if ui.button("Licenses").clicked() {
                    picked = Some(MenuCommand::Licenses);
                    ui.close();
                }
                if ui.button("Export Session Commands…").clicked() {
                    picked = Some(MenuCommand::ExportSessionCommands);
                    ui.close();
                }
            });
        });
    });
    picked
}
