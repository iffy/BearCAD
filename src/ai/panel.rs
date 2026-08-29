//! The AI pane's contents (#1594, #1620/#1621, #1633).
//!
//! One pane, **AI** ([`SHELL_ID`]), for the one thing BearCAD does about AI: let an agent
//! drive it. Its two sections are the **MCP Server** — the local endpoint an agent connects
//! to — and the **Agent Skill**, the page an agent reads to learn the API. It opens from
//! View ▸ Panes, Integration ▸ AI, or a command palette (`"AI pane"`), docks right, and is
//! hidden by default.
//!
//! Nothing here listens until the user switches the server on.

use eframe::egui;

use crate::actions::{Action, AppState};

/// The pane's title, shown in its heading and (on phones) its window bar.
pub const PANE_TITLE: &str = "AI";

/// The pane's panel id — also the key a scripted `bearcad.ui.screenshot("ai")` crops to.
pub const SHELL_ID: &str = "ai";

/// The pane's sections, in the order they are drawn. Also the ids their collapsing headers
/// remember their open/closed state under, and what `bearcad.ui.ai_sections(...)` drives
/// (#1619).
const SECTIONS: [&str; 2] = ["MCP Server", "Agent Skill"];

/// The pane's section titles, in draw order — what `bearcad.ui.ai_pane_sections()` reports
/// so a script can prove the split.
pub fn section_titles() -> &'static [&'static str; 2] {
    &SECTIONS
}

/// Draw the pane body. `state` is the live app state; the pane writes back through actions
/// like any other pane.
pub fn contents(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading(PANE_TITLE);
    ui.add_space(8.0);

    // On phones the pane is a window that scrolls itself; nesting a second scroll area
    // inside it would only fight it.
    if crate::touch::compact(ui.ctx()) {
        sections(ui, state);
        return;
    }

    // #1619: the sections together can outgrow the pane once they are open, so the pane
    // scrolls rather than clipping whatever does not fit.
    let out = egui::ScrollArea::vertical()
        .id_salt("ai_pane_scroll")
        .auto_shrink([false, true])
        .show(ui, |ui| sections(ui, state));
    crate::script::remember_pane_scroll(
        ui.ctx(),
        SHELL_ID,
        Some(crate::script::PaneScroll {
            offset: out.state.offset.y,
            content: out.content_size.y,
            viewport: out.inner_rect.height(),
        }),
    );
}

/// The pane's two sections, in a fixed order.
fn sections(ui: &mut egui::Ui, state: &mut AppState) {
    // Integration ▸ AI ▸ MCP Server… and ▸ Install AI Agent Skill… each open the pane at
    // their own section, for one frame; the headers remember it from there.
    // `bearcad.ui.ai_sections(...)` opens or collapses both at once (#1619).
    let (open_skill, open_mcp, open_all) = {
        let mut ai = state.ai.borrow_mut();
        (
            std::mem::take(&mut ai.open_skill_section),
            std::mem::take(&mut ai.open_mcp_section),
            std::mem::take(&mut ai.sections_open),
        )
    };

    section_open(
        ui,
        SECTIONS[0],
        true,
        open_all.or(open_mcp.then_some(true)),
        |ui| mcp_section(ui, state),
    );
    section_open(
        ui,
        SECTIONS[1],
        false,
        open_all.or(open_skill.then_some(true)),
        |ui| skill_section(ui, state),
    );
}

/// A collapsible pane section. `default_open` decides the state before the user touches it;
/// egui remembers each section's state per id afterwards. `force` overrides the remembered
/// state for this frame — how a menu item opens the pane *at* a particular section, and how
/// `bearcad.ui.ai_sections(...)` opens or collapses all of them.
fn section_open(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    force: Option<bool>,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(("ai_section", title))
        .default_open(default_open)
        .open(force)
        .show(ui, |ui| {
            ui.add_space(2.0);
            add_contents(ui);
            ui.add_space(4.0);
        });
}

/// **Agents & Skill** (#1604): install the BearCAD agent skill into the AI tools on this
/// machine, so an outside agent knows how to drive the app.
fn skill_section(ui: &mut egui::Ui, state: &mut AppState) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::ai::skill;

        ui.label(
            egui::RichText::new(
                "One page that teaches an AI agent to drive BearCAD. Install it where your                  tools look for instructions.",
            )
            .size(11.0)
            .weak(),
        );
        ui.add_space(4.0);

        let home = home_dir();
        // Project targets write into a project; default to the open document's folder,
        // which is the project the user is most likely thinking of.
        let mut project = ui.data_mut(|d| {
            d.get_temp::<std::path::PathBuf>(egui::Id::new("ai_skill_project"))
        });
        if project.is_none() {
            project = state
                .path
                .as_deref()
                .map(std::path::Path::new)
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf());
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Project").size(11.0));
            let label = project
                .as_ref()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "none".to_string());
            ui.label(egui::RichText::new(label).size(11.0).weak());
            if ui
                .small_button("Choose…")
                .on_hover_text("Where project-scoped installs (AGENTS.md, Copilot, Cursor) go")
                .clicked()
            {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    ui.data_mut(|d| {
                        d.insert_temp(egui::Id::new("ai_skill_project"), dir.clone())
                    });
                    project = Some(dir);
                }
            }
        });

        let mut action: Option<Action> = None;
        ui.add_space(4.0);
        for target in skill::TARGETS {
            let dir = matches!(target.scope, skill::Scope::Project)
                .then(|| project.clone())
                .flatten();
            let usable = !matches!(target.scope, skill::Scope::Project) || dir.is_some();
            let installed = target.installed(home.as_deref(), dir.as_deref());
            let detected = target.detected(home.as_deref(), dir.as_deref());

            ui.horizontal(|ui| {
                let mark = if installed {
                    "✓"
                } else if detected {
                    "·"
                } else {
                    " "
                };
                ui.label(egui::RichText::new(format!("{mark} {}", target.label)).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if installed {
                        if ui.small_button("Remove").clicked() {
                            action = Some(Action::UninstallAiSkill {
                                target: target.id.to_string(),
                                dir: dir.clone(),
                            });
                        }
                    } else if ui
                        .add_enabled(usable, egui::Button::new("Install").small())
                        .on_hover_text(if usable {
                            target.note
                        } else {
                            "Choose a project directory first"
                        })
                        .clicked()
                    {
                        action = Some(Action::InstallAiSkill {
                            target: target.id.to_string(),
                            dir: dir.clone(),
                        });
                    }
                });
            });
            ui.label(egui::RichText::new(target.note).size(10.0).weak());
            ui.add_space(2.0);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .small_button("Copy skill")
                .on_hover_text("For a tool that is not listed")
                .clicked()
            {
                ui.ctx().copy_text(skill::SKILL.to_string());
            }
            if ui.small_button("Save as…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("SKILL.md")
                    .save_file()
                {
                    match std::fs::write(&path, skill::SKILL) {
                        Ok(()) => state.status = format!("Wrote {}", path.display()),
                        Err(e) => state.status = format!("Could not write {}: {e}", path.display()),
                    }
                }
            }
            if ui
                .small_button("Copy URL")
                .on_hover_text(skill::SKILL_URL)
                .clicked()
            {
                ui.ctx().copy_text(skill::SKILL_URL.to_string());
            }
        });
        ui.label(
            egui::RichText::new("Same thing from a terminal: bearcad skill install")
                .size(10.0)
                .weak(),
        );

        if let Some(action) = action {
            state.apply(action);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = state;
        ui.label(
            egui::RichText::new("Installing the skill needs the desktop app.")
                .size(12.0)
                .weak(),
        );
    }
}

/// The user's home directory, where user-scoped installs land.
#[cfg(not(target_arch = "wasm32"))]
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}
/// **MCP Server** (#1606): switch the local server on, and copy what a client needs.
#[cfg(target_arch = "wasm32")]
fn mcp_section(ui: &mut egui::Ui, _state: &mut AppState) {
    ui.label(
        egui::RichText::new("The MCP server needs the desktop app.")
            .size(12.0)
            .weak(),
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn mcp_section(ui: &mut egui::Ui, state: &mut AppState) {
    let (running, port, url, log) = {
        let ai = state.ai.borrow();
        match &ai.mcp {
            Some(server) => (true, server.port(), server.url(), server.log()),
            None => (false, ai.config.mcp_port, String::new(), Vec::new()),
        }
    };
    let mut action: Option<Action> = None;

    ui.label(
        egui::RichText::new(
            "Lets an AI agent read and edit the document you have open. Loopback only, and \
             it needs the token below.",
        )
        .size(11.0)
        .weak(),
    );
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        if running {
            if ui.button("Stop").clicked() {
                action = Some(Action::StopMcpServer);
            }
            ui.label(
                egui::RichText::new(format!("● listening on {port}"))
                    .size(11.0)
                    .color(egui::Color32::from_rgb(46, 160, 67)),
            );
        } else {
            if ui.button("Start").clicked() {
                action = Some(Action::StartMcpServer { port: None });
            }
            ui.label(egui::RichText::new("off").size(11.0).weak());
            // Only editable while stopped: the running port is whatever the socket got.
            let mut port_text = port.to_string();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut port_text)
                        .id_salt("ai_mcp_port")
                        .desired_width(56.0),
                )
                .changed()
            {
                if let Ok(port) = port_text.trim().parse::<u16>() {
                    state.ai.borrow_mut().config.mcp_port = port;
                    state.ai.borrow_mut().config_dirty = true;
                }
            }
        }
    });

    if running {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&url).size(11.0).monospace());
            if ui.small_button("Copy URL").clicked() {
                ui.ctx().copy_text(url.clone());
            }
        });
        ui.horizontal(|ui| {
            // The token itself is never drawn: a pane screenshot would carry it. It goes to
            // the clipboard, or into a client configuration below.
            ui.label(egui::RichText::new("Token hidden").size(11.0).weak());
            if ui
                .small_button("Copy token")
                .on_hover_text("Never shown on screen — a screenshot of this pane would carry it")
                .clicked()
            {
                let token = state
                    .ai
                    .borrow()
                    .mcp
                    .as_ref()
                    .map(|s| s.token())
                    .unwrap_or_default();
                ui.ctx().copy_text(token);
            }
            if ui
                .small_button("New token")
                .on_hover_text("Any client configured with the old token stops working")
                .clicked()
            {
                action = Some(Action::RegenerateMcpToken);
            }
        });

        // Ready-made configurations, filled in with this server's URL and token.
        egui::CollapsingHeader::new("Connect a client")
            .id_salt("ai_mcp_clients")
            .show(ui, |ui| {
                let token = state
                    .ai
                    .borrow()
                    .mcp
                    .as_ref()
                    .map(|s| s.token())
                    .unwrap_or_default();
                for config in crate::ai::mcp::client_configs(&url, &token) {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(config.label).size(11.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(config.text.clone());
                            }
                        });
                    });
                    ui.label(egui::RichText::new(config.note).size(10.0).weak());
                    ui.add_space(4.0);
                }
            });

        // What an agent has been doing: the only visible sign something external is
        // changing the document.
        egui::CollapsingHeader::new(format!("Activity ({})", log.len()))
            .id_salt("ai_mcp_log")
            .show(ui, |ui| {
                if log.is_empty() {
                    ui.label(egui::RichText::new("Nothing yet.").size(11.0).weak());
                }
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .id_salt("ai_mcp_log_scroll")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for entry in &log {
                            let color = if entry.ok {
                                ui.visuals().weak_text_color()
                            } else {
                                ui.visuals().error_fg_color
                            };
                            ui.label(
                                egui::RichText::new(format!("{}  {}", entry.what, entry.detail))
                                    .size(10.0)
                                    .color(color),
                            );
                        }
                    });
            });
    } else {
        ui.label(
            egui::RichText::new("Off — nothing is listening.")
                .size(11.0)
                .weak(),
        );
    }

    if let Some(action) = action {
        state.apply(action);
    }
}
#[cfg(all(test, not(target_arch = "wasm32")))]
mod scroll_tests {
    //! #1619: the pane scrolls rather than clipping whatever does not fit.

    use super::*;
    use crate::actions::{AppState, Pane};

    /// Draw one frame of the pane into a fixed-size window, with `events` delivered to it.
    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
        // Wide enough to stay out of the compact (phone) layout, short enough that the
        // pane's own content cannot fit.
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 220.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            egui::Panel::right("ai")
                .resizable(false)
                .exact_size(340.0)
                .show(ui, |ui| contents(ui, state));
        });
    }

    /// Where the pane thinks it is scrolled to, as scripts read it.
    fn scroll(ctx: &egui::Context) -> crate::script::PaneScroll {
        crate::script::pane_scroll(ctx, Pane::Ai).expect("the drawn pane reports its scroll")
    }

    #[test]
    fn a_wheel_scrolls_a_pane_taller_than_its_window() {
        let ctx = egui::Context::default();
        let mut state = AppState::default();
        // Every section open — the state the pane is in when it outgrows the window.
        state.ai.borrow_mut().sections_open = Some(true);
        let centre = egui::pos2(900.0 - 170.0, 110.0);
        for _ in 0..3 {
            frame(&ctx, &mut state, vec![egui::Event::PointerMoved(centre)]);
        }

        let start = scroll(&ctx);
        assert!(
            start.content > start.viewport,
            "the fixture has to overflow to test scrolling: {} in {}",
            start.content,
            start.viewport
        );
        assert_eq!(start.offset, 0.0, "a pane starts at the top");

        // A wheel over the pane moves it down, and stops at the bottom rather than past it.
        let wheel = |dy: f32| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -dy),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![wheel(10_000.0)]);
        for _ in 0..12 {
            frame(&ctx, &mut state, vec![]);
        }
        let bottom = scroll(&ctx);
        let max = start.content - start.viewport;
        assert!(bottom.offset > 0.0, "the wheel should scroll the pane down");
        assert!(
            (bottom.offset - max).abs() < 1.0,
            "the wheel should reach the bottom ({max}), got {}",
            bottom.offset
        );

        // And back up: the top is as far as it goes.
        frame(&ctx, &mut state, vec![wheel(-10_000.0)]);
        for _ in 0..12 {
            frame(&ctx, &mut state, vec![]);
        }
        assert_eq!(scroll(&ctx).offset, 0.0, "scrolling up stops at the top");
    }
}
