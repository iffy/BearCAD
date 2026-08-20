//! The AI panes' contents (#1594, #1620/#1621).
//!
//! Two panes, split apart in #1620:
//!
//! - **AI** ([`SHELL_ID`]) holds the configuration. Its two sections (#1621) are
//!   *Use AI inside BearCAD* (the chat backend configuration) and *Have AI use
//!   BearCAD* (the agent skill and the local MCP server). It opens from View ▸ Panes or
//!   a command palette (`"AI pane"`), docks right, and is hidden by default.
//! - **AI Chat** ([`CHAT_SHELL_ID`]) holds the conversation. It opens at the bottom of
//!   the app and spans it, the way the command palette does.
//!
//! Both are silent until the user configures a backend and presses send.

use eframe::egui;

use crate::actions::{Action, AppState};
use crate::ai::backends::{Backend, KeySource, Provider};
use crate::ai::pricing;

/// The configuration pane's title, shown in its heading and (on phones) its window bar.
pub const PANE_TITLE: &str = "AI";

/// The config pane's panel id — also the key a scripted `bearcad.ui.screenshot("ai")`
/// crops to.
pub const SHELL_ID: &str = "ai";

/// The chat pane's title (its bottom console has no window bar to repeat it, but a
/// distinct name keeps it straight in scripts and screenshots).
pub const CHAT_PANE_TITLE: &str = "AI Chat";

/// The chat pane's panel id — a bottom console spanning the app, like the command
/// palette. Also the key `bearcad.ui.screenshot("ai_chat")` crops to.
pub const CHAT_SHELL_ID: &str = "ai_chat";

/// The configuration pane's sections, in the order they are drawn. Also the ids their
/// collapsing headers remember their open/closed state under, and what
/// `bearcad.ui.ai_sections(...)` drives (#1619).
const SECTIONS: [&str; 2] = ["Use AI inside BearCAD", "Have AI use BearCAD"];

/// The configuration pane's section titles (#1621), in draw order — what
/// `bearcad.ui.ai_pane_sections()` reports so a script can prove the split.
pub fn section_titles() -> &'static [&'static str; 2] {
    &SECTIONS
}

/// Draw the configuration pane body (#1621). `state` is the live app state; the pane
/// reads the open documents and writes back through actions like any other pane.
pub fn contents(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading(PANE_TITLE);
    ui.add_space(8.0);

    // Widget rects recorded for `bearcad.ui.ai_backend_widget` are scoped to a frame: clear
    // before drawing, re-record only what this frame shows, so a script never aims at a
    // control that is gone (#1627).
    crate::script::clear_widget_rects(ui.ctx());

    // On phones the pane is a window that scrolls itself; nesting a second scroll area
    // inside it would only fight it.
    if crate::touch::compact(ui.ctx()) {
        sections(ui, state);
        return;
    }

    // #1619: the two sections together outgrow the pane once they are all open, so the
    // pane scrolls rather than clipping whatever does not fit.
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

/// The configuration pane's two sections, in a fixed order (#1621).
fn sections(ui: &mut egui::Ui, state: &mut AppState) {
    // Help ▸ Install AI Agent Skill… opens the pane at that section, so it wins over the
    // usual default (chat config) for one frame. Integration ▸ AI MCP Server… does the
    // same: the MCP section lives under the same header as the skill (#1622).
    // `bearcad.ui.ai_sections(...)` opens or collapses both at once (#1619); like the
    // skill request it applies for one frame, and the headers remember it from there.
    let (open_skill, open_mcp, open_all) = {
        let mut ai = state.ai.borrow_mut();
        (
            std::mem::take(&mut ai.open_skill_section),
            std::mem::take(&mut ai.open_mcp_section),
            std::mem::take(&mut ai.sections_open),
        )
    };

    section_open(ui, SECTIONS[0], true, open_all, |ui| backend_picker(ui, state));
    section_open(
        ui,
        SECTIONS[1],
        false,
        open_all
            .or(open_skill.then_some(true))
            .or(open_mcp.then_some(true)),
        |ui| {
            skill_section(ui, state);
            mcp_section(ui, state);
        },
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

/// The AI Chat pane's body (#1620): the conversation itself. The chat backend
/// configuration lives in the AI config pane, under "Use AI inside BearCAD" (#1621), so
/// the input here only reports when there is still no backend to talk to.
#[cfg(target_arch = "wasm32")]
pub fn chat_contents(ui: &mut egui::Ui, _state: &mut AppState) {
    ui.heading(CHAT_PANE_TITLE);
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("The AI chat needs the desktop app.")
            .size(12.0)
            .weak(),
    );
}

#[cfg(not(target_arch = "wasm32"))]
pub fn chat_contents(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::ai::context::ContextScope;

    ui.heading(CHAT_PANE_TITLE);
    ui.add_space(6.0);

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
            egui::RichText::new("Add a backend in the AI pane to start a conversation.")
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
    let mut connect_after_adding = false;

    // Which backend's Edit form is open, if any (#1627). UI-only state, in egui memory —
    // the half-typed draft itself belongs to the form, not to `AppState` (or a script).
    let edit_target = egui::Id::new("ai_edit_backend");
    let mut editing: Option<String> = ui.data_mut(|d| d.get_temp(edit_target).unwrap_or(None));

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
            egui::RichText::new(format!(
                "{} · key {}",
                if backend.model.is_empty() { "no model" } else { &backend.model },
                backend.key_description()
            ))
            .size(11.0)
            .weak(),
        );
        // Connecting (#1624) and choosing a model (#1617) are the two steps between adding a
        // backend and using it, so both sit here rather than inside the editor.
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(connect) = connect_row(ui, state, backend) {
                action = Some(connect);
            }
            if let Some(model) = model_row(ui, state, backend) {
                action = Some(model);
            }
        }
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
                        // Edit (#1627): opens this backend's inline form, which changes its
                        // name, model and key in place — keeping the all-time spend.
                        let edit_button = ui
                            .small_button("Edit")
                            .on_hover_text(
                                "Edit this backend in place — its all-time spend survives",
                            );
                        script_widget(ui, format!("edit:{}", backend.id), edit_button.rect);
                        if edit_button.clicked() {
                            editing = Some(backend.id.clone());
                        }
                        let remove_button = ui
                            .small_button("Remove")
                            .on_hover_text(
                                "Forget this backend and any key stored for it",
                            );
                        script_widget(ui, format!("remove:{}", backend.id), remove_button.rect);
                        if remove_button.clicked() {
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
                // Editing one backend at a time: the inline form under it (#1627).
                if editing == Some(backend.id.clone()) {
                    if let Some(commit) = edit_backend_form(ui, &backend) {
                        if let Some(a) = commit.action {
                            action = Some(a);
                        }
                        if commit.close {
                            editing = None;
                        }
                    }
                }
                ui.add_space(4.0);
            }
            ui.separator();
            if let Some((added, connect)) = add_backend_form(ui) {
                action = Some(Action::AddAiBackend { backend: added });
                // "Add and connect" is one button to the user; two actions here, because the
                // backend has to exist before there is anything to connect.
                connect_after_adding = connect;
            }
        });

    // Persist which backend's Edit form is open (or none) for the next frame.
    ui.data_mut(|d| match &editing {
        Some(id) => {
            let _ = d.insert_temp(edit_target, Some(id.clone()));
        }
        None => {
            let _ = d.remove_temp::<Option<String>>(edit_target);
        }
    });

    if let Some(action) = action {
        state.apply(action);
        if connect_after_adding {
            let id = state.ai.borrow().config.backends.last().map(|b| b.id.clone());
            if let Some(id) = id {
                state.apply(Action::ConnectAiBackend { id });
            }
        }
    }
}

/// The connect button for a backend whose provider offers PKCE OAuth (#1624), and the
/// "waiting for your browser" line while an attempt is running.
///
/// Nothing here for a provider without a flow: those show the key fields in the editor, the
/// way they always have.
#[cfg(not(target_arch = "wasm32"))]
fn connect_row(ui: &mut egui::Ui, state: &mut AppState, backend: &Backend) -> Option<Action> {
    use crate::ai::oauth::Connect;

    let running = {
        let ai = state.ai.borrow();
        ai.connect
            .as_ref()
            .filter(|flow| flow.backend == backend.id)
            .map(|flow| (flow.authorize_url.clone(), flow.state()))
    };
    if let Some((url, connect)) = running {
        let mut action = None;
        ui.horizontal(|ui| {
            match connect {
                Connect::Failed(message) => {
                    ui.label(
                        egui::RichText::new(message)
                            .size(11.0)
                            .color(ui.visuals().error_fg_color),
                    );
                }
                _ => {
                    ui.spinner();
                    ui.label(egui::RichText::new("Waiting for your browser…").size(11.0));
                }
            }
            if ui
                .small_button("Open again")
                .on_hover_text(&url)
                .clicked()
            {
                let _ = crate::open_in_browser(&url);
            }
            if ui.small_button("Cancel").clicked() {
                action = Some(Action::CancelAiConnect);
            }
        });
        return action;
    }

    backend.provider.oauth()?;
    let connected = matches!(&backend.key, KeySource::OAuth(key) if !key.trim().is_empty());
    let label = if connected {
        "Reconnect".to_string()
    } else {
        format!("Connect to {}", backend.provider.label())
    };
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .button(label)
            .on_hover_text(
                "Opens your browser to approve BearCAD. The key is issued to this app — you \
                 never see or paste one.",
            )
            .clicked()
        {
            action = Some(Action::ConnectAiBackend { id: backend.id.clone() });
        }
        if connected {
            ui.label(egui::RichText::new("Connected").size(11.0).weak());
        }
    });
    action
}

/// The **Model** row (#1617): a dropdown of what the backend says it has, or a field to type
/// a name into when it has not been asked yet.
#[cfg(not(target_arch = "wasm32"))]
fn model_row(ui: &mut egui::Ui, state: &mut AppState, backend: &Backend) -> Option<Action> {
    use crate::ai::models::Catalog;

    let catalog = state
        .ai
        .borrow()
        .models
        .get(&backend.id)
        .map(|c| c.lock().expect("catalog lock").clone());
    // An escape hatch for a model the backend does not list: typing beats a dropdown that
    // cannot hold what you want.
    let typing_id = egui::Id::new(("ai_model_typed", &backend.id));
    let mut typing: bool = ui.data_mut(|d| d.get_temp(typing_id).unwrap_or(false));
    let mut action = None;

    ui.horizontal(|ui| {
        ui.label("Model");
        match (&catalog, typing) {
            (Some(Catalog::Ready(models)), false) => {
                let selected = if backend.model.is_empty() {
                    "Choose a model".to_string()
                } else {
                    backend.model.clone()
                };
                egui::ComboBox::from_id_salt(("ai_model_picker", &backend.id))
                    .selected_text(selected)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for model in models {
                            let picked = model.id == backend.model;
                            if ui.selectable_label(picked, &model.label).clicked() && !picked {
                                action = Some(Action::SetAiBackendModel {
                                    id: backend.id.clone(),
                                    model: model.id.clone(),
                                });
                            }
                        }
                        ui.separator();
                        if ui.selectable_label(false, "Type a name…").clicked() {
                            typing = true;
                        }
                    });
            }
            (Some(Catalog::Loading), _) => {
                ui.spinner();
                ui.label(egui::RichText::new("asking the backend…").size(11.0).weak());
            }
            _ => {
                let text_id = egui::Id::new(("ai_model_text", &backend.id));
                let mut text: String =
                    ui.data_mut(|d| d.get_temp(text_id).unwrap_or_else(|| backend.model.clone()));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text("model id")
                        .desired_width(200.0),
                );
                if response.lost_focus() {
                    if text.trim() != backend.model {
                        action = Some(Action::SetAiBackendModel {
                            id: backend.id.clone(),
                            model: text.trim().to_string(),
                        });
                    }
                } else if !response.has_focus() {
                    // Not being edited: follow the backend, so a model chosen elsewhere shows.
                    text = backend.model.clone();
                }
                ui.data_mut(|d| d.insert_temp(text_id, text));
            }
        }
        if ui
            .small_button("⟳")
            .on_hover_text("Ask this backend which models it has")
            .clicked()
        {
            typing = false;
            action = Some(Action::RefreshAiModels { id: backend.id.clone() });
        }
    });
    if let Some(Catalog::Failed(message)) = &catalog {
        ui.label(
            egui::RichText::new(message)
                .size(10.0)
                .color(ui.visuals().error_fg_color),
        );
    }
    ui.data_mut(|d| d.insert_temp(typing_id, typing));
    action
}

/// The "add a backend" form. Keeps its draft in egui memory — it is UI-only state, and a
/// half-typed key has no business living in `AppState` (or reaching a script).
///
/// It never asks for a model (#1617): which models a backend has is a question for the
/// backend, and it cannot be asked until the backend exists and has a key. The flag returned
/// alongside says the user pressed **Add and connect**, so the caller starts the OAuth flow
/// as soon as the backend is there (#1624).
fn add_backend_form(ui: &mut egui::Ui) -> Option<(Backend, bool)> {
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
    labelled_script(ui, "Name", "add_name", &mut draft.name, &preset.name);
    labelled_script(ui, "URL", "add_url", &mut draft.base_url, preset.effective_base_url());

    let connectable = draft.provider.oauth().is_some();
    ui.horizontal(|ui| {
        ui.label("Key");
        if connectable {
            let connect = ui
                .selectable_value(&mut draft.key_mode, KeyMode::Connect, "Connect")
                .on_hover_text("Approve BearCAD in your browser — no key to find or paste");
            let _ = connect;
        }
        let env = ui
            .selectable_value(&mut draft.key_mode, KeyMode::Env, "Env var")
            .on_hover_text("Read at send time from an environment variable — nothing is stored");
        let _ = env;
        let stored = ui
            .selectable_value(&mut draft.key_mode, KeyMode::Stored, "Paste")
            .on_hover_text("Stored in ai.json, readable only by you");
        script_widget(ui, "add_key_mode_stored", stored.rect);
        let none = ui
            .selectable_value(&mut draft.key_mode, KeyMode::None, "None")
            .on_hover_text("A local server that needs no key");
        let _ = none;
    });
    match draft.key_mode {
        KeyMode::Env => {
            let hint = preset.provider.default_env_var().unwrap_or("API_KEY");
            labelled_script(ui, "Var", "add_key_var", &mut draft.key_env, hint);
        }
        KeyMode::Stored => {
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft.key)
                    .password(true)
                    .hint_text("sk-…")
                    .desired_width(f32::INFINITY),
            );
            script_widget(ui, "add_key_paste", response.rect);
        }
        KeyMode::Connect | KeyMode::None => {}
    }

    let connect = draft.key_mode == KeyMode::Connect && connectable;
    let button = if connect { "Add and connect" } else { "Add backend" };
    let add_response = ui
        .button(button)
        .on_hover_text("Which models it has is asked afterwards, once it can answer");
    script_widget(ui, "add_button", add_response.rect);
    if add_response.clicked() {
        let name = non_empty(&draft.name, &preset.name);
        added = Some((
            Backend {
                id: String::new(),
                name,
                provider: draft.provider,
                // No model yet: the dropdown fills from what the backend reports (#1617).
                model: preset.model.clone(),
                base_url: non_empty(&draft.base_url, preset.effective_base_url()),
                key: match draft.key_mode {
                    KeyMode::None => KeySource::None,
                    KeyMode::Connect if connectable => KeySource::OAuth(String::new()),
                    KeyMode::Connect | KeyMode::Env => KeySource::Env(non_empty(
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
            },
            connect,
        ));
        draft = Draft::for_provider(draft.provider);
    }

    ui.data_mut(|d| d.insert_temp(id, draft));
    added
}

/// The inline "edit this backend" form (#1627): name, URL, model and key — the fields a
/// remove-and-re-add used to cost somebody. Saving dispatches `Action::EditAiBackend`,
/// which changes the one entry in place and keeps the all-time spend; the draft is
/// pre-filled from the backend and lives in egui memory, so an existing key is never drawn.
#[cfg(not(target_arch = "wasm32"))]
fn edit_backend_form(ui: &mut egui::Ui, backend: &Backend) -> Option<EditCommit> {
    let draft_id = egui::Id::new(("ai_edit_draft", &backend.id));
    let mut draft: EditDraft = ui.data_mut(|d| {
        d.get_temp::<EditDraft>(draft_id).unwrap_or_else(|| EditDraft::from_backend(backend))
    });
    let mut commit = None;

    ui.label(
        egui::RichText::new(format!("Editing {} — the all-time spend survives", backend.name))
            .size(10.0)
            .weak(),
    );
    labelled_script(ui, "Name", format!("edit_name:{}", backend.id), &mut draft.name, &backend.name);
    labelled_script(
        ui,
        "URL",
        format!("edit_url:{}", backend.id),
        &mut draft.base_url,
        backend.effective_base_url(),
    );
    labelled_script(
        ui,
        "Model",
        format!("edit_model:{}", backend.id),
        &mut draft.model,
        "model id",
    );

    ui.horizontal(|ui| {
        ui.label("Key");
        for (mode, script, label, hover) in [
            (EditKeyMode::Keep, "keep", "Leave", "Leave the key exactly as it is"),
            (EditKeyMode::Env, "env", "Env var", "Read at send time from an environment variable"),
            (EditKeyMode::Stored, "stored", "Paste", "Stored in ai.json, readable only by you"),
            (EditKeyMode::None, "none", "None", "A local server that needs no key"),
        ] {
            let selected = ui
                .selectable_value(&mut draft.key_mode, mode, label)
                .on_hover_text(hover);
            script_widget(ui, format!("edit_key_mode_{script}:{}", backend.id), selected.rect);
        }
    });
    match draft.key_mode {
        EditKeyMode::Env => {
let hint = match &backend.key {
        KeySource::Env(var) => var.clone(),
        _ => backend.provider.default_env_var().unwrap_or("API_KEY").to_string(),
    };
            labelled_script(
                ui,
                "Var",
                format!("edit_key_var:{}", backend.id),
                &mut draft.key_env,
                &hint,
            );
        }
        EditKeyMode::Stored => {
            ui.horizontal(|ui| {
                ui.label("Key");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut draft.key)
                        .password(true)
                        .hint_text("sk-… (leave blank to keep)")
                        .desired_width(f32::INFINITY),
                );
                script_widget(ui, format!("edit_key_paste:{}", backend.id), response.rect);
            });
        }
        EditKeyMode::Keep | EditKeyMode::None => {}
    }

    ui.horizontal(|ui| {
        let save = ui
            .button("Save")
            .on_hover_text("Edit in place — the all-time spend survives");
        script_widget(ui, format!("edit_save:{}", backend.id), save.rect);
        if save.clicked() {
            let key = match draft.key_mode {
                EditKeyMode::None => crate::ai::backends::KeySource::None,
                EditKeyMode::Env => {
                    let hint = match &backend.key {
                        KeySource::Env(var) => var.clone(),
                        _ => backend.provider.default_env_var().unwrap_or("API_KEY").to_string(),
                    };
                    crate::ai::backends::KeySource::Env(non_empty(&draft.key_env, &hint))
                }
                EditKeyMode::Stored if draft.key.trim().is_empty() => backend.key.clone(),
                EditKeyMode::Stored => {
                    crate::ai::backends::KeySource::Stored(draft.key.trim().to_string())
                }
                EditKeyMode::Keep => backend.key.clone(),
            };
            commit = Some(EditCommit {
                action: Some(Action::EditAiBackend {
                    id: backend.id.clone(),
                    name: non_empty(&draft.name, &backend.name),
                    base_url: non_empty(&draft.base_url, backend.effective_base_url()),
                    model: draft.model.trim().to_string(),
                    key,
                }),
                close: true,
            });
        }
        let cancel = ui.button("Cancel").on_hover_text("Close without changing this backend");
        script_widget(ui, format!("edit_cancel:{}", backend.id), cancel.rect);
        if cancel.clicked() {
            commit = Some(EditCommit { action: None, close: true });
        }
    });

    // Keep the draft only while the form stays open: closing (Save or Cancel) drops it, so
    // the next edit of this backend re-seeds from the backend.
    match commit {
        Some(EditCommit { close: true, .. }) => {
            ui.data_mut(|d| d.remove_temp::<EditDraft>(draft_id));
        }
        _ => {
            ui.data_mut(|d| d.insert_temp(draft_id, draft));
        }
    }
    commit
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.trim().to_string()
    }
}

/// Record where a pane widget a script's interaction test targets landed this frame
/// (#1627) — read back by `bearcad.ui.ai_backend_widget(name)`. UI geometry only, the
/// same policy as `pane_rect`/`pane_scroll`; never an AI state or a key.
fn script_widget(ui: &egui::Ui, name: impl Into<String>, rect: egui::Rect) {
    crate::script::remember_widget_rect(ui.ctx(), &name.into(), rect);
}

/// A labelled single-line field, with its screen rect also remembered for scripted
/// clicks (#1627).
fn labelled_script(
    ui: &mut egui::Ui,
    label: &str,
    widget: impl Into<String>,
    value: &mut String,
    hint: &str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        script_widget(ui, widget.into(), response.rect);
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KeyMode {
    /// Read from an environment variable at send time — the default, because it stores
    /// nothing.
    #[default]
    Env,
    Stored,
    None,
    /// Ask the provider for a key through its OAuth flow (#1624). Offered only where the
    /// provider has one, and the default there: it is the shortest path to a working
    /// backend.
    Connect,
}

/// The add-backend form's in-progress values. Lives in egui memory for the pane's lifetime.
#[derive(Clone, Debug, Default)]
struct Draft {
    provider: Provider,
    name: String,
    base_url: String,
    key_mode: KeyMode,
    key_env: String,
    key: String,
}

impl Draft {
    fn for_provider(provider: Provider) -> Self {
        Self {
            provider,
            // Connecting where the provider allows it; a local server usually wants no key at
            // all; everything else defaults to the environment variable it conventionally
            // uses.
            key_mode: match (provider.oauth(), provider.default_env_var()) {
                (Some(_), _) => KeyMode::Connect,
                (None, Some(_)) => KeyMode::Env,
                (None, None) => KeyMode::None,
            },
            ..Default::default()
        }
    }
}

/// How the Edit form treats the key (#1627). A backend may be OAuth-connected, hold a
/// stored key, or read one from an environment variable; the form never shows an existing
/// secret, so **Keep** is the safe way to say "leave it alone", and a pasted key is the
/// only way to replace one.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EditKeyMode {
    /// Leave the existing key untouched — the default for a backend that has one.
    #[default]
    Keep,
    Env,
    Stored,
    None,
}

/// The edit-backend form's in-progress values (#1627). Like the add form's `Draft`, it
/// lives in egui memory — a half-typed key never reaches `AppState` or a screenshot.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default)]
struct EditDraft {
    name: String,
    base_url: String,
    model: String,
    key_mode: EditKeyMode,
    key_env: String,
    key: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl EditDraft {
    fn from_backend(b: &Backend) -> Self {
        let (key_mode, key_env) = match &b.key {
            KeySource::None => (EditKeyMode::None, String::new()),
            KeySource::Env(var) => (EditKeyMode::Env, var.clone()),
            KeySource::Stored(_) | KeySource::OAuth(_) => (EditKeyMode::Keep, String::new()),
        };
        Self {
            name: b.name.clone(),
            base_url: b.base_url.clone(),
            model: b.model.clone(),
            key_mode,
            key_env,
            key: String::new(),
        }
    }
}

/// What closing the Edit form means: an action to dispatch (Edit, or none for Cancel) and
/// whether the form should stop being shown (#1627).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default)]
struct EditCommit {
    action: Option<Action>,
    close: bool,
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
