//! Everything AI, split across two panes (#1593/#1620): an **AI** configuration pane and
//! an **AI Chat** pane.
//!
//! The configuration pane has two sections (#1621) — **Use AI inside BearCAD** (the chat
//! backends BearCAD talks to) and **Have AI use BearCAD** (the agent skill for outside
//! agents, and the MCP server that lets one drive the open document). The conversation
//! itself lives in the **AI Chat** bottom pane.
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
#[cfg(not(target_arch = "wasm32"))]
pub mod models;
#[cfg(not(target_arch = "wasm32"))]
pub mod oauth;
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
    /// Set when something asks for the "Have AI use BearCAD" section specifically (Help ▸
    /// Install AI Agent Skill…), so the pane opens showing it rather than the chat
    /// backend configuration (#1604).
    pub open_skill_section: bool,
    /// Set for one frame when something asks every section to open (or collapse) at once —
    /// `bearcad.ui.ai_sections(...)` (#1619). The headers remember it from there.
    pub sections_open: Option<bool>,
    /// The running MCP server (#1605), or `None` — which is the default, and the state a
    /// fresh install stays in until the user switches it on.
    #[cfg(not(target_arch = "wasm32"))]
    pub mcp: Option<mcp::Server>,
    /// A connection attempt in progress (#1624). At most one: the browser can only be
    /// pointed at one permission page at a time.
    #[cfg(not(target_arch = "wasm32"))]
    pub connect: Option<oauth::Flow>,
    /// What each backend answered when asked for its models (#1617), by backend id. Only
    /// ever filled by an explicit request — nothing is asked at startup.
    #[cfg(not(target_arch = "wasm32"))]
    pub models: std::collections::HashMap<String, models::SharedCatalog>,
}

#[cfg(not(target_arch = "wasm32"))]
impl AiState {
    /// Finish a connection attempt that has landed (#1624).
    ///
    /// Called once a frame. The flow runs on its own thread; storing the key it produced is
    /// done here, on the thread that owns the configuration. Returns a line for the status
    /// bar when something happened.
    ///
    /// A successful connection asks the backend for its models straight away (#1617) — the
    /// user has just said yes, and choosing a model is the only step left.
    pub fn poll_connect(&mut self, repaint: Option<egui::Context>) -> Option<String> {
        let flow = self.connect.as_ref()?;
        let id = flow.backend.clone();
        match flow.state() {
            oauth::Connect::Waiting => None,
            oauth::Connect::Connected(key) => {
                self.connect = None;
                let Some(backend) = self.config.get_mut(&id) else {
                    // The backend was removed while the browser was open.
                    return Some("Connected, but that backend is gone".to_string());
                };
                backend.key = backends::KeySource::OAuth(key);
                let name = backend.name.clone();
                let backend = backend.clone();
                self.config_dirty = true;
                self.models.insert(id, models::fetch(&backend, repaint));
                Some(format!("Connected to {name} — choose a model"))
            }
            oauth::Connect::Failed(message) => {
                self.connect = None;
                Some(format!("Could not connect: {message}"))
            }
        }
    }
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
        assert!(!state.panes.is_visible(crate::actions::Pane::Ai), "the config pane is closed too");
        assert!(
            !state.panes.is_visible(crate::actions::Pane::AiChat),
            "so is the AI Chat pane (#1620)"
        );
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn nothing_about_the_ai_is_reachable_from_a_script() {
        // #1616: a script off the internet must not be able to talk to a backend, spend a
        // key, start an MCP server or install an agent skill. `registered_names` walks the
        // live Lua API, so nothing under an AI name may show up in it.
        let names = crate::ai::api::registered_names();
        assert!(names.len() > 50, "the API walker found nothing: {names:?}");
        let reachable: Vec<_> = names
            .iter()
            .filter(|n| n.starts_with("bearcad.ai.") || n.starts_with("bearcad.ai_"))
            .collect();
        assert!(reachable.is_empty(), "scripts can still reach {reachable:?}");
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod setup_tests {
    //! Setting a backend up (#1617, #1624): add it, connect it, then pick a model — in that
    //! order, and without a live service or a real credential anywhere in these tests.

    use super::*;
    use crate::actions::{Action, ActionResult, AppState};
    use backends::{Backend, KeySource, Provider};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    /// A stand-in for a provider's token endpoint, on loopback. One request, one reply.
    fn fake_provider(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(&socket);
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut discard = vec![0u8; length];
            let _ = reader.read_exact(&mut discard);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    /// Play the browser: fetch the callback the flow is waiting on.
    fn visit(url: &str) {
        let rest = url.split("://").nth(1).expect("an http url");
        let (host, path) = rest.split_once('/').expect("a path");
        let mut stream = TcpStream::connect(host).expect("connect to the callback");
        write!(stream, "GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n").expect("send");
        let mut page = String::new();
        let _ = stream.read_to_string(&mut page);
    }

    fn callback_of(authorize_url: &str) -> String {
        let query = authorize_url.split_once('?').expect("a query").1;
        let escaped = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("callback_url="))
            .expect("a callback");
        escaped.replace("%3A", ":").replace("%2F", "/")
    }

    #[test]
    fn a_backend_without_a_model_will_not_send_until_one_is_chosen() {
        // #1617: a backend is added before a model is known, so "which model?" has to be a
        // refusal with an answer in it, not a request that quietly names nothing.
        let mut state = AppState::default();
        state.apply(Action::AddAiBackend {
            backend: Backend {
                key: KeySource::Stored("sk-or-test".into()),
                ..Backend::preset(Provider::OpenRouter)
            },
        });
        match state.apply(Action::SendAiMessage { text: "how wide?".into() }) {
            ActionResult::Err(message) => {
                assert!(message.contains("no model chosen"), "got {message}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        // Choosing one from the dropdown is all that was missing.
        state.apply(Action::SetAiBackendModel {
            id: "openrouter".into(),
            model: "anthropic/claude-opus-5".into(),
        });
        assert!(state.ai.borrow().config.selected().unwrap().unusable_reason().is_none());
        assert!(state.ai.borrow().config_dirty, "the choice is persisted");
    }

    #[test]
    fn choosing_a_model_from_the_backends_own_list_takes_its_price_with_it() {
        // #1599 cannot ship rates for a gateway's hundreds of models; the gateway publishes
        // them with the list, so the choice carries the real one (#1617).
        let mut state = AppState::default();
        state.apply(Action::AddAiBackend {
            backend: Backend {
                key: KeySource::OAuth("sk-or-connected".into()),
                ..Backend::preset(Provider::OpenRouter)
            },
        });
        let catalog = std::sync::Arc::new(std::sync::Mutex::new(models::Catalog::Ready(vec![
            models::Model {
                id: "anthropic/claude-opus-5".into(),
                label: "Anthropic: Claude Opus 5".into(),
                price: Some(pricing::Price::new(5.0, 25.0)),
            },
        ])));
        state.ai.borrow_mut().models.insert("openrouter".into(), catalog);

        state.apply(Action::SetAiBackendModel {
            id: "openrouter".into(),
            model: "anthropic/claude-opus-5".into(),
        });
        let ai = state.ai.borrow();
        let backend = ai.config.get("openrouter").unwrap();
        assert_eq!(backend.model, "anthropic/claude-opus-5");
        assert_eq!(backend.price, Some(pricing::Price::new(5.0, 25.0)));
        assert_eq!(pricing::price_for(backend), backend.price);
    }

    #[test]
    fn a_backend_that_cannot_answer_yet_is_not_asked_for_its_models() {
        // Nothing reaches the network before there is a credential to reach it with.
        let mut state = AppState::default();
        state.apply(Action::AddAiBackend {
            backend: Backend {
                key: KeySource::OAuth(String::new()),
                ..Backend::preset(Provider::OpenRouter)
            },
        });
        match state.apply(Action::RefreshAiModels { id: "openrouter".into() }) {
            ActionResult::Err(message) => {
                assert!(message.contains("not connected"), "got {message}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(state.ai.borrow().models.is_empty(), "nothing was asked");
        assert!(matches!(
            state.apply(Action::RefreshAiModels { id: "nope".into() }),
            ActionResult::Err(_)
        ));
    }

    #[test]
    fn only_a_provider_with_a_pkce_flow_can_be_connected() {
        // #1624: everywhere else still takes a pasted key, and says so.
        let mut state = AppState::default();
        state.apply(Action::AddAiBackend { backend: Backend::preset(Provider::Anthropic) });
        match state.apply(Action::ConnectAiBackend { id: "anthropic".into() }) {
            ActionResult::Err(message) => {
                assert!(message.contains("paste an API key"), "got {message}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(state.ai.borrow().connect.is_none());
        assert!(
            matches!(state.apply(Action::CancelAiConnect), ActionResult::Err(_)),
            "nothing to cancel"
        );
    }

    #[test]
    fn connecting_stores_a_key_the_user_never_saw_and_asks_what_models_there_are() {
        // The whole of #1624 end to end, against a local stand-in for the provider: press
        // Connect, approve in the "browser", and the backend is usable without a key ever
        // being typed, shown, or written into a script line.
        const ISSUED: &str = "sk-or-v1-issued-to-bearcad";
        let (base_url, provider) = fake_provider(r#"{"key":"sk-or-v1-issued-to-bearcad"}"#);
        let mut state = AppState::default();
        state.apply(Action::AddAiBackend {
            backend: Backend {
                base_url: base_url.clone(),
                key: KeySource::OAuth(String::new()),
                ..Backend::preset(Provider::OpenRouter)
            },
        });
        assert_eq!(
            state.ai.borrow().config.selected().unwrap().key_description(),
            "not connected"
        );

        assert!(matches!(
            state.apply(Action::ConnectAiBackend { id: "openrouter".into() }),
            ActionResult::Ok
        ));
        let authorize_url = {
            let ai = state.ai.borrow();
            let flow = ai.connect.as_ref().expect("a flow is running");
            assert_eq!(flow.backend, "openrouter");
            assert!(!flow.opened, "the frame loop opens the browser, not the action");
            flow.authorize_url.clone()
        };
        assert!(authorize_url.starts_with("https://openrouter.ai/auth?"), "got {authorize_url}");

        visit(&format!("{}?code=APPROVED", callback_of(&authorize_url)));

        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = state.ai.borrow_mut().poll_connect(None) {
                break status;
            }
            assert!(Instant::now() < deadline, "the connection never landed");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.contains("choose a model"), "got {status}");

        let ai = state.ai.borrow();
        assert!(ai.connect.is_none(), "the attempt is over");
        let backend = ai.config.get("openrouter").expect("the backend");
        assert_eq!(backend.key, KeySource::OAuth(ISSUED.to_string()));
        assert_eq!(backend.key_description(), "connected");
        assert!(ai.config_dirty, "the key is written to ai.json");
        assert!(ai.models.contains_key("openrouter"), "its models are asked for straight away");

        // The issued key is a credential like any other: it never reaches a log line or a
        // debug dump.
        assert!(!format!("{ai:?}").contains(ISSUED));
        assert!(!status.contains(ISSUED));
        drop(ai);
        let _ = provider.join();
    }
}
