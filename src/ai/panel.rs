//! The AI pane's contents (#1594).
//!
//! Three sections, in a fixed order, each collapsible. The pane is hidden by default and is
//! opened from View ▸ Panes or the command palette ("AI pane").

use eframe::egui;

use crate::actions::{Action, AppState};
use crate::ai::backends::{Backend, KeySource, Provider};

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
                if !entry.text.is_empty() {
                    ui.label(egui::RichText::new(&entry.text).size(12.0));
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
                ui.add_space(6.0);
            }
        });
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
