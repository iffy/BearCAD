//! What the AI is told about your documents (#1597).
//!
//! A document is described to the model as **the Lua script that recreates it**
//! ([`crate::export_lua`]). That is the same language the published agent skill teaches, so
//! the model reads the document in a notation it already understands and can answer in a
//! form the user can run.
//!
//! The scope switch decides how much goes: the active document only (the default), or every
//! document open in the app. Everything assembled here is shown to the user before it is
//! sent — [`Context::text`] is exactly what leaves the machine, not a summary of it.

use crate::model::Document;

/// How much of the workspace the conversation sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextScope {
    /// The document in the active tab. The default: the smallest thing that is useful.
    #[default]
    Document,
    /// Every document open in the app, across all tabs and windows.
    AllOpen,
}

impl ContextScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::AllOpen => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "This document",
            Self::AllOpen => "All open documents",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "document" | "current" | "this" | "active" => Some(Self::Document),
            "all" | "all_open" | "everything" | "workspace" => Some(Self::AllOpen),
            _ => None,
        }
    }
}

/// One open document, as the builder sees it. Borrowed — building a context never clones a
/// document.
pub struct DocumentInput<'a> {
    /// Tab title, e.g. "bracket" or "Untitled".
    pub title: String,
    /// The document in the active tab.
    pub active: bool,
    pub doc: &'a Document,
}

/// The assembled context, plus what the user needs to know about it before sending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Exactly what will be sent as the system prompt.
    pub text: String,
    /// How many documents it describes.
    pub documents: usize,
    /// Rough token count (see [`estimate_tokens`]).
    pub estimated_tokens: usize,
    /// Whether anything had to be cut to fit the budget.
    pub truncated: bool,
}

/// What the model is told it is doing. Kept short: the document itself is the context, and
/// the agent skill (#1602) is where the full API lives.
const PREAMBLE: &str = "\
You are helping with BearCAD, a parametric CAD app. Each open document is given below as \
the Lua script that recreates it, using BearCAD's scripting API — the same API the user can \
run. Lengths are millimetres and angles are radians unless a call says otherwise.

Answer questions about the geometry directly. When you propose a change, write it as a Lua \
snippet in a ```lua block that would apply to the document as it is now: the user runs \
blocks themselves, one click each, so a block should do one coherent thing and should not \
recreate the whole document unless asked.";

/// Roughly how many tokens a string costs. Four characters per token is the usual English
/// approximation; this only has to be good enough to warn about a large context and to
/// decide what to cut, never to bill anybody.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// How much context to send before cutting. Generous enough for several real documents,
/// small enough that a local model with a modest window still works.
pub const DEFAULT_TOKEN_BUDGET: usize = 40_000;

/// Build the context for `scope` from every open document.
///
/// The active document always comes first and is never the one cut: it is the document the
/// user is looking at, and the one their question is almost certainly about.
pub fn build(scope: ContextScope, docs: &[DocumentInput]) -> Context {
    build_with_budget(scope, docs, DEFAULT_TOKEN_BUDGET)
}

pub fn build_with_budget(
    scope: ContextScope,
    docs: &[DocumentInput],
    token_budget: usize,
) -> Context {
    // Active first, then the rest in tab order.
    let mut ordered: Vec<&DocumentInput> = Vec::new();
    ordered.extend(docs.iter().filter(|d| d.active));
    if scope == ContextScope::AllOpen {
        ordered.extend(docs.iter().filter(|d| !d.active));
    }

    let mut text = String::from(PREAMBLE);
    let mut truncated = false;
    let mut included = 0usize;
    let mut skipped = 0usize;

    for input in &ordered {
        let section = describe(input);
        let remaining = token_budget.saturating_sub(estimate_tokens(&text));
        if estimate_tokens(&section) <= remaining {
            text.push_str(&section);
            included += 1;
            continue;
        }
        // The first (active) document is always included, cut down rather than dropped.
        if included == 0 {
            text.push_str(&truncate_to_tokens(&section, remaining));
            text.push_str("\n-- (truncated to fit the context budget)\n```\n");
            truncated = true;
            included += 1;
            continue;
        }
        skipped += 1;
        truncated = true;
    }

    if skipped > 0 {
        text.push_str(&format!(
            "\n{skipped} more open document(s) were left out to fit the context budget.\n"
        ));
    }

    Context {
        estimated_tokens: estimate_tokens(&text),
        documents: included,
        truncated,
        text,
    }
}

/// One document's section: a heading, a one-line inventory, and the Lua that recreates it.
fn describe(input: &DocumentInput) -> String {
    let marker = if input.active { " (active — the user is looking at this one)" } else { "" };
    format!(
        "\n\n## Document: {}{}\n{}\n\n```lua\n{}```\n",
        input.title,
        marker,
        inventory(input.doc),
        crate::export_lua::document_to_lua(input.doc),
    )
}

/// A one-line count of what the document holds. The Lua below it is authoritative; this
/// just lets the model (and the user reading the preview) see the shape at a glance.
fn inventory(doc: &Document) -> String {
    let mut parts = vec![format!(
        "units: {} / {}",
        doc.default_length_unit.script_name(),
        doc.default_angle_unit.script_name()
    )];
    for (label, count) in [
        ("bodies", doc.bodies.len()),
        ("sketches", doc.sketches.len()),
        ("lines", doc.lines.len()),
        ("circles", doc.circles.len()),
        ("constraints", doc.constraints.len()),
        ("parameters", doc.parameters.len()),
    ] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if parts.len() == 1 {
        parts.push("empty".to_string());
    }
    parts.join(", ")
}

/// Cut `text` to roughly `tokens`, on a line boundary so the Lua stays readable.
fn truncate_to_tokens(text: &str, tokens: usize) -> String {
    let budget = tokens.saturating_mul(4);
    if text.len() <= budget {
        return text.to_string();
    }
    let mut out = String::with_capacity(budget);
    for line in text.lines() {
        if out.len() + line.len() + 1 > budget {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Action, AppState};

    /// A document with one rectangle, so the export has something in it.
    fn with_rect(width: f32, height: f32) -> Document {
        let mut state = AppState::default();
        let ground = state.doc.ground_plane().expect("ground plane");
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(ground),
            viewport: None,
        });
        state.apply(Action::CreateRectangle {
            x: 0.0,
            y: 0.0,
            width,
            height,
            width_expr: None,
            height_expr: None,
        });
        state.doc
    }

    fn inputs<'a>(docs: &[(&'a str, bool, &'a Document)]) -> Vec<DocumentInput<'a>> {
        docs.iter()
            .map(|(title, active, doc)| DocumentInput {
                title: (*title).to_string(),
                active: *active,
                doc: *doc,
            })
            .collect()
    }

    #[test]
    fn scope_names_round_trip() {
        for scope in [ContextScope::Document, ContextScope::AllOpen] {
            assert_eq!(ContextScope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(ContextScope::parse("current"), Some(ContextScope::Document));
        assert_eq!(ContextScope::parse("all_open"), Some(ContextScope::AllOpen));
        assert_eq!(ContextScope::parse("nonsense"), None);
        // The default is the smallest scope: one document.
        assert_eq!(ContextScope::default(), ContextScope::Document);
    }

    #[test]
    fn the_document_scope_sends_only_the_active_document() {
        let active = with_rect(80.0, 50.0);
        let other = with_rect(20.0, 20.0);
        let docs = inputs(&[("bracket", true, &active), ("plate", false, &other)]);

        let context = build(ContextScope::Document, &docs);
        assert_eq!(context.documents, 1);
        assert!(context.text.contains("## Document: bracket"));
        assert!(!context.text.contains("plate"), "other tabs stay out of it");
        assert!(!context.truncated);
        // The document really is described as its Lua export.
        assert!(context.text.contains("```lua"));
        assert!(context.text.contains("bearcad.rect"));
    }

    #[test]
    fn the_all_open_scope_sends_every_document_with_the_active_one_first() {
        let active = with_rect(80.0, 50.0);
        let other = with_rect(20.0, 20.0);
        // Deliberately out of order: the active tab is second.
        let docs = inputs(&[("plate", false, &other), ("bracket", true, &active)]);

        let context = build(ContextScope::AllOpen, &docs);
        assert_eq!(context.documents, 2);
        let bracket = context.text.find("## Document: bracket").expect("active document");
        let plate = context.text.find("## Document: plate").expect("other document");
        assert!(bracket < plate, "the active document leads");
        assert!(context.text.contains("(active"), "the model is told which one is in front");
    }

    #[test]
    fn each_document_carries_a_one_line_inventory() {
        let doc = with_rect(80.0, 50.0);
        let context = build(ContextScope::Document, &inputs(&[("bracket", true, &doc)]));
        assert!(context.text.contains("units: mm / deg"), "got: {}", context.text);
        assert!(context.text.contains("4 lines"), "a rectangle is four lines");
    }

    #[test]
    fn an_empty_document_says_so() {
        let doc = Document::default();
        let context = build(ContextScope::Document, &inputs(&[("Untitled", true, &doc)]));
        assert!(context.text.contains("empty"));
        assert_eq!(context.documents, 1);
    }

    #[test]
    fn a_budget_too_small_for_everything_keeps_the_active_document_and_says_it_cut() {
        let active = with_rect(80.0, 50.0);
        let other = with_rect(20.0, 20.0);
        let docs = inputs(&[("bracket", true, &active), ("plate", false, &other)]);

        // Room for the preamble and roughly one document.
        let budget = estimate_tokens(PREAMBLE) + 120;
        let context = build_with_budget(ContextScope::AllOpen, &docs, budget);
        assert_eq!(context.documents, 1, "the active document is never the one dropped");
        assert!(context.truncated);
        assert!(context.text.contains("## Document: bracket"));
        assert!(
            context.text.contains("1 more open document"),
            "the user is told something was left out: {}",
            context.text
        );
    }

    #[test]
    fn an_active_document_larger_than_the_whole_budget_is_cut_not_dropped() {
        let doc = with_rect(80.0, 50.0);
        let docs = inputs(&[("bracket", true, &doc)]);
        let context = build_with_budget(ContextScope::Document, &docs, estimate_tokens(PREAMBLE) + 10);
        assert_eq!(context.documents, 1);
        assert!(context.truncated);
        assert!(context.text.contains("truncated"), "a cut is always declared");
    }

    #[test]
    fn the_preamble_tells_the_model_the_units_and_how_to_propose_changes() {
        let doc = Document::default();
        let context = build(ContextScope::Document, &inputs(&[("Untitled", true, &doc)]));
        assert!(context.text.starts_with("You are helping with BearCAD"));
        assert!(context.text.contains("millimetres"));
        assert!(context.text.contains("```lua block"));
    }

    #[test]
    fn token_estimates_scale_with_length() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
