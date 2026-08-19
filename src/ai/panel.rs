//! The AI pane's contents (#1594).
//!
//! Layout only: the three sections, in a fixed order, each collapsible. The sections fill
//! in as the rest of #1593 lands. The pane is hidden by default and is opened from
//! View ▸ Panes or the command palette ("AI pane").

use eframe::egui;

use crate::actions::AppState;

/// The pane's title, shown in its heading and (on phones) its window bar.
pub const PANE_TITLE: &str = "AI";

/// The panel id the pane docks under — also the key a scripted
/// `bearcad.ui.screenshot("ai")` crops to.
pub const SHELL_ID: &str = "ai";

/// One line under the heading. Says what the pane is for, and that it does nothing on its
/// own — the pane is the only place the opt-in nature of these features is visible.
const SUBTITLE: &str = "Chat, agent skills and MCP — all opt-in. Nothing leaves this \
                        machine until you set up a backend.";

/// Draw the pane body. `state` is the live app state; the pane reads the open documents
/// and writes back through actions like any other pane.
pub fn contents(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading(PANE_TITLE);
    ui.label(egui::RichText::new(SUBTITLE).size(12.0).weak());
    ui.add_space(8.0);

    section(ui, "Chat", true, |ui| chat_section(ui, state));
    section(ui, "Agents & Skill", false, |ui| skill_section(ui, state));
    section(ui, "MCP Server", false, |ui| mcp_section(ui, state));
}

/// A collapsible pane section. `default_open` decides the state before the user touches it;
/// egui remembers each section's state per id afterwards.
fn section(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(("ai_section", title))
        .default_open(default_open)
        .show(ui, |ui| {
            ui.add_space(2.0);
            add_contents(ui);
            ui.add_space(4.0);
        });
}

fn chat_section(ui: &mut egui::Ui, _state: &mut AppState) {
    ui.label(
        egui::RichText::new("Add a backend to start a conversation about your documents.")
            .size(12.0)
            .weak(),
    );
}

fn skill_section(ui: &mut egui::Ui, _state: &mut AppState) {
    ui.label(
        egui::RichText::new("Install the BearCAD skill into the AI tools on this machine.")
            .size(12.0)
            .weak(),
    );
}

fn mcp_section(ui: &mut egui::Ui, _state: &mut AppState) {
    ui.label(
        egui::RichText::new("Off. Turn it on to let an agent drive the open document.")
            .size(12.0)
            .weak(),
    );
}
