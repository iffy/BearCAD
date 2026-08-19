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
pub mod panel;

/// Live AI state on [`crate::actions::AppState`].
///
/// Only the configuration lives here so far. The conversation (#1598) is session-only and
/// will join it; the MCP server (#1605) keeps its handle here too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiState {
    /// Configured backends and the selected one, mirrored from `ai.json`.
    pub config: backends::AiConfig,
    /// Set when [`Self::config`] changes so the host writes `ai.json` after the frame.
    pub config_dirty: bool,
}

impl AiState {
    /// Load the persisted backends. Native only — the browser has no config directory, and
    /// storing a key in browser storage is not something the user opted into.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                config: backends::AiConfig::load(),
                config_dirty: false,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self::default()
        }
    }
}
