//! AI backends: which service a conversation talks to, and where its key lives (#1595).
//!
//! A **backend** is one endpoint plus one model — "Claude (Opus 5)", "Ollama on this
//! laptop". The user adds them, removes them, and picks which one the current conversation
//! uses.
//!
//! Keys are the sensitive part, so they live in their own file — `ai.json`, next to
//! `settings.json` but written **0600** — rather than in the shareable settings file. A key
//! can also be left out of the file entirely by naming an environment variable to read it
//! from, which is the safer option and the one an existing `ANTHROPIC_API_KEY` already
//! satisfies.
//!
//! Nothing in this module's public surface returns a key. [`Backend::key_description`] says
//! *where* the key comes from; only [`Backend::resolve_key`] — used by the transport, at
//! the moment of the request — produces the secret itself, and `Debug` redacts it so a
//! stray `{:?}` in a log line cannot leak it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Which wire protocol a backend speaks.
///
/// Only two shapes exist in practice: Anthropic's Messages API, and OpenAI's chat
/// completions, which xAI and every local server (Ollama, LM Studio, vLLM) also implement.
/// [`Provider::XAi`] and [`Provider::OpenAiCompatible`] are separate variants so the UI can
/// present them by name and default their URLs, not because the wire format differs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Anthropic,
    OpenAi,
    /// A gateway in front of every other vendor's models (#1617). The one backend that can
    /// be connected with a browser click rather than a pasted key (#1624).
    OpenRouter,
    XAi,
    /// Any endpoint speaking OpenAI's chat-completions shape: a local model server, a
    /// gateway, a provider not listed above.
    OpenAiCompatible,
}

impl Provider {
    /// Every provider, in the order the "add a backend" UI offers them.
    pub const ALL: &'static [Provider] = &[
        Provider::Anthropic,
        Provider::OpenAi,
        Provider::OpenRouter,
        Provider::XAi,
        Provider::OpenAiCompatible,
    ];

    /// Stable name used in `ai.json` and in scripts.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::XAi => "xai",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Human-readable name for the picker.
    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::OpenRouter => "OpenRouter",
            Self::XAi => "xAI (Grok)",
            Self::OpenAiCompatible => "OpenAI-compatible",
        }
    }

/// The API root a new backend of this provider starts with. Editable afterwards — a
    /// gateway or a self-hosted server is the whole point of the compatible variant.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::XAi => "https://api.x.ai/v1",
            // Ollama's OpenAI-compatible endpoint, the most common local server.
            Self::OpenAiCompatible => "http://localhost:11434/v1",
        }
    }

    /// The model a new backend starts with, or `""` when there is no sensible guess.
    ///
    /// A starting point only. Adding a backend never asks for a model (#1617): the backend
    /// itself is asked which models it has once it is connected, and the answer fills the
    /// **Model** dropdown. A gateway carrying hundreds of models has no default at all, so
    /// one of those starts empty and is unusable until a model is picked.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-opus-5",
            Self::OpenAi => "gpt-5",
            Self::OpenRouter => "",
            Self::XAi => "grok-4",
            Self::OpenAiCompatible => "llama3.2",
        }
    }

    /// How to connect to this provider with a browser instead of a pasted key (#1624), or
    /// `None` for one that offers no such flow — those still take an API key.
    pub fn oauth(self) -> Option<OAuthService> {
        match self {
            // OpenRouter's PKCE flow: send the user to `/auth` with a code challenge, take
            // the code it hands back, trade it for a key at `/auth/keys`.
            Self::OpenRouter => Some(OAuthService {
                authorize_url: "https://openrouter.ai/auth",
                token_path: "/auth/keys",
            }),
            Self::Anthropic | Self::OpenAi | Self::XAi | Self::OpenAiCompatible => None,
        }
    }

    /// The environment variable this provider's key conventionally lives in, which a new
    /// backend defaults to reading. `None` for local servers, which usually want no key.
    pub fn default_env_var(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::OpenRouter => Some("OPENROUTER_API_KEY"),
            Self::XAi => Some("XAI_API_KEY"),
            Self::OpenAiCompatible => None,
        }
    }
}

/// A provider's PKCE OAuth endpoints (#1624).
///
/// PKCE only — BearCAD is a desktop app and has no client secret to keep, which is exactly
/// the case the flow was designed for. The code challenge, the loopback callback and the
/// exchange live in [`super::oauth`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OAuthService {
    /// Where the browser is sent to ask the user for permission.
    pub authorize_url: &'static str,
    /// Path under the backend's API root that trades the code for a key.
    pub token_path: &'static str,
}

/// Where a backend's API key comes from.
///
/// `Debug` is implemented by hand: deriving it would print the key.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// No key at all — a local server that does not ask for one.
    #[default]
    None,
    /// Read at request time from this environment variable. Nothing is stored on disk.
    Env(String),
    /// The key itself, kept in `ai.json` (0600).
    Stored(String),
    /// A key the provider issued through its OAuth flow (#1624), kept in `ai.json` the same
    /// way. Separate from [`Self::Stored`] so the pane can say **Connected** and offer to
    /// reconnect rather than asking for a key the user never typed. Empty until the flow
    /// finishes.
    OAuth(String),
}

impl std::fmt::Debug for KeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Env(var) => write!(f, "Env({var})"),
            Self::Stored(_) => write!(f, "Stored(<redacted>)"),
            Self::OAuth(_) => write!(f, "OAuth(<redacted>)"),
        }
    }
}

/// One configured service: an endpoint, a model, and a key source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backend {
    /// Stable slug, used by scripts and to remember the selection across restarts.
    pub id: String,
    /// What the picker shows.
    pub name: String,
    pub provider: Provider,
    pub model: String,
    /// API root. Empty means the provider's default.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub key: KeySource,
    /// What this backend actually charges, per million tokens (#1599). `None` uses the
    /// rates shipped with this build, which may be out of date — published prices change
    /// faster than releases do.
    #[serde(default)]
    pub price: Option<super::pricing::Price>,
    /// Everything this backend has cost since it was added, or since the user reset it.
    /// Persisted: the answer to "what has this cost me?" outlives the conversation.
    #[serde(default)]
    pub spend: super::pricing::Spend,
    /// Whether the user has agreed, once, to send document content to this backend (#1609).
    /// A new backend starts at `false`, so the first message asks before anything leaves
    /// the machine.
    #[serde(default)]
    pub consented: bool,
}

impl Backend {
    /// A backend with this provider's defaults, named after the provider.
    pub fn preset(provider: Provider) -> Self {
        Self {
            id: provider.as_str().to_string(),
            name: provider.label().to_string(),
            provider,
            model: provider.default_model().to_string(),
            base_url: provider.default_base_url().to_string(),
            key: match provider.default_env_var() {
                Some(var) => KeySource::Env(var.to_string()),
                None => KeySource::None,
            },
            price: None,
            spend: super::pricing::Spend::default(),
            consented: false,
        }
    }

    /// The host a message to this backend would reach — what the confirmation names, since
    /// "where does this go?" is the question worth answering (#1609).
    pub fn host(&self) -> String {
        self.effective_base_url()
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_else(|| self.effective_base_url())
            .to_string()
    }

    /// Whether this backend runs on this machine. A local model server is worth saying so
    /// about: nothing leaves the machine at all.
    pub fn is_local(&self) -> bool {
        let host = self.host();
        let host = host.split(':').next().unwrap_or_default();
        matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
    }

    /// The API root to call: the configured one, or the provider default when blank.
    pub fn effective_base_url(&self) -> &str {
        if self.base_url.trim().is_empty() {
            self.provider.default_base_url()
        } else {
            self.base_url.trim_end_matches('/')
        }
    }

    /// Where this backend's key comes from, in words — `"stored"`, `"env:NAME"`, `"none"`.
    /// This is what scripts and the UI show; the key itself is never part of it.
    pub fn key_description(&self) -> String {
        match &self.key {
            KeySource::None => "none".to_string(),
            KeySource::Env(var) => format!("env:{var}"),
            KeySource::Stored(_) => "stored".to_string(),
            KeySource::OAuth(key) if key.trim().is_empty() => "not connected".to_string(),
            KeySource::OAuth(_) => "connected".to_string(),
        }
    }

    /// The key to send with a request, or `None` when this backend has none. The only
    /// function in the module that yields the secret — call it at request time, do not
    /// hold the result.
    pub fn resolve_key(&self) -> Option<String> {
        match &self.key {
            KeySource::None => None,
            KeySource::Env(var) => match std::env::var(var) {
                Ok(value) if !value.trim().is_empty() => Some(value),
                _ => None,
            },
            KeySource::Stored(key) | KeySource::OAuth(key) if !key.trim().is_empty() => {
                Some(key.clone())
            }
            KeySource::Stored(_) | KeySource::OAuth(_) => None,
        }
    }

    /// Whether a request to this backend would carry the credential it needs.
    pub fn has_key(&self) -> bool {
        matches!(self.key, KeySource::None) || self.resolve_key().is_some()
    }

    /// Why this backend cannot be used, for the UI to show next to it.
    pub fn unusable_reason(&self) -> Option<String> {
        if !self.has_key() {
            return Some(match &self.key {
                KeySource::Env(var) => format!("${var} is not set"),
                KeySource::OAuth(_) => format!("not connected to {}", self.provider.label()),
                _ => "no API key".to_string(),
            });
        }
        if self.model.trim().is_empty() {
            return Some("no model chosen".to_string());
        }
        None
    }
}

/// Every configured backend, plus which one the next message goes to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub backends: Vec<Backend>,
    /// Id of the selected backend. `None` when nothing is configured.
    #[serde(default)]
    pub selected: Option<String>,
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

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            backends: Vec::new(),
            selected: None,
            // Opt-in: nothing listens until the user says so.
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
            mcp_token: String::new(),
        }
    }
}

impl AiConfig {
    /// The backend the next message would use.
    pub fn selected(&self) -> Option<&Backend> {
        let id = self.selected.as_deref()?;
        self.backends.iter().find(|b| b.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&Backend> {
        self.backends.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Backend> {
        self.backends.iter_mut().find(|b| b.id == id)
    }

    /// Add a backend, giving it a unique id derived from its name. Returns the id it got.
    /// The first backend added becomes the selected one — otherwise a user who adds one
    /// backend and types a message would be told nothing is selected.
    pub fn add(&mut self, mut backend: Backend) -> String {
        backend.id = self.unique_id(if backend.id.trim().is_empty() {
            &backend.name
        } else {
            &backend.id
        });
        let id = backend.id.clone();
        self.backends.push(backend);
        if self.selected.is_none() {
            self.selected = Some(id.clone());
        }
        id
    }

    /// Remove a backend. Removing the selected one moves the selection to whatever remains
    /// (or clears it), so the app is never pointed at a backend that is gone.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.backends.iter().position(|b| b.id == id) else {
            return false;
        };
        self.backends.remove(index);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.backends.first().map(|b| b.id.clone());
        }
        true
    }

    /// Point the next message at `id`. Fails if there is no such backend.
    pub fn select(&mut self, id: &str) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err(format!("no AI backend '{id}'"));
        }
        self.selected = Some(id.to_string());
        Ok(())
    }

    /// Edit a backend in place (#1627): name, base URL, model and key change; the id,
    /// per-model rate override, all-time spend and consent all survive. The old
    /// remove-and-re-add flow threw the spend away — an edit is an edit of one entry,
    /// and keeps the backend the conversation is already consented to.
    pub fn edit(
        &mut self,
        id: &str,
        name: String,
        base_url: String,
        model: String,
        key: KeySource,
    ) -> Result<(), String> {
        let Some(existing) = self.backends.iter_mut().find(|b| b.id == id) else {
            return Err(format!("no AI backend '{id}'"));
        };
        existing.name = name;
        existing.base_url = base_url;
        existing.model = model;
        existing.key = key;
        Ok(())
    }

    /// A slug not already taken: `claude`, then `claude-2`, `claude-3`, …
    fn unique_id(&self, from: &str) -> String {
        let base = slug(from);
        let base = if base.is_empty() { "backend".to_string() } else { base };
        if !self.backends.iter().any(|b| b.id == base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|candidate| !self.backends.iter().any(|b| &b.id == candidate))
            .expect("an unused suffix exists")
    }

    /// Load from the standard location; missing or malformed → no backends, no error.
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

    /// Write the file, owner-read/write only. The permissions are set **before** the key
    /// is written, so the secret is never briefly world-readable.
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

/// Where the AI config lives: the same directory as `settings.json`, in its own file so
/// the settings file stays free of secrets.
///
/// `BEARCAD_AI_CONFIG` overrides the location. Interaction tests and CI set it so a test
/// that adds a backend cannot touch the real one on the machine running it.
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

/// Lowercase, hyphen-separated, alphanumeric — a stable id from a display name.
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bearcad-ai-test-{name}.json"))
    }

    #[test]
    fn presets_cover_every_provider_with_usable_defaults() {
        for &provider in Provider::ALL {
            let backend = Backend::preset(provider);
            assert_eq!(backend.provider, provider);
            assert!(
                backend.effective_base_url().starts_with("http"),
                "{provider:?} needs a base URL"
            );
            assert!(!backend.id.is_empty(), "{provider:?} needs a stable id");
        }
    }

    #[test]
    fn openrouter_is_a_gateway_that_starts_without_a_model() {
        // #1617: a gateway carries hundreds of models, so guessing one would be worse than
        // asking. The backend is added first and the model is picked from what it reports.
        let backend = Backend::preset(Provider::OpenRouter);
        assert_eq!(backend.effective_base_url(), "https://openrouter.ai/api/v1");
        assert_eq!(backend.model, "");
        // With the key in hand, the only thing still missing is the model.
        std::env::set_var("OPENROUTER_API_KEY", "sk-or-from-the-environment");
        assert_eq!(backend.unusable_reason().as_deref(), Some("no model chosen"));

        let chosen = Backend { model: "anthropic/claude-opus-5".into(), ..backend };
        assert!(chosen.unusable_reason().is_none());
        assert!(chosen.unusable_reason().is_none());
        std::env::remove_var("OPENROUTER_API_KEY");
    }

    #[test]
    fn only_providers_with_a_pkce_flow_offer_to_connect() {
        // #1624: connecting is a browser click where the provider supports it, and a pasted
        // key everywhere else. Nothing here invents a flow a provider does not have.
        let service = Provider::OpenRouter.oauth().expect("OpenRouter connects");
        assert_eq!(service.authorize_url, "https://openrouter.ai/auth");
        assert_eq!(service.token_path, "/auth/keys");
        for provider in [
            Provider::Anthropic,
            Provider::OpenAi,
            Provider::XAi,
            Provider::OpenAiCompatible,
        ] {
            assert!(provider.oauth().is_none(), "{provider:?} has no PKCE flow");
        }
    }

    #[test]
    fn a_connected_key_reads_back_as_connected_and_never_as_itself() {
        const SECRET: &str = "sk-or-v1-issued-by-the-flow";
        let backend = Backend {
            key: KeySource::OAuth(SECRET.into()),
            model: "anthropic/claude-opus-5".into(),
            ..Backend::preset(Provider::OpenRouter)
        };
        assert_eq!(backend.key_description(), "connected");
        assert!(backend.unusable_reason().is_none());
        let debugged = format!("{backend:?}");
        assert!(!debugged.contains(SECRET), "Debug leaked the key: {debugged}");
        assert_eq!(backend.resolve_key().as_deref(), Some(SECRET));

        // Before the flow finishes there is no key, and the pane says so in those words.
        let waiting = Backend {
            key: KeySource::OAuth(String::new()),
            ..backend
        };
        assert_eq!(waiting.key_description(), "not connected");
        assert!(waiting.resolve_key().is_none());
        assert_eq!(
            waiting.unusable_reason().as_deref(),
            Some("not connected to OpenRouter")
        );
    }

    #[test]
    fn adding_selects_the_first_backend_and_makes_ids_unique() {
        let mut config = AiConfig::default();
        assert!(config.selected().is_none());

        let first = config.add(Backend::preset(Provider::Anthropic));
        assert_eq!(config.selected().map(|b| b.id.as_str()), Some(first.as_str()));

        // A second backend with a colliding name gets its own id, and does not steal the
        // selection.
        let second = config.add(Backend {
            id: String::new(),
            name: "Anthropic".into(),
            ..Backend::preset(Provider::Anthropic)
        });
        assert_ne!(first, second);
        assert_eq!(config.selected().map(|b| b.id.as_str()), Some(first.as_str()));

        config.select(&second).expect("select the second");
        assert_eq!(config.selected().map(|b| b.id.as_str()), Some(second.as_str()));
        assert!(config.select("nope").is_err());
    }

    #[test]
    fn editing_a_backend_changes_fields_in_place_and_keeps_its_all_time_spend() {
        // #1627: before this, changing a model or a key meant remove-and-re-add, which
        // discarded the all-time spend. An edit must change the one entry — same id,
        // spend, consent, unnamed rates — and only the fields the user edited.
        let mut config = AiConfig::default();
        let id = config.add(Backend {
            key: KeySource::Stored("sk-old".into()),
            model: "gpt-5".into(),
            ..Backend::preset(Provider::OpenAi)
        });
        let backend = config.get_mut(&id).unwrap();
        backend.consented = true;
        backend.price = Some(super::super::pricing::Price::new(2.0, 8.0));
        backend.spend = super::super::pricing::Spend {
            input_tokens: 1_000,
            output_tokens: 300,
            cost: 0.42,
            exchanges: 5,
            ..super::super::pricing::Spend::default()
        };

        config
            .edit(
                &id,
                "My Company Models".into(),
                "https://gateway.example.com/v1".into(),
                "gpt-5-mini".into(),
                KeySource::Stored("sk-new".into()),
            )
            .expect("editing an existing backend edits it");

        assert_eq!(config.backends.len(), 1, "an edit is an edit, not a new entry");
        assert_eq!(config.selected().map(|b| b.id.as_str()), Some(id.as_str()));
        let edited = config.get(&id).expect("the same entry comes back");
        assert_eq!(edited.name, "My Company Models");
        assert_eq!(edited.model, "gpt-5-mini");
        assert_eq!(edited.base_url, "https://gateway.example.com/v1");
        assert_eq!(edited.key, KeySource::Stored("sk-new".into()));
        assert_eq!(edited.provider, Provider::OpenAi, "the provider does not change");
        assert_eq!(edited.spend.cost, 0.42, "the all-time spend survives");
        assert_eq!(edited.spend.exchanges, 5);
        assert_eq!(edited.spend.tokens(), 1_300);
        assert!(edited.consented, "consent survives too");
        assert_eq!(edited.price.map(|p| p.input), Some(2.0), "the rate override survives");

        // The id is preserved, so a script or a saved config can still find the backend.
        assert!(!edited.id.is_empty());
    }

    #[test]
    fn editing_an_unknown_backend_fails() {
        let mut config = AiConfig::default();
        assert!(
            config
                .edit("ghost", "Nope".into(), String::new(), String::new(), KeySource::None)
                .is_err(),
            "editing a backend that does not exist must fail"
        );
        assert!(config.backends.is_empty());
    }

    #[test]
    fn removing_the_selected_backend_moves_the_selection() {
        let mut config = AiConfig::default();
        let first = config.add(Backend::preset(Provider::Anthropic));
        let second = config.add(Backend::preset(Provider::OpenAi));
        config.select(&second).unwrap();

        assert!(config.remove(&second));
        assert_eq!(config.selected().map(|b| b.id.as_str()), Some(first.as_str()));

        assert!(config.remove(&first));
        assert!(config.selected().is_none(), "nothing left to select");
        assert!(!config.remove("gone"));
    }

    #[test]
    fn a_stored_key_never_shows_up_in_a_description_or_a_debug_line() {
        const SECRET: &str = "sk-super-secret-value";
        let backend = Backend {
            key: KeySource::Stored(SECRET.into()),
            ..Backend::preset(Provider::OpenAi)
        };
        assert_eq!(backend.key_description(), "stored");
        assert!(!backend.key_description().contains(SECRET));

        // `{:?}` reaches logs and panics; it must not carry the key.
        let debugged = format!("{backend:?}");
        assert!(!debugged.contains(SECRET), "Debug leaked the key: {debugged}");
        assert!(debugged.contains("redacted"));

        // The transport, and only the transport, can still get it.
        assert_eq!(backend.resolve_key().as_deref(), Some(SECRET));
    }

    #[test]
    fn an_env_key_is_read_at_request_time_and_named_in_the_description() {
        let var = "BEARCAD_TEST_AI_KEY";
        let backend = Backend {
            key: KeySource::Env(var.into()),
            ..Backend::preset(Provider::Anthropic)
        };
        assert_eq!(backend.key_description(), format!("env:{var}"));

        std::env::remove_var(var);
        assert!(backend.resolve_key().is_none());
        assert_eq!(backend.unusable_reason().as_deref(), Some("$BEARCAD_TEST_AI_KEY is not set"));

        std::env::set_var(var, "key-from-the-environment");
        assert_eq!(backend.resolve_key().as_deref(), Some("key-from-the-environment"));
        assert!(backend.unusable_reason().is_none());
        std::env::remove_var(var);
    }

    #[test]
    fn a_new_backend_has_not_been_consented_to_yet() {
        // #1609: nothing goes to a backend until the user has agreed once.
        for &provider in Provider::ALL {
            assert!(
                !Backend::preset(provider).consented,
                "{provider:?} must start unconsented"
            );
        }
    }

    #[test]
    fn a_backend_names_the_host_it_would_reach_and_knows_if_it_is_local() {
        assert_eq!(Backend::preset(Provider::Anthropic).host(), "api.anthropic.com");
        assert!(!Backend::preset(Provider::Anthropic).is_local());
        // The local-server preset is Ollama on this machine.
        let local = Backend::preset(Provider::OpenAiCompatible);
        assert_eq!(local.host(), "localhost:11434");
        assert!(local.is_local(), "nothing leaves the machine for a local model");
        let loopback = Backend {
            base_url: "http://127.0.0.1:1234/v1".into(),
            ..Backend::preset(Provider::OpenAiCompatible)
        };
        assert!(loopback.is_local());
    }

    #[test]
    fn a_local_backend_with_no_key_is_usable() {
        let backend = Backend::preset(Provider::OpenAiCompatible);
        assert_eq!(backend.key_description(), "none");
        assert!(backend.has_key());
        assert!(backend.unusable_reason().is_none());
    }

    #[test]
    fn config_round_trips_through_its_file() {
        let path = temp_path("round-trip");
        let _ = std::fs::remove_file(&path);
        let mut config = AiConfig::default();
        config.add(Backend {
            key: KeySource::Stored("sk-persisted".into()),
            ..Backend::preset(Provider::Anthropic)
        });
        config.add(Backend::preset(Provider::OpenAiCompatible));
        config.save_to(&path).expect("save");

        let loaded = AiConfig::load_from(&path);
        assert_eq!(loaded, config);
        assert_eq!(loaded.selected().map(|b| b.provider), Some(Provider::Anthropic));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_malformed_file_means_no_backends_rather_than_an_error() {
        let path = temp_path("malformed");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(AiConfig::load_from(&path), AiConfig::default());
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("permissions");
        let _ = std::fs::remove_file(&path);
        // A pre-existing world-readable file must be tightened, not left as it was.
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut config = AiConfig::default();
        config.add(Backend {
            key: KeySource::Stored("sk-secret".into()),
            ..Backend::preset(Provider::OpenAi)
        });
        config.save_to(&path).expect("save");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "ai.json holds API keys; it must be owner-only");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_config_location_can_be_overridden_for_tests() {
        let path = temp_path("override");
        std::env::set_var("BEARCAD_AI_CONFIG", &path);
        assert_eq!(config_path(), Some(path));
        std::env::remove_var("BEARCAD_AI_CONFIG");
    }

    #[test]
    fn the_config_file_sits_beside_settings_but_apart_from_it() {
        // The unoverridden location: tests run in one process, and a sibling test sets
        // BEARCAD_AI_CONFIG.
        let (Some(ai), Some(settings)) = (default_config_path(), crate::settings::settings_path())
        else {
            return; // No home directory on this platform; nothing to check.
        };
        assert_eq!(ai.parent(), settings.parent());
        assert_ne!(ai, settings, "keys do not belong in the settings file");
        assert_eq!(ai.file_name().unwrap(), "ai.json");
    }
}
