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
