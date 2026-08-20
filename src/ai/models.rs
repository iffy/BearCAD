//! Asking a backend which models it has (#1617).
//!
//! Adding a backend does not ask for a model. Model names change faster than any list
//! shipped in a build could keep up with, and a gateway like OpenRouter carries hundreds —
//! so once a backend is connected, the app asks *it* what it can run and the answer fills
//! the **Model** dropdown.
//!
//! The request is a plain `GET`, and it is only ever made when the user asks for it: opening
//! the dropdown, pressing refresh, or finishing a connection. Nothing here runs at startup,
//! which keeps the promise that a fresh install is silent (#1609).
//!
//! Every provider BearCAD talks to answers with the same envelope — `{"data": [{"id": …}]}`
//! — so one parser covers all of them. OpenRouter adds per-model prices, which are picked up
//! too: a gateway model has no shipped rate, and reading the real one is better than showing
//! "price unknown" (#1599).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::backends::{Backend, Provider};
use super::pricing::Price;

/// How long to wait for the list. Short: this is a dropdown, not a reply.
const TIMEOUT: Duration = Duration::from_secs(20);

/// One model a backend says it can run.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// What goes in the request — the id the provider expects.
    pub id: String,
    /// What the dropdown shows. The provider's own display name when it gives one.
    pub label: String,
    /// The provider's published rate, when it publishes one with the list.
    pub price: Option<Price>,
}

/// A `GET` that asks a backend for its models. Keys ride in headers, never in the URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// Where to ask this backend, and with what.
pub fn list_request(backend: &Backend) -> ListRequest {
    let base = backend.effective_base_url();
    let key = backend.resolve_key();
    match backend.provider {
        Provider::Anthropic => ListRequest {
            // Anthropic's root has no version segment, and it pages — ask for the lot.
            url: format!("{base}/v1/models?limit=1000"),
            headers: key
                .map(|key| ("x-api-key".to_string(), key))
                .into_iter()
                .chain([(
                    "anthropic-version".to_string(),
                    super::providers::ANTHROPIC_VERSION.to_string(),
                )])
                .collect(),
        },
        // Everything else speaks OpenAI's shape, whose base URL already carries `/v1`.
        _ => ListRequest {
            url: format!("{base}/models"),
            headers: key
                .map(|key| ("authorization".to_string(), format!("Bearer {key}")))
                .into_iter()
                .collect(),
        },
    }
}

/// Turn a model-list response into models, newest-name-last.
pub fn parse_list(body: &str) -> Result<Vec<Model>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "the backend's model list was not JSON".to_string())?;
    // `data` everywhere; a bare array is accepted too, since a local gateway might send one.
    let entries = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(|d| d.as_array())
        .or_else(|| value.as_array())
        .ok_or_else(|| "the backend listed no models".to_string())?;

    let mut models: Vec<Model> = entries
        .iter()
        .filter_map(|entry| {
            let id = entry
                .get("id")
                .or_else(|| entry.get("name"))
                .and_then(|v| v.as_str())?
                .trim();
            if id.is_empty() {
                return None;
            }
            let label = entry
                .get("display_name")
                .or_else(|| entry.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id);
            Some(Model {
                id: id.to_string(),
                label: label.to_string(),
                price: parse_price(entry.get("pricing")),
            })
        })
        .collect();
    if models.is_empty() {
        return Err("the backend listed no models".to_string());
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

/// OpenRouter's per-model rates, in dollars **per token** as strings. The rest of the app
/// works per million, so scale on the way in.
fn parse_price(pricing: Option<&serde_json::Value>) -> Option<Price> {
    let pricing = pricing?;
    let rate = |name: &str| -> Option<f64> {
        pricing
            .get(name)
            .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()))
            .map(|per_token: f64| per_token * 1_000_000.0)
    };
    let input = rate("prompt").or_else(|| rate("input"))?;
    let output = rate("completion").or_else(|| rate("output"))?;
    Some(Price {
        input,
        output,
        // A provider that does not price its cache bills it as ordinary input.
        cache_read: rate("input_cache_read").unwrap_or(input),
        cache_write: rate("input_cache_write").unwrap_or(input),
    })
}

/// What the dropdown knows about one backend's models.
#[derive(Clone, Debug)]
pub enum Catalog {
    /// The request is in flight.
    Loading,
    Ready(Vec<Model>),
    Failed(String),
}

/// Shared with the thread doing the asking.
pub type SharedCatalog = Arc<Mutex<Catalog>>;

/// Ask `backend` for its models, on a background thread. Returns the slot the answer lands
/// in; `repaint` is the egui context to wake when it does.
pub fn fetch(backend: &Backend, repaint: Option<egui::Context>) -> SharedCatalog {
    let shared: SharedCatalog = Arc::new(Mutex::new(Catalog::Loading));
    let request = list_request(backend);
    let worker = Arc::clone(&shared);
    std::thread::spawn(move || {
        let outcome = get(&request);
        if let Ok(mut slot) = worker.lock() {
            *slot = match outcome {
                Ok(models) => Catalog::Ready(models),
                Err(message) => Catalog::Failed(message),
            };
        }
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    });
    shared
}

/// Perform the request and parse it. The one place this module touches a socket.
fn get(request: &ListRequest) -> Result<Vec<Model>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(TIMEOUT))
        .timeout_recv_response(Some(TIMEOUT))
        // The backend's own message on a 4xx is what the user needs to see.
        .http_status_as_error(false)
        .build()
        .into();
    let mut get = agent.get(&request.url);
    for (name, value) in &request.headers {
        get = get.header(name, value);
    }
    let mut response = get.call().map_err(|e| {
        let host = request
            .url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or(&request.url);
        format!("could not reach {host}: {e}")
    })?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(super::providers::error_message(status, &body));
    }
    parse_list(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::backends::{KeySource, Provider};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    /// A backend's model endpoint, faked on loopback: one request, one canned reply.
    struct FakeBackend {
        port: u16,
        handle: std::thread::JoinHandle<String>,
    }

    impl FakeBackend {
        fn replying(status: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("accept");
                let request = read_head(&socket);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
                let _ = socket.flush();
                request
            });
            Self { port, handle }
        }

        fn backend(&self) -> Backend {
            Backend {
                base_url: format!("http://127.0.0.1:{}/api/v1", self.port),
                key: KeySource::Stored("sk-or-test".into()),
                ..Backend::preset(Provider::OpenRouter)
            }
        }

        fn received(self) -> String {
            self.handle.join().expect("server thread")
        }
    }

    fn read_head(socket: &TcpStream) -> String {
        let mut reader = BufReader::new(socket);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let blank = line.trim().is_empty();
            head.push_str(&line);
            if blank {
                break;
            }
        }
        head
    }

    fn settled(shared: &SharedCatalog) -> Catalog {
        for _ in 0..500 {
            let catalog = shared.lock().unwrap().clone();
            if !matches!(catalog, Catalog::Loading) {
                return catalog;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the model list never arrived");
    }

    #[test]
    fn each_provider_is_asked_the_way_it_expects() {
        let anthropic = Backend {
            key: KeySource::Stored("sk-ant-test".into()),
            ..Backend::preset(Provider::Anthropic)
        };
        let request = list_request(&anthropic);
        assert_eq!(request.url, "https://api.anthropic.com/v1/models?limit=1000");
        assert!(request.headers.iter().any(|(n, v)| n == "x-api-key" && v == "sk-ant-test"));
        assert!(request.headers.iter().any(|(n, _)| n == "anthropic-version"));

        let router = Backend {
            key: KeySource::OAuth("sk-or-connected".into()),
            ..Backend::preset(Provider::OpenRouter)
        };
        let request = list_request(&router);
        assert_eq!(request.url, "https://openrouter.ai/api/v1/models");
        assert!(request
            .headers
            .iter()
            .any(|(n, v)| n == "authorization" && v == "Bearer sk-or-connected"));
        // A key never rides in a URL, where it would reach logs and history.
        assert!(!request.url.contains("sk-or-connected"));

        // A local server needs no key, and must still be askable.
        let local = Backend::preset(Provider::OpenAiCompatible);
        let request = list_request(&local);
        assert_eq!(request.url, "http://localhost:11434/v1/models");
        assert!(request.headers.is_empty());
    }

    #[test]
    fn every_providers_list_shape_parses() {
        // OpenAI / Ollama / xAI: ids only.
        let models = parse_list(r#"{"object":"list","data":[{"id":"gpt-5"},{"id":"gpt-5-mini"}]}"#)
            .expect("parse");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
        assert_eq!(models[0].label, "gpt-5", "no display name means the id is the label");
        assert!(models[0].price.is_none(), "no published rate, no invented one");

        // Anthropic: a display name worth showing instead of the id.
        let models = parse_list(
            r#"{"data":[{"id":"claude-opus-5","display_name":"Claude Opus 5","type":"model"}]}"#,
        )
        .expect("parse");
        assert_eq!(models[0].id, "claude-opus-5");
        assert_eq!(models[0].label, "Claude Opus 5");

        assert!(parse_list("not json").is_err());
        assert!(parse_list(r#"{"data":[]}"#).is_err(), "an empty list is not a catalog");
    }

    #[test]
    fn a_gateways_published_rates_come_back_with_its_models() {
        // #1599 shows tokens only when no rate is known. OpenRouter publishes one per model,
        // per token, so the dropdown can hand the real number over instead.
        let models = parse_list(
            r#"{"data":[{"id":"anthropic/claude-opus-5","name":"Anthropic: Claude Opus 5",
                 "pricing":{"prompt":"0.000005","completion":"0.000025",
                            "input_cache_read":"0.0000005"}},
               {"id":"meta-llama/llama-3.3-70b:free","name":"Llama 3.3 (free)",
                 "pricing":{"prompt":"0","completion":"0"}}]}"#,
        )
        .expect("parse");
        assert_eq!(models[0].id, "anthropic/claude-opus-5");
        assert_eq!(models[0].label, "Anthropic: Claude Opus 5");
        let price = models[0].price.expect("a published rate");
        assert!((price.input - 5.0).abs() < 1e-9, "per million, got {}", price.input);
        assert!((price.output - 25.0).abs() < 1e-9, "per million, got {}", price.output);
        assert!((price.cache_read - 0.5).abs() < 1e-9, "cache reads are cheaper");
        // Free is a price, not an absence of one.
        assert_eq!(models[1].price, Some(Price::new(0.0, 0.0)));
    }

    #[test]
    fn fetching_a_list_asks_the_backend_and_fills_the_catalog() {
        let server = FakeBackend::replying(
            "200 OK",
            r#"{"data":[{"id":"z-model"},{"id":"a-model"},{"id":"a-model"}]}"#,
        );
        let catalog = settled(&fetch(&server.backend(), None));
        match catalog {
            Catalog::Ready(models) => {
                let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
                assert_eq!(ids, ["a-model", "z-model"], "sorted, and never twice");
            }
            other => panic!("expected a list, got {other:?}"),
        }
        let request = server.received();
        assert!(request.starts_with("GET /api/v1/models"), "got: {request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer sk-or-test"));
    }

    #[test]
    fn a_backend_that_refuses_says_why_instead_of_showing_an_empty_dropdown() {
        let server = FakeBackend::replying(
            "401 Unauthorized",
            r#"{"error":{"message":"invalid credentials"}}"#,
        );
        match settled(&fetch(&server.backend(), None)) {
            Catalog::Failed(message) => {
                assert!(message.contains("invalid credentials"), "got {message}");
                assert!(!message.contains("sk-or-test"), "a failure must not carry the key");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        let _ = server.received();
    }

    #[test]
    fn an_unreachable_backend_fails_the_catalog_rather_than_the_app() {
        let backend = Backend {
            // Nothing listens on port 1.
            base_url: "http://127.0.0.1:1/v1".into(),
            ..Backend::preset(Provider::OpenAiCompatible)
        };
        match settled(&fetch(&backend, None)) {
            Catalog::Failed(message) => assert!(message.contains("127.0.0.1:1"), "got {message}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}
