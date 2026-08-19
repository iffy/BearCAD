//! What a conversation costs (#1599).
//!
//! Providers report tokens, not money. This module turns one into the other, and is honest
//! about when it cannot: an unknown model shows **tokens only**, never an invented price.
//!
//! Published prices change faster than BearCAD releases, so the shipped table is a
//! convenience, not a source of truth — every backend can carry its own rates, and those
//! win. The numbers below are per **million** tokens, in US dollars.

use serde::{Deserialize, Serialize};

use super::backends::{Backend, Provider};
use super::providers::Usage;

/// Per-million-token rates for one model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Price {
    /// Ordinary input tokens, per million.
    pub input: f64,
    /// Output tokens, per million.
    pub output: f64,
    /// Cache reads, per million. Cheaper than input everywhere that offers them.
    #[serde(default)]
    pub cache_read: f64,
    /// Cache writes, per million. Dearer than input.
    #[serde(default)]
    pub cache_write: f64,
}

impl Price {
    /// A price with only the two rates every provider publishes. Cache rates default to the
    /// input rate, which is what an unreported cache costs in practice.
    pub const fn new(input: f64, output: f64) -> Self {
        Self { input, output, cache_read: input, cache_write: input }
    }

    /// The same, with Anthropic's published cache multipliers: reads are a tenth of input,
    /// writes a quarter more than input.
    pub fn with_anthropic_cache(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            cache_read: input * 0.1,
            cache_write: input * 1.25,
        }
    }

    /// What `usage` costs at these rates, in dollars.
    pub fn cost(&self, usage: Usage) -> f64 {
        let per_million = |tokens: u64, rate: f64| tokens as f64 * rate / 1_000_000.0;
        per_million(usage.input_tokens, self.input)
            + per_million(usage.output_tokens, self.output)
            + per_million(usage.cache_read_tokens, self.cache_read)
            + per_million(usage.cache_write_tokens, self.cache_write)
    }

    /// A price of zero all round — a local model costs nothing to run a token through.
    pub fn free() -> Self {
        Self::default()
    }
}

/// Rates shipped with this build, keyed by the model id a backend names.
///
/// Matching is by prefix, so a dated or suffixed variant of a listed model still finds its
/// rate. Longest match wins, so a more specific entry beats a general one.
const TABLE: &[(&str, f64, f64)] = &[
    // Anthropic (per million, input/output).
    ("claude-opus-5", 5.0, 25.0),
    ("claude-opus-4-8", 5.0, 25.0),
    ("claude-opus-4-7", 5.0, 25.0),
    ("claude-opus-4-6", 5.0, 25.0),
    ("claude-fable-5", 10.0, 50.0),
    ("claude-sonnet-5", 3.0, 15.0),
    ("claude-sonnet-4-6", 3.0, 15.0),
    ("claude-haiku-4-5", 1.0, 5.0),
];

/// The shipped rate for `model`, if this build knows one.
pub fn shipped_price(provider: Provider, model: &str) -> Option<Price> {
    // A local server bills nothing; its "model" name is whatever the user pulled.
    if provider == Provider::OpenAiCompatible {
        return Some(Price::free());
    }
    let model = model.trim().to_ascii_lowercase();
    let best = TABLE
        .iter()
        .filter(|(id, _, _)| model.starts_with(id))
        .max_by_key(|(id, _, _)| id.len())?;
    Some(match provider {
        Provider::Anthropic => Price::with_anthropic_cache(best.1, best.2),
        _ => Price::new(best.1, best.2),
    })
}

/// The rate to charge `backend` at: its own override, else the shipped table, else nothing
/// — in which case the UI shows tokens and says the price is unknown.
pub fn price_for(backend: &Backend) -> Option<Price> {
    backend
        .price
        .or_else(|| shipped_price(backend.provider, &backend.model))
}

/// Running totals for one backend, kept across restarts so "what has this cost me?" has an
/// answer that outlives the conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Dollars, accumulated at the rate in force when each reply arrived — so editing a
    /// price later does not silently rewrite history.
    #[serde(default)]
    pub cost: f64,
    /// Replies counted.
    #[serde(default)]
    pub exchanges: u64,
}

impl Spend {
    /// Fold in one finished exchange.
    pub fn add(&mut self, usage: Usage, price: Option<Price>) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.cache_read_tokens += usage.cache_read_tokens;
        self.cache_write_tokens += usage.cache_write_tokens;
        self.exchanges += 1;
        if let Some(price) = price {
            self.cost += price.cost(usage);
        }
    }

    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn is_empty(&self) -> bool {
        self.exchanges == 0 && self.tokens() == 0
    }
}

/// A dollar amount, at a sensible precision for the size. Small spends are the normal case
/// and rounding them to cents would show "$0.00" for every message.
pub fn format_cost(dollars: f64) -> String {
    if dollars <= 0.0 {
        return "$0".to_string();
    }
    if dollars < 0.01 {
        format!("${dollars:.4}")
    } else if dollars < 1.0 {
        format!("${dollars:.3}")
    } else {
        format!("${dollars:.2}")
    }
}

/// A token count, abbreviated once it gets long: `842`, `1.3k`, `2.4M`.
pub fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..=999 => tokens.to_string(),
        1_000..=999_999 => format!("{:.1}k", tokens as f64 / 1_000.0),
        _ => format!("{:.1}M", tokens as f64 / 1_000_000.0),
    }
}

/// One exchange's line in the pane: tokens always, cost when the rate is known.
pub fn usage_line(usage: Usage, price: Option<Price>) -> String {
    if usage.input_tokens == 0 && usage.output_tokens == 0 {
        return String::new();
    }
    let tokens = format!(
        "{} in / {} out",
        format_tokens(usage.input_tokens),
        format_tokens(usage.output_tokens)
    );
    let cached = if usage.cache_read_tokens > 0 {
        format!(" ({} cached)", format_tokens(usage.cache_read_tokens))
    } else {
        String::new()
    };
    match price {
        Some(price) if price != Price::free() => {
            format!("{tokens}{cached} · {}", format_cost(price.cost(usage)))
        }
        Some(_) => format!("{tokens}{cached} · free"),
        None => format!("{tokens}{cached} · price unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::backends::Backend;

    fn usage(input: u64, output: u64) -> Usage {
        Usage { input_tokens: input, output_tokens: output, ..Usage::default() }
    }

    #[test]
    fn a_known_model_prices_at_its_published_rate() {
        let price = shipped_price(Provider::Anthropic, "claude-opus-5").expect("a known model");
        assert_eq!(price.input, 5.0);
        assert_eq!(price.output, 25.0);
        // A million in and a million out is $5 + $25.
        assert!((price.cost(usage(1_000_000, 1_000_000)) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn a_dated_or_suffixed_model_id_still_finds_its_rate() {
        let price = shipped_price(Provider::Anthropic, "claude-haiku-4-5-20251001")
            .expect("a dated variant matches its family");
        assert_eq!(price.input, 1.0);
        assert_eq!(price.output, 5.0);
    }

    #[test]
    fn an_unknown_model_has_no_price_at_all() {
        // Never guess: a made-up rate would be worse than saying nothing.
        assert!(shipped_price(Provider::OpenAi, "some-model-we-have-never-heard-of").is_none());
        assert_eq!(
            usage_line(usage(1000, 100), None),
            "1.0k in / 100 out · price unknown"
        );
    }

    #[test]
    fn a_local_backend_is_free_whatever_it_is_running() {
        let price = shipped_price(Provider::OpenAiCompatible, "llama3.2").expect("local is free");
        assert_eq!(price, Price::free());
        assert_eq!(price.cost(usage(10_000_000, 10_000_000)), 0.0);
        assert_eq!(usage_line(usage(1000, 100), Some(price)), "1.0k in / 100 out · free");
    }

    #[test]
    fn a_backend_price_override_beats_the_shipped_table() {
        let mut backend = Backend::preset(Provider::Anthropic);
        assert_eq!(price_for(&backend).map(|p| p.input), Some(5.0));
        // Published prices move; the user can say what they are actually paying.
        backend.price = Some(Price::new(2.0, 8.0));
        let price = price_for(&backend).expect("the override");
        assert_eq!(price.input, 2.0);
        assert_eq!(price.output, 8.0);
    }

    #[test]
    fn cached_tokens_are_charged_at_their_own_rate() {
        let price = shipped_price(Provider::Anthropic, "claude-opus-5").unwrap();
        assert!(price.cache_read < price.input, "cache reads are cheaper than input");
        assert!(price.cache_write > price.input, "cache writes cost more than input");
        let cached = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,
            cache_write_tokens: 0,
        };
        // A tenth of the input rate, not the input rate itself.
        assert!((price.cost(cached) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn spend_accumulates_across_replies_and_survives_a_price_change() {
        let mut spend = Spend::default();
        let price = Price::new(10.0, 20.0);
        spend.add(usage(100_000, 10_000), Some(price));
        assert_eq!(spend.exchanges, 1);
        assert!((spend.cost - (1.0 + 0.2)).abs() < 1e-9);

        // A later, different rate adds to the same total rather than restating it: what was
        // already spent was spent at the old rate.
        spend.add(usage(100_000, 10_000), Some(Price::new(1.0, 2.0)));
        assert_eq!(spend.exchanges, 2);
        assert!((spend.cost - (1.2 + 0.12)).abs() < 1e-9);
        assert_eq!(spend.tokens(), 220_000);
    }

    #[test]
    fn spend_still_counts_tokens_when_no_price_is_known() {
        let mut spend = Spend::default();
        spend.add(usage(500, 50), None);
        assert_eq!(spend.tokens(), 550);
        assert_eq!(spend.cost, 0.0, "an unknown price adds no money, only tokens");
        assert!(!spend.is_empty());
    }

    #[test]
    fn amounts_read_sensibly_at_every_size() {
        assert_eq!(format_cost(0.0), "$0");
        assert_eq!(format_cost(0.00021), "$0.0002");
        assert_eq!(format_cost(0.0342), "$0.034");
        assert_eq!(format_cost(12.3456), "$12.35");
        assert_eq!(format_tokens(842), "842");
        assert_eq!(format_tokens(1_300), "1.3k");
        assert_eq!(format_tokens(2_400_000), "2.4M");
    }

    #[test]
    fn an_exchange_with_no_reported_usage_shows_nothing() {
        assert_eq!(usage_line(Usage::default(), None), "");
    }
}
