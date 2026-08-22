//! The AI configuration file — `ai.json`, next to `settings.json` (#1595/#1633).
//!
//! BearCAD does not talk to AI services; agents talk to *it*. So all this file holds is how
//! the local MCP server (#1605) is set up: whether it starts with the app, the port it
//! listens on, and the bearer token clients must send.
//!
//! It is written **0600** all the same: the token is what stands between a stranger on the
//! machine and the open document.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfig {
    /// Whether the local MCP server starts with the app (#1605). Off by default.
    #[serde(default)]
    pub mcp_enabled: bool,
    /// The port it listens on. 0 asks the OS for a free one.
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// The bearer token clients must send. Persisted so a copied client configuration keeps
    /// working across restarts; regenerating it is a deliberate act.
    #[serde(default)]
    pub mcp_token: String,
}

/// The port the MCP server offers by default. Nothing standard claims it, and a fixed
/// default means a client config keeps working across restarts.
fn default_mcp_port() -> u16 {
    8721
}

/// Redacts the token: `{:?}` of app state reaches the diagnostics file and panic output.
impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("mcp_enabled", &self.mcp_enabled)
            .field("mcp_port", &self.mcp_port)
            .field("mcp_token", &if self.mcp_token.is_empty() { "none" } else { "redacted" })
            .finish()
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            // Opt-in: nothing listens until the user says so.
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
            mcp_token: String::new(),
        }
    }
}

impl AiConfig {
    /// Load from the standard location; missing or malformed → defaults, no error.
    /// (Same policy as settings: a config file is never worth an error dialog at boot.)
    pub fn load() -> Self {
        config_path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("no config directory on this platform")?;
        self.save_to(&path)
    }

    /// Write the file, owner-read/write only. The permissions are set **before** the token
    /// is written, so it is never briefly world-readable.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        create_private(path)?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

/// Create (or truncate) `path` with owner-only permissions, before anything is written to
/// it. On platforms without unix permissions this just makes sure the file exists.
fn create_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        // An existing file created before this ran (or by an older build) keeps its old
        // mode through `create`, so set it explicitly too.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Where the AI config lives: the same directory as `settings.json`, in its own file.
///
/// `BEARCAD_AI_CONFIG` overrides the location. Interaction tests and CI set it so a test
/// that switches the server on cannot touch the real config on the machine running it.
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BEARCAD_AI_CONFIG") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    default_config_path()
}

/// Where the config lives when nothing overrides it.
fn default_config_path() -> Option<PathBuf> {
    Some(crate::settings::settings_path()?.with_file_name("ai.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bearcad_ai_config_{name}_{}.json", std::process::id()))
    }

    #[test]
    fn a_missing_file_loads_the_defaults() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        let config = AiConfig::load_from(&path);
        assert_eq!(config, AiConfig::default());
        assert!(!config.mcp_enabled, "the server is off until switched on");
    }

    #[test]
    fn the_token_round_trips_through_an_owner_only_file() {
        let path = temp_path("roundtrip");
        let config = AiConfig {
            mcp_enabled: true,
            mcp_port: 9111,
            mcp_token: "token-abc".to_string(),
            ..AiConfig::default()
        };
        config.save_to(&path).expect("save");
        assert_eq!(AiConfig::load_from(&path), config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the token file stays owner-only");
        }
        let _ = std::fs::remove_file(&path);
    }
}
