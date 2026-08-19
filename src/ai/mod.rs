//! Everything AI-related, in one pane (#1593).
//!
//! The pane has three sections — **Chat** (talk to a model about the open documents),
//! **Agents & Skill** (install the BearCAD agent skill into the AI tools on this machine),
//! and **MCP Server** (let an outside agent drive the document that is open on screen).
//!
//! Two rules hold across the whole module:
//!
//! - **Opt-in.** Nothing here reaches the network until the user configures a backend and
//!   presses send, and the MCP server stays off until switched on. A fresh install is
//!   silent.
//! - **Secrets stay put.** API keys and the MCP token never appear in a scripting return
//!   value, a Lua export, `--show-commands`, the diagnostics file, or a screenshot.

pub mod api;
pub mod backends;
#[cfg(not(target_arch = "wasm32"))]
pub mod chat;
pub mod context;
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // Started from the pane in #1606.
pub mod mcp;
pub mod panel;
pub mod pricing;
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // Installed by the CLI (#1603) and the pane (#1604).
pub mod skill;
#[cfg(not(target_arch = "wasm32"))]
pub mod providers;
#[cfg(not(target_arch = "wasm32"))]
pub mod transport;

/// A handle to the one [`AiState`] the app has.
///
/// Every tab's [`crate::actions::AppState`] holds a clone of the *same* handle: backends and
/// the conversation belong to the app, not to a document, and switching tabs swaps whole
/// `AppState`s (`tabs::Workspace`). Sharing the state is what keeps a reply streaming into
/// the pane while the user looks at another tab.
pub type SharedAi = std::rc::Rc<std::cell::RefCell<AiState>>;

/// Live AI state, shared by every tab through [`SharedAi`].
///
/// Only the configuration lives here so far. The conversation (#1598) is session-only and
/// will join it; the MCP server (#1605) keeps its handle here too.
#[derive(Debug, Default)]
pub struct AiState {
    /// Configured backends and the selected one, mirrored from `ai.json`.
    pub config: backends::AiConfig,
    /// Set when [`Self::config`] changes so the host writes `ai.json` after the frame.
    pub config_dirty: bool,
    /// The session-only conversation (#1598). Native only: the browser build has no
    /// transport to answer with.
    #[cfg(not(target_arch = "wasm32"))]
    pub chat: chat::Conversation,
    /// A script asked what the next message would send (`bearcad.ai.context_preview`).
    /// Answered by the frame loop, which is the only place every open document is reachable.
    pub preview_wanted: bool,
    /// The answer to the last preview request, or `None` while one is outstanding.
    pub preview: Option<context::Context>,
    /// Set when something asks for the Agents & Skill section specifically (Help ▸ Install
    /// AI Agent Skill…), so the pane opens showing it rather than the chat (#1604).
    pub open_skill_section: bool,
    /// Set for one frame when something asks every section to open (or collapse) at once —
    /// `bearcad.ui.ai_sections(...)` (#1619). The headers remember it from there.
    pub sections_open: Option<bool>,
    /// The running MCP server (#1605), or `None` — which is the default, and the state a
    /// fresh install stays in until the user switches it on.
    #[cfg(not(target_arch = "wasm32"))]
    pub mcp: Option<mcp::Server>,
}

impl AiState {
    /// Load the persisted backends. Native only — the browser has no config directory, and
    /// storing a key in browser storage is not something the user opted into.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                config: backends::AiConfig::load(),
                ..Self::default()
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self::default()
        }
    }
}

#[cfg(test)]
mod privacy_tests {
    //! The privacy pass (#1609), as tests rather than prose: each one fails if a later
    //! change would let something out that should not get out.

    use super::*;
    use crate::actions::{Action, AppState};
    use backends::{Backend, KeySource, Provider};

    const SECRET: &str = "sk-do-not-leak-this-anywhere";

    /// An app state with one configured backend holding a stored key.
    fn state_with_key() -> AppState {
        let state = AppState::default();
        state.ai.borrow_mut().config.add(Backend {
            key: KeySource::Stored(SECRET.into()),
            ..Backend::preset(Provider::Anthropic)
        });
        state
    }

    #[test]
    fn a_fresh_install_is_silent() {
        // No backends, nothing selected, no MCP server: there is nothing for the app to
        // contact and nothing listening.
        let config = backends::AiConfig::default();
        assert!(config.backends.is_empty());
        assert!(config.selected.is_none());
        assert!(!config.mcp_enabled, "the MCP server is off until switched on");
        assert!(config.mcp_token.is_empty(), "and has no token until it runs");

        let state = AppState::default();
        assert!(state.ai.borrow().mcp.is_none());
        assert!(!state.panes.is_visible(crate::actions::Pane::Ai), "the pane is closed too");
    }

    #[test]
    fn nothing_is_sent_before_a_backend_is_configured() {
        let mut state = AppState::default();
        let result = state.apply(Action::SendAiMessage { text: "hello".into() });
        assert!(matches!(result, crate::actions::ActionResult::Err(_)));
        assert!(state.ai.borrow().chat.entries.is_empty(), "not even a recorded turn");
        assert!(state.ai.borrow().chat.pending_consent.is_none());
    }

    #[test]
    fn the_first_message_to_a_backend_waits_for_consent() {
        let mut state = state_with_key();
        state.apply(Action::SendAiMessage { text: "how wide is it?".into() });

        let ai = state.ai.borrow();
        assert_eq!(
            ai.chat.pending_consent.as_deref(),
            Some("how wide is it?"),
            "the message is held, not sent"
        );
        assert!(ai.chat.entries.is_empty(), "nothing is on the wire, or in the thread");
        assert!(!ai.chat.awaiting_context, "and no context has been built");
        drop(ai);

        // Declining puts the message back and still sends nothing.
        state.apply(Action::ResolveAiConsent { agreed: false });
        let ai = state.ai.borrow();
        assert!(ai.chat.pending_consent.is_none());
        assert_eq!(ai.chat.input, "how wide is it?", "what was typed is not lost");
        assert!(ai.chat.entries.is_empty());
        assert!(!ai.config.selected().unwrap().consented, "declining is not consent");
    }

    #[test]
    fn consent_is_remembered_per_backend_and_asked_once() {
        let mut state = state_with_key();
        state.apply(Action::SendAiMessage { text: "first".into() });
        state.apply(Action::ResolveAiConsent { agreed: true });
        assert!(state.ai.borrow().config.selected().unwrap().consented);
        assert!(state.ai.borrow().config_dirty, "the answer is persisted");

        // A second message goes straight through.
        state.ai.borrow_mut().chat.clear();
        state.apply(Action::SendAiMessage { text: "second".into() });
        let ai = state.ai.borrow();
        assert!(ai.chat.pending_consent.is_none(), "asked once, not every time");
        assert!(ai.chat.awaiting_context, "this one is really being sent");
        drop(ai);

        // A different backend has its own answer.
        let second = state.ai.borrow_mut().config.add(Backend::preset(Provider::OpenAi));
        assert!(!state.ai.borrow().config.get(&second).unwrap().consented);
    }

    #[test]
    fn a_key_never_appears_in_a_recorded_script_line() {
        // --show-commands output is pasted into bug reports.
        let backend = Backend {
            key: KeySource::Stored(SECRET.into()),
            ..Backend::preset(Provider::Anthropic)
        };
        let line = crate::script::Instruction::AddAiBackend { backend: backend.clone() }
            .as_lua_in(None);
        assert!(!line.contains(SECRET), "recorded line leaked the key: {line}");
        assert!(line.contains("stored"), "it says where the key came from: {line}");

        let line = crate::script::Instruction::UpdateAiBackend {
            id: "anthropic".into(),
            backend,
        }
        .as_lua_in(None);
        assert!(!line.contains(SECRET), "recorded line leaked the key: {line}");
    }

    #[test]
    fn a_key_never_appears_in_a_debug_dump_of_app_state() {
        // `{:?}` of AI state reaches the diagnostics file and panic output.
        let state = state_with_key();
        let dumped = format!("{:?}", state.ai.borrow());
        assert!(!dumped.contains(SECRET), "Debug leaked the key: {dumped}");
    }

    #[test]
    fn the_mcp_token_never_appears_in_a_debug_dump() {
        let server = mcp::Server::start(0, "token-abc-123".into()).expect("start");
        let dumped = format!("{server:?}");
        assert!(!dumped.contains("token-abc-123"), "Debug leaked the token: {dumped}");
        assert!(dumped.contains("redacted"));
    }

    #[test]
    fn a_document_export_carries_no_ai_configuration_at_all() {
        // A .bearcad document and its Lua export describe geometry. Nothing about which
        // service the user talks to, or with what key, belongs in a file they share.
        let state = state_with_key();
        let exported = crate::export_lua::document_to_lua(&state.doc);
        assert!(!exported.contains(SECRET));
        assert!(!exported.contains("bearcad.ai"), "no AI state in an export: {exported}");

        let json = serde_json::to_string(&state.doc).expect("document json");
        assert!(!json.contains(SECRET));
        assert!(!json.contains("mcp_token"));
    }

    #[test]
    fn the_context_says_when_it_left_something_out() {
        // What is sent is shown exactly, and a cut is always declared (#1597).
        let doc = crate::model::Document::default();
        let inputs = vec![context::DocumentInput {
            title: "Untitled".into(),
            active: true,
            doc: &doc,
        }];
        let full = context::build(context::ContextScope::Document, &inputs);
        assert!(!full.truncated);
        let cut = context::build_with_budget(context::ContextScope::Document, &inputs, 4);
        assert!(cut.truncated && cut.text.contains("truncated"));
    }
}
