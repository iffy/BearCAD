//! The AI pane's contents (#1594).
//!
//! Three sections, in a fixed order, each collapsible. The pane is hidden by default and is
//! opened from View ▸ Panes or the command palette ("AI pane").

use eframe::egui;

use crate::actions::{Action, AppState};
use crate::ai::backends::{Backend, KeySource, Provider};
use crate::ai::pricing;

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

    // Help ▸ Install AI Agent Skill… opens the pane at that section, so it wins over the
    // usual default of Chat for one frame.
    let open_skill = std::mem::take(&mut state.ai.borrow_mut().open_skill_section);

    section(ui, "Chat", true, |ui| chat_section(ui, state));
    section_open(ui, "Agents & Skill", false, open_skill.then_some(true), |ui| {
        skill_section(ui, state)
    });
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
    section_open(ui, title, default_open, None, add_contents);
}

/// As [`section`], but `force` overrides the remembered state for this frame — how a menu
/// item can open the pane *at* a particular section.
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

#[cfg(target_arch = "wasm32")]
fn chat_section(ui: &mut egui::Ui, state: &mut AppState) {
    backend_picker(ui, state);
    ui.label(
        egui::RichText::new("Chat runs in the desktop app.")
            .size(12.0)
            .weak(),
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn chat_section(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::ai::context::ContextScope;

    backend_picker(ui, state);

    let mut action: Option<Action> = None;
    let (has_backend, streaming, scope) = {
        let ai = state.ai.borrow();
        (
            ai.config.selected().is_some(),
            ai.chat.is_streaming() || ai.chat.awaiting_context,
            ai.chat.scope,
        )
    };

    // What the next message will carry. The default is the one document in front of you.
    ui.horizontal(|ui| {
        ui.label("Sees");
        for option in [ContextScope::Document, ContextScope::AllOpen] {
            if ui
                .selectable_label(scope == option, option.label())
                .clicked()
                && scope != option
            {
                action = Some(Action::SetAiContextScope { scope: option });
            }
        }
    });

    ui.add_space(4.0);
    thread(ui, state);
    if let Some(run) = suggested_blocks(ui, state) {
        action = Some(run);
    }
    conversation_total(ui, state);
    ui.add_space(4.0);

    // What was actually sent last time, exactly as sent (#1597).
    let last = {
        let ai = state.ai.borrow();
        ai.chat
            .last_context
            .as_ref()
            .map(|c| (c.documents, c.estimated_tokens, c.truncated, c.text.clone()))
    };
    if let Some((documents, tokens, truncated, text)) = last {
        let summary = format!(
            "Sent {documents} document(s), ~{tokens} tokens{}",
            if truncated { ", truncated" } else { "" }
        );
        egui::CollapsingHeader::new(egui::RichText::new(summary).size(11.0).weak())
            .id_salt("ai_context_disclosure")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .id_salt("ai_context_scroll")
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
    }

    // The first message to a backend asks first (#1609), naming exactly what would go and
    // where it would go. Until this is answered, nothing has left the machine.
    let pending_consent = state.ai.borrow().chat.pending_consent.clone();
    if let Some(text) = pending_consent {
        let (name, host, local) = {
            let ai = state.ai.borrow();
            match ai.config.selected() {
                Some(b) => (b.name.clone(), b.host(), b.is_local()),
                None => (String::new(), String::new(), false),
            }
        };
        let context = crate::ai::context::estimate_tokens(&text);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(
                egui::RichText::new(if local {
                    format!("First message to {name}. It runs on this machine ({host}), so \
                             nothing leaves it.")
                } else {
                    format!("First message to {name}. This sends your question and the \
                             document context to {host}.")
                })
                .size(11.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Scope: {}. Expand “Sends …” after the first message to see the exact text.",
                    scope.label()
                ))
                .size(10.0)
                .weak(),
            );
            let _ = context;
            ui.horizontal(|ui| {
                if ui.button("Send it").clicked() {
                    action = Some(Action::ResolveAiConsent { agreed: true });
                }
                if ui.button("Not now").clicked() {
                    action = Some(Action::ResolveAiConsent { agreed: false });
                }
            });
        });
        ui.add_space(4.0);
    }

    // The input box. Enter sends, Shift+Enter starts a new line.
    let mut send_clicked = false;
    {
        let mut ai = state.ai.borrow_mut();
        let input = &mut ai.chat.input;
        let response = ui.add_enabled(
            has_backend && !streaming,
            egui::TextEdit::multiline(input)
                .id_salt("ai_chat_input")
                .hint_text("Ask about this document…")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
        {
            send_clicked = true;
        }
    }

    ui.horizontal(|ui| {
        let text = state.ai.borrow().chat.input.trim().to_string();
        if streaming {
            if ui.button("Stop").clicked() {
                action = Some(Action::CancelAiMessage);
            }
            ui.spinner();
        } else if ui
            .add_enabled(has_backend && !text.is_empty(), egui::Button::new("Send"))
            .clicked()
            || (send_clicked && has_backend && !text.is_empty())
        {
            action = Some(Action::SendAiMessage { text });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let any = !state.ai.borrow().chat.entries.is_empty();
            if ui
                .add_enabled(any, egui::Button::new("Clear"))
                .on_hover_text("Forget this conversation — it is never saved anyway")
                .clicked()
            {
                action = Some(Action::ClearAiConversation);
            }
        });
    });

    if !has_backend {
        ui.label(
            egui::RichText::new("Add a backend above to start a conversation.")
                .size(11.0)
                .weak(),
        );
    }

    if let Some(action) = action {
        state.apply(action);
    }
}

/// The message thread. Assistant text renders as-is; a reply still arriving shows what has
/// landed so far.
#[cfg(not(target_arch = "wasm32"))]
fn thread(ui: &mut egui::Ui, state: &mut AppState) {
    let ai = state.ai.borrow();
    if ai.chat.entries.is_empty() {
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(260.0)
        .stick_to_bottom(true)
        .id_salt("ai_chat_thread")
        .show(ui, |ui| {
            for entry in &ai.chat.entries {
                let (who, colour) = match entry.role {
                    crate::ai::providers::Role::User => ("You", ui.visuals().strong_text_color()),
                    crate::ai::providers::Role::Assistant => {
                        ("BearCAD AI", ui.visuals().weak_text_color())
                    }
                };
                ui.label(egui::RichText::new(who).size(10.0).color(colour));
                // Fenced blocks draw as code rather than showing ``` markers to the reader;
                // the latest reply's blocks also get Run buttons below the thread.
                for segment in crate::ai::chat::segments(&entry.text) {
                    match segment {
                        crate::ai::chat::Segment::Prose(text) => {
                            ui.label(egui::RichText::new(text).size(12.0));
                        }
                        crate::ai::chat::Segment::Code(code) => {
                            ui.label(
                                egui::RichText::new(code)
                                    .size(11.0)
                                    .monospace()
                                    .background_color(ui.visuals().extreme_bg_color),
                            );
                        }
                    }
                }
                if entry.streaming && entry.text.is_empty() {
                    ui.label(egui::RichText::new("…").size(12.0).weak());
                }
                if let Some(error) = &entry.error {
                    ui.label(
                        egui::RichText::new(error)
                            .size(11.0)
                            .color(ui.visuals().error_fg_color),
                    );
                }
                // What this exchange cost (#1599). Tokens always; money only when the rate
                // for that backend's model is known.
                if entry.role == crate::ai::providers::Role::Assistant && !entry.streaming {
                    let price = ai.config.get(&entry.backend).and_then(pricing::price_for);
                    let line = pricing::usage_line(entry.usage, price);
                    if !line.is_empty() {
                        ui.label(egui::RichText::new(line).size(10.0).weak());
                    }
                }
                ui.add_space(6.0);
            }
        });
}

/// The Lua the last reply suggested (#1600), each block with a **Run** and a **Copy**.
///
/// Nothing here runs on its own. One click runs one block, through the ordinary script
/// path, so Undo takes it back like any other edit.
#[cfg(not(target_arch = "wasm32"))]
fn suggested_blocks(ui: &mut egui::Ui, state: &mut AppState) -> Option<Action> {
    let blocks = state.ai.borrow().chat.blocks();
    if blocks.is_empty() {
        return None;
    }
    let mut run = None;
    ui.add_space(4.0);
    for (index, block) in blocks.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Suggested Lua {}", index + 1))
                    .size(10.0)
                    .weak(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Run")
                    .on_hover_text("Run this block on the active document — undoable")
                    .clicked()
                {
                    run = Some(Action::RunAiBlock { index });
                }
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(block.source.clone());
                }
            });
        });
        ui.add(
            egui::TextEdit::multiline(&mut block.source.as_str())
                .id_salt(("ai_block", block.entry, block.index))
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        match &block.outcome {
            Some(Ok(status)) => {
                ui.label(egui::RichText::new(format!("✓ {status}")).size(10.0).weak());
            }
            Some(Err(error)) => {
                ui.label(
                    egui::RichText::new(error)
                        .size(10.0)
                        .color(ui.visuals().error_fg_color),
                );
            }
            None => {}
        }
        ui.add_space(4.0);
    }
    run
}

/// What this conversation has cost so far (#1599). Silent until a reply reports usage.
#[cfg(not(target_arch = "wasm32"))]
fn conversation_total(ui: &mut egui::Ui, state: &mut AppState) {
    let ai = state.ai.borrow();
    let usage = ai.chat.total_usage();
    if usage.input_tokens == 0 && usage.output_tokens == 0 {
        return;
    }
    // Price by the backend that is answering now: mixing backends mid-conversation is rare,
    // and each message's own line is exact.
    let price = ai.config.selected().and_then(pricing::price_for);
    let line = pricing::usage_line(usage, price);
    ui.label(
        egui::RichText::new(format!("This conversation: {line}"))
            .size(11.0)
            .weak(),
    );
}

/// The backend row: which service the conversation talks to, and the editor for adding and
/// removing them (#1595).
fn backend_picker(ui: &mut egui::Ui, state: &mut AppState) {
    let (selected, backends, none_yet) = {
        let ai = state.ai.borrow();
        (
            ai.config.selected().cloned(),
            ai.config.backends.clone(),
            ai.config.backends.is_empty(),
        )
    };
    let mut action: Option<Action> = None;

    ui.horizontal(|ui| {
        ui.label("Backend");
        let label = selected
            .as_ref()
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "None".to_string());
        egui::ComboBox::from_id_salt("ai_backend_picker")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for backend in &backends {
                    let picked = selected.as_ref().is_some_and(|s| s.id == backend.id);
                    if ui.selectable_label(picked, &backend.name).clicked() && !picked {
                        action = Some(Action::SelectAiBackend {
                            id: backend.id.clone(),
                        });
                    }
                }
                if none_yet {
                    ui.label(egui::RichText::new("No backends yet").weak());
                }
            });
    });

    // A backend whose key has gone missing is the single most likely reason a message
    // fails, so say it here rather than at send time.
    if let Some(reason) = selected.as_ref().and_then(|b| b.unusable_reason()) {
        ui.label(
            egui::RichText::new(format!("⚠ {reason}"))
                .size(11.0)
                .color(ui.visuals().warn_fg_color),
        );
    }
    if let Some(backend) = &selected {
        ui.label(
            egui::RichText::new(format!("{} · key {}", backend.model, backend.key_description()))
                .size(11.0)
                .weak(),
        );
    }

    egui::CollapsingHeader::new("Manage backends")
        .id_salt("ai_manage_backends")
        .default_open(none_yet)
        .show(ui, |ui| {
            for backend in &backends {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} · {}", backend.name, backend.model))
                            .size(12.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("Remove")
                            .on_hover_text("Forget this backend and any key stored for it")
                            .clicked()
                        {
                            action = Some(Action::RemoveAiBackend {
                                id: backend.id.clone(),
                            });
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {} · key {}",
                        backend.provider.label(),
                        backend.effective_base_url(),
                        backend.key_description()
                    ))
                    .size(10.0)
                    .weak(),
                );
                // What this backend has cost since it was added, or since the last reset
                // (#1599). Persisted, so it survives restarts.
                if !backend.spend.is_empty() {
                    ui.horizontal(|ui| {
                        let price = pricing::price_for(backend);
                        let total = if price.is_some() {
                            pricing::format_cost(backend.spend.cost)
                        } else {
                            "price unknown".to_string()
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "All time: {} · {} tokens · {} replies",
                                total,
                                pricing::format_tokens(backend.spend.tokens()),
                                backend.spend.exchanges
                            ))
                            .size(10.0)
                            .weak(),
                        );
                        if ui
                            .small_button("Reset")
                            .on_hover_text("Start this backend's running total from zero")
                            .clicked()
                        {
                            action = Some(Action::ResetAiBackendSpend {
                                id: backend.id.clone(),
                            });
                        }
                    });
                }
                ui.add_space(4.0);
            }
            ui.separator();
            if let Some(added) = add_backend_form(ui) {
                action = Some(Action::AddAiBackend { backend: added });
            }
        });

    if let Some(action) = action {
        state.apply(action);
    }
}

/// The "add a backend" form. Keeps its draft in egui memory — it is UI-only state, and a
/// half-typed key has no business living in `AppState` (or reaching a script).
fn add_backend_form(ui: &mut egui::Ui) -> Option<Backend> {
    let id = ui.make_persistent_id("ai_add_backend_draft");
    let mut draft: Draft = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
    let mut added = None;

    ui.horizontal(|ui| {
        ui.label("Add");
        egui::ComboBox::from_id_salt("ai_add_provider")
            .selected_text(draft.provider.label())
            .show_ui(ui, |ui| {
                for &provider in Provider::ALL {
                    if ui
                        .selectable_label(draft.provider == provider, provider.label())
                        .clicked()
                        && draft.provider != provider
                    {
                        // Switching provider re-bases every default the user has not typed
                        // over, so the model and URL always match the service.
                        draft = Draft::for_provider(provider);
                    }
                }
            });
    });

    let preset = Backend::preset(draft.provider);
    labelled(ui, "Name", &mut draft.name, &preset.name);
    labelled(ui, "Model", &mut draft.model, &preset.model);
    labelled(ui, "URL", &mut draft.base_url, preset.effective_base_url());

    ui.horizontal(|ui| {
        ui.label("Key");
        ui.selectable_value(&mut draft.key_mode, KeyMode::Env, "Env var")
            .on_hover_text("Read at send time from an environment variable — nothing is stored");
        ui.selectable_value(&mut draft.key_mode, KeyMode::Stored, "Paste")
            .on_hover_text("Stored in ai.json, readable only by you");
        ui.selectable_value(&mut draft.key_mode, KeyMode::None, "None")
            .on_hover_text("A local server that needs no key");
    });
    match draft.key_mode {
        KeyMode::Env => {
            let hint = preset.provider.default_env_var().unwrap_or("API_KEY");
            labelled(ui, "Var", &mut draft.key_env, hint);
        }
        KeyMode::Stored => {
            ui.add(
                egui::TextEdit::singleline(&mut draft.key)
                    .password(true)
                    .hint_text("sk-…")
                    .desired_width(f32::INFINITY),
            );
        }
        KeyMode::None => {}
    }

    if ui.button("Add backend").clicked() {
        let name = non_empty(&draft.name, &preset.name);
        added = Some(Backend {
            id: String::new(),
            name,
            provider: draft.provider,
            model: non_empty(&draft.model, &preset.model),
            base_url: non_empty(&draft.base_url, preset.effective_base_url()),
            key: match draft.key_mode {
                KeyMode::None => KeySource::None,
                KeyMode::Env => KeySource::Env(non_empty(
                    &draft.key_env,
                    preset.provider.default_env_var().unwrap_or("API_KEY"),
                )),
                KeyMode::Stored => KeySource::Stored(draft.key.clone()),
            },
            // Rates come from the shipped table until the user overrides them (#1599).
            price: None,
            spend: crate::ai::pricing::Spend::default(),
            // The first message to it asks before sending anything (#1609).
            consented: false,
        });
        draft = Draft::for_provider(draft.provider);
    }

    ui.data_mut(|d| d.insert_temp(id, draft));
    added
}

/// A labelled single-line field that shows the preset value as its placeholder, so leaving
/// it blank is the same as accepting the default.
fn labelled(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
    });
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.trim().to_string()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KeyMode {
    /// Read from an environment variable at send time — the default, because it stores
    /// nothing.
    #[default]
    Env,
    Stored,
    None,
}

/// The add-backend form's in-progress values. Lives in egui memory for the pane's lifetime.
#[derive(Clone, Debug, Default)]
struct Draft {
    provider: Provider,
    name: String,
    model: String,
    base_url: String,
    key_mode: KeyMode,
    key_env: String,
    key: String,
}

impl Draft {
    fn for_provider(provider: Provider) -> Self {
        Self {
            provider,
            // A local server usually wants no key at all; everything else defaults to the
            // environment variable it conventionally uses.
            key_mode: match provider.default_env_var() {
                Some(_) => KeyMode::Env,
                None => KeyMode::None,
            },
            ..Default::default()
        }
    }
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
                            let colour = if entry.ok {
                                ui.visuals().weak_text_color()
                            } else {
                                ui.visuals().error_fg_color
                            };
                            ui.label(
                                egui::RichText::new(format!("{}  {}", entry.what, entry.detail))
                                    .size(10.0)
                                    .color(colour),
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
