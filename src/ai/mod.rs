//! Everything AI — which, since #1633, is one direction only: **agents drive BearCAD**.
//!
//! BearCAD does not reach out to a model. Outside agents reach *in*, two ways, both offered
//! by the **AI** pane (Integration ▸ AI):
//!
//! - the **agent skill** — a file an agent reads to learn how to drive BearCAD (#1603);
//! - the **MCP server** — a local endpoint that lets one drive the open document (#1605).
//!
//! Two rules hold across the whole module:
//!
//! - **Opt-in.** The MCP server stays off until switched on. A fresh install is silent.
//! - **Secrets stay put.** The MCP token never appears in a scripting return value, a Lua
//!   export, `--show-commands`, the diagnostics file, or a screenshot.

pub mod api;
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // Started from the pane in #1606.
pub mod mcp;
pub mod panel;
pub mod signatures;
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // Installed by the CLI (#1603) and the pane (#1604).
pub mod skill;

/// A handle to the one [`AiState`] the app has.
///
/// Every tab's [`crate::actions::AppState`] holds a clone of the *same* handle: the server
/// belongs to the app, not to a document, and switching tabs swaps whole `AppState`s
/// (`tabs::Workspace`). Sharing the state is what keeps the server running while the user
/// looks at another tab.
pub type SharedAi = std::rc::Rc<std::cell::RefCell<AiState>>;

/// Live AI state, shared by every tab through [`SharedAi`].
#[derive(Debug, Default)]
pub struct AiState {
    /// The MCP server's configuration, mirrored from `ai.json`.
    pub config: config::AiConfig,
    /// Set when [`Self::config`] changes so the host writes `ai.json` after the frame.
    pub config_dirty: bool,
    /// Set when something asks for the agent-skill section specifically (Integration ▸ AI ▸
    /// Install AI Agent Skill…), so the pane opens showing it (#1604).
    pub open_skill_section: bool,
    /// Set for one frame when something asks for the MCP Server section specifically
    /// (Integration ▸ AI ▸ MCP Server…).
    pub open_mcp_section: bool,
    /// Set for one frame when something asks every section to open (or collapse) at once —
    /// `bearcad.ui.ai_sections(...)` (#1619). The headers remember it from there.
    pub sections_open: Option<bool>,
    /// The running MCP server (#1605), or `None` — which is the default, and the state a
    /// fresh install stays in until the user switches it on.
    #[cfg(not(target_arch = "wasm32"))]
    pub mcp: Option<mcp::Server>,
}

impl AiState {
    /// Load the persisted configuration. Native only — the browser has no config directory
    /// and no MCP server to configure.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                config: config::AiConfig::load(),
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
    use crate::actions::AppState;

    #[test]
    fn a_fresh_install_is_silent() {
        // Nothing listening, and nothing to listen with: the MCP server is off until the
        // user switches it on.
        let config = config::AiConfig::default();
        assert!(!config.mcp_enabled, "the MCP server is off until switched on");
        assert!(config.mcp_token.is_empty(), "and has no token until it runs");

        let state = AppState::default();
        assert!(state.ai.borrow().mcp.is_none());
        assert!(
            !state.panes.is_visible(crate::actions::Pane::Ai),
            "the config pane is closed too"
        );
    }

    /// #1633: BearCAD talks to no AI service, so there is no backend, key or conversation
    /// for anything to leak — and the document never carried the MCP token either.
    #[test]
    fn a_document_export_carries_no_ai_configuration_at_all() {
        let state = AppState::default();
        let exported = crate::export_lua::document_to_lua(&state.doc);
        assert!(!exported.contains("bearcad.ai"), "no AI state in an export: {exported}");

        let json = serde_json::to_string(&state.doc).expect("document json");
        assert!(!json.contains("mcp_token"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_mcp_token_never_appears_in_a_debug_dump() {
        let server = mcp::Server::start(0, "token-abc-123".into()).expect("start");
        let dumped = format!("{server:?}");
        assert!(!dumped.contains("token-abc-123"), "Debug leaked the token: {dumped}");
        assert!(dumped.contains("redacted"));

        let mut state = AiState::default();
        state.config.mcp_token = "token-abc-123".into();
        assert!(
            !format!("{state:?}").contains("token-abc-123"),
            "Debug leaked the configured token"
        );
    }
}
