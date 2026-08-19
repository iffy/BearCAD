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

pub mod panel;
