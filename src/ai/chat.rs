//! The conversation in the AI pane (#1598).
//!
//! **Session-only.** The conversation is not saved with the document and never reaches
//! disk: closing the app forgets it. A document is a thing you keep; a chat about it is
//! not, and a transcript that quietly persisted would be a surprise.
//!
//! Sending is a two-step dance because the context spans documents the action layer cannot
//! see: [`Conversation::begin`] records the user's message and marks the conversation as
//! waiting, and the frame loop — which *can* reach every open tab — builds the context and
//! starts the request ([`Conversation::start`]).

use super::context::{Context, ContextScope};
use super::providers::{ChatMessage, Role, Usage};
use super::transport::SharedExchange;

/// One message in the thread.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub role: Role,
    /// The message text. For an assistant entry this grows as the reply streams in.
    pub text: String,
    /// A failure instead of (or after) a reply.
    pub error: Option<String>,
    /// Tokens the exchange used, once the provider reports them.
    pub usage: Usage,
    /// Which backend answered — the picker can change mid-conversation.
    pub backend: String,
    /// True while this entry is still being written.
    pub streaming: bool,
}

impl Default for Role {
    fn default() -> Self {
        Role::User
    }
}

/// A runnable Lua block found in a reply (#1600).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlock {
    /// Which entry it came from.
    pub entry: usize,
    /// Its position within that entry, from zero.
    pub index: usize,
    /// The Lua itself, without the fences.
    pub source: String,
    /// Output or error from the last time the user ran it, shown under the block.
    pub outcome: Option<Result<String, String>>,
}

/// Every ```lua block in `text`, in order. A block still being streamed — one whose closing
/// fence has not arrived — is left out: half a script is not something to offer a Run button
/// for.
pub fn lua_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        match &mut current {
            Some(block) => {
                if trimmed == "```" {
                    blocks.push(std::mem::take(block));
                    current = None;
                } else {
                    block.push_str(line);
                    block.push('\n');
                }
            }
            None => {
                // ```lua, ```Lua, or a bare ``` — the model is not always consistent, and a
                // fenced block in a BearCAD answer is Lua unless it says otherwise.
                if let Some(tag) = trimmed.strip_prefix("```") {
                    let tag = tag.trim().to_ascii_lowercase();
                    if tag.is_empty() || tag == "lua" {
                        current = Some(String::new());
                    }
                }
            }
        }
    }
    blocks
}

/// The outcome of one [`Conversation::poll`].
#[derive(Debug, Default)]
pub struct PollResult {
    /// True while the reply is still arriving — the frame loop keeps repainting.
    pub running: bool,
    /// Set exactly once, on the poll where a reply finishes: which backend answered, and
    /// what it used. The caller bills it to that backend's running total (#1599).
    pub completed: Option<(String, Usage)>,
}

/// A request in flight.
pub struct Pending {
    /// Where the worker thread writes the reply.
    pub exchange: SharedExchange,
    /// Index of the assistant entry being filled in.
    pub entry: usize,
}

/// The whole conversation, plus the state the pane needs to draw it.
#[derive(Default)]
pub struct Conversation {
    pub entries: Vec<Entry>,
    /// How much of the workspace goes with each message.
    pub scope: ContextScope,
    /// The request in flight, if any.
    pub pending: Option<Pending>,
    /// Set by [`Self::begin`]; the frame loop builds the context and calls [`Self::start`].
    pub awaiting_context: bool,
    /// What the last message actually sent, kept so the pane can show it after the fact.
    pub last_context: Option<Context>,
    /// The pane's input box. Lives here so a script can type into it too.
    pub input: String,
    /// What each run of a code block produced, keyed by (entry, block index) (#1600).
    pub block_outcomes: std::collections::HashMap<(usize, usize), Result<String, String>>,
    /// A block the user clicked **Run** on, waiting for the frame loop to execute it. Only
    /// the frame loop can run Lua against the live app.
    pub pending_run: Option<(usize, usize, String)>,
}

impl std::fmt::Debug for Conversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conversation")
            .field("entries", &self.entries.len())
            .field("scope", &self.scope)
            .field("streaming", &self.is_streaming())
            .finish()
    }
}

impl Conversation {
    /// Whether a reply is arriving right now.
    pub fn is_streaming(&self) -> bool {
        self.pending.is_some()
    }

    /// Record the user's message and the empty assistant entry it will be answered into,
    /// then wait for the frame loop to supply the context. `backend` is the id that will
    /// answer, recorded on the assistant entry so a later switch does not rewrite history.
    pub fn begin(&mut self, text: String, backend: String) {
        self.entries.push(Entry {
            role: Role::User,
            text,
            backend: backend.clone(),
            ..Entry::default()
        });
        self.entries.push(Entry {
            role: Role::Assistant,
            backend,
            streaming: true,
            ..Entry::default()
        });
        self.awaiting_context = true;
    }

    /// Attach the running request to the waiting assistant entry.
    pub fn start(&mut self, exchange: SharedExchange, context: Context) {
        self.awaiting_context = false;
        self.last_context = Some(context);
        if let Some(entry) = self.entries.len().checked_sub(1) {
            self.pending = Some(Pending { exchange, entry });
        }
    }

    /// Abandon a send that never started — no backend, or the context could not be built.
    pub fn fail_pending(&mut self, message: String) {
        self.awaiting_context = false;
        if let Some(entry) = self.entries.last_mut() {
            if entry.role == Role::Assistant && entry.streaming {
                entry.streaming = false;
                entry.error = Some(message);
                return;
            }
        }
    }

    /// What one [`Conversation::poll`] found.
    ///
    /// `completed` is set on the single poll where a reply finishes, so the caller can bill
    /// it exactly once (#1599).
    pub fn poll(&mut self) -> PollResult {
        let Some(pending) = &self.pending else {
            return PollResult::default();
        };
        let entry_index = pending.entry;
        let (text, usage, finished, error) = {
            let Ok(exchange) = pending.exchange.lock() else {
                return PollResult::default();
            };
            (
                exchange.text.clone(),
                exchange.usage,
                exchange.finished,
                exchange.error.clone(),
            )
        };
        let mut backend = String::new();
        if let Some(entry) = self.entries.get_mut(entry_index) {
            entry.text = text;
            entry.usage = usage;
            backend = entry.backend.clone();
            if finished {
                entry.streaming = false;
                entry.error = error;
            }
        }
        if finished {
            self.pending = None;
            return PollResult {
                running: false,
                completed: Some((backend, usage)),
            };
        }
        PollResult { running: true, completed: None }
    }

    /// Stop a reply in progress. What arrived stays in the thread.
    pub fn cancel(&mut self) {
        let Some(pending) = &self.pending else { return };
        if let Ok(exchange) = pending.exchange.lock() {
            exchange.cancel();
        }
    }

    /// Start over. Cancels anything running first, so a stopped reply cannot land in the
    /// new conversation.
    pub fn clear(&mut self) {
        self.cancel();
        self.pending = None;
        self.awaiting_context = false;
        self.entries.clear();
        self.last_context = None;
    }

    /// The conversation as the provider wants it: every complete turn, oldest first. The
    /// entry currently streaming is left out — it is what we are asking for.
    pub fn wire_messages(&self) -> Vec<ChatMessage> {
        self.entries
            .iter()
            .filter(|e| !e.streaming && !e.text.trim().is_empty())
            .map(|e| ChatMessage { role: e.role, text: e.text.clone() })
            .collect()
    }

    /// Add a finished question-and-answer pair without contacting anything (#1600).
    ///
    /// The canned-conversation path: documentation screenshots and tests need a thread with
    /// content, and neither should need an API key to get one. Usage stays zero — nothing
    /// was spent, so nothing is billed.
    pub fn seed(&mut self, question: String, reply: String, backend: String) {
        self.entries.push(Entry {
            role: Role::User,
            text: question,
            backend: backend.clone(),
            ..Entry::default()
        });
        self.entries.push(Entry {
            role: Role::Assistant,
            text: reply,
            backend,
            ..Entry::default()
        });
    }

    /// The runnable blocks in the most recent finished reply (#1600).
    pub fn blocks(&self) -> Vec<CodeBlock> {
        let Some((entry_index, entry)) = self
            .entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| e.role == Role::Assistant && !e.streaming)
        else {
            return Vec::new();
        };
        lua_blocks(&entry.text)
            .into_iter()
            .enumerate()
            .map(|(index, source)| CodeBlock {
                entry: entry_index,
                index,
                source,
                outcome: self
                    .block_outcomes
                    .get(&(entry_index, index))
                    .cloned(),
            })
            .collect()
    }

    /// Record what running a block did, so the pane can show it under that block.
    pub fn record_block_outcome(&mut self, entry: usize, index: usize, outcome: Result<String, String>) {
        self.block_outcomes.insert((entry, index), outcome);
    }

    /// Totals across the conversation so far (#1599 builds the cost view on this).
    #[allow(dead_code)] // Shown by the cost readout (#1599); asserted in tests today.
    pub fn total_usage(&self) -> Usage {
        let mut total = Usage::default();
        for entry in &self.entries {
            total.input_tokens += entry.usage.input_tokens;
            total.output_tokens += entry.usage.output_tokens;
            total.cache_read_tokens += entry.usage.cache_read_tokens;
            total.cache_write_tokens += entry.usage.cache_write_tokens;
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::transport::Exchange;
    use std::sync::{Arc, Mutex};

    fn exchange() -> SharedExchange {
        Arc::new(Mutex::new(Exchange::default()))
    }

    #[test]
    fn beginning_a_message_adds_both_turns_and_waits_for_context() {
        let mut chat = Conversation::default();
        chat.begin("how wide?".into(), "claude".into());
        assert_eq!(chat.entries.len(), 2);
        assert_eq!(chat.entries[0].role, Role::User);
        assert_eq!(chat.entries[0].text, "how wide?");
        assert_eq!(chat.entries[1].role, Role::Assistant);
        assert!(chat.entries[1].streaming, "the reply slot is open");
        assert!(chat.awaiting_context, "the frame loop supplies the documents");
        assert!(!chat.is_streaming(), "nothing is on the wire yet");
    }

    #[test]
    fn polling_copies_the_streamed_reply_into_the_thread() {
        let mut chat = Conversation::default();
        chat.begin("how wide?".into(), "claude".into());
        let shared = exchange();
        chat.start(Arc::clone(&shared), Context::default());
        assert!(chat.is_streaming());

        shared.lock().unwrap().text.push_str("80 mm");
        assert!(chat.poll().running, "still running");
        assert_eq!(chat.entries[1].text, "80 mm");
        assert!(chat.entries[1].streaming);

        {
            let mut exchange = shared.lock().unwrap();
            exchange.text.push_str(" wide.");
            exchange.usage = Usage { input_tokens: 100, output_tokens: 5, ..Usage::default() };
            exchange.finished = true;
        }
        let result = chat.poll();
        assert!(!result.running, "finished");
        let (backend, billed) = result.completed.expect("a finished reply is billed once");
        assert_eq!(backend, "claude");
        assert_eq!(billed.output_tokens, 5);
        assert!(chat.poll().completed.is_none(), "and only once");
        assert_eq!(chat.entries[1].text, "80 mm wide.");
        assert!(!chat.entries[1].streaming);
        assert_eq!(chat.entries[1].usage.output_tokens, 5);
        assert!(!chat.is_streaming());
    }

    #[test]
    fn a_failed_request_shows_as_an_error_on_the_reply() {
        let mut chat = Conversation::default();
        chat.begin("hi".into(), "claude".into());
        let shared = exchange();
        chat.start(Arc::clone(&shared), Context::default());
        {
            let mut exchange = shared.lock().unwrap();
            exchange.error = Some("401: invalid key".into());
            exchange.finished = true;
        }
        chat.poll();
        assert_eq!(chat.entries[1].error.as_deref(), Some("401: invalid key"));
        assert!(!chat.entries[1].streaming);
    }

    #[test]
    fn a_send_that_never_starts_reports_on_the_reply_slot() {
        let mut chat = Conversation::default();
        chat.begin("hi".into(), "claude".into());
        chat.fail_pending("no backend selected".into());
        assert!(!chat.awaiting_context);
        assert_eq!(chat.entries[1].error.as_deref(), Some("no backend selected"));
        assert!(!chat.entries[1].streaming);
    }

    #[test]
    fn the_wire_history_leaves_out_the_reply_being_written() {
        let mut chat = Conversation::default();
        chat.entries.push(Entry {
            role: Role::User,
            text: "first".into(),
            ..Entry::default()
        });
        chat.entries.push(Entry {
            role: Role::Assistant,
            text: "answer".into(),
            ..Entry::default()
        });
        chat.begin("second".into(), "claude".into());

        let wire = chat.wire_messages();
        assert_eq!(wire.len(), 3, "two finished turns plus the new question");
        assert_eq!(wire[0].text, "first");
        assert_eq!(wire[1].role, Role::Assistant);
        assert_eq!(wire[2].text, "second");
    }

    #[test]
    fn clearing_stops_a_running_reply_and_empties_the_thread() {
        let mut chat = Conversation::default();
        chat.begin("hi".into(), "claude".into());
        let shared = exchange();
        chat.start(Arc::clone(&shared), Context::default());
        chat.clear();
        assert!(shared.lock().unwrap().is_cancelled(), "the worker is told to stop");
        assert!(chat.entries.is_empty());
        assert!(!chat.is_streaming());
        assert!(chat.last_context.is_none());
    }

    #[test]
    fn lua_blocks_are_found_in_a_reply_and_half_written_ones_are_not() {
        let reply = "Sure — widen it:\n\n```lua\nbearcad.rect{ width = 100 }\n```\n\nThen:\n\n```lua\nbearcad.extrude{ distance = 10 }\n```\n";
        let blocks = lua_blocks(reply);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], "bearcad.rect{ width = 100 }\n");
        assert_eq!(blocks[1], "bearcad.extrude{ distance = 10 }\n");

        // A block whose closing fence has not streamed in yet is not offered.
        assert!(lua_blocks("here you go:\n```lua\nbearcad.rect{ wid").is_empty());
        // Fenced blocks of another language are left alone.
        assert!(lua_blocks("```python\nprint(1)\n```").is_empty());
        // An untagged fence in a BearCAD answer is Lua.
        assert_eq!(lua_blocks("```\nbearcad.new()\n```").len(), 1);
    }

    #[test]
    fn blocks_come_from_the_latest_finished_reply_only() {
        let mut chat = Conversation::default();
        chat.entries.push(Entry {
            role: Role::Assistant,
            text: "old\n```lua\nbearcad.new()\n```".into(),
            ..Entry::default()
        });
        chat.entries.push(Entry {
            role: Role::User,
            text: "and now?".into(),
            ..Entry::default()
        });
        chat.entries.push(Entry {
            role: Role::Assistant,
            text: "new\n```lua\nbearcad.rect{ width = 5 }\n```".into(),
            ..Entry::default()
        });

        let blocks = chat.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].source.trim(), "bearcad.rect{ width = 5 }");
        assert_eq!(blocks[0].entry, 2);
        assert!(blocks[0].outcome.is_none(), "not run yet");

        // While a new reply streams in, the buttons stay on the last *finished* reply
        // rather than disappearing — a half-written block is never offered, but the
        // previous suggestion is still the one on screen.
        chat.entries.last_mut().unwrap().streaming = true;
        let blocks = chat.blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].entry, 0, "falls back to the earlier finished reply");
    }

    #[test]
    fn running_a_block_records_its_outcome_against_that_block() {
        let mut chat = Conversation::default();
        chat.entries.push(Entry {
            role: Role::Assistant,
            text: "```lua\nbearcad.rect{ width = 5 }\n```\n```lua\nbroken(\n```".into(),
            ..Entry::default()
        });
        chat.record_block_outcome(0, 1, Err("syntax error".into()));
        let blocks = chat.blocks();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].outcome.is_none(), "the first block was never run");
        assert_eq!(blocks[1].outcome, Some(Err("syntax error".into())));
    }

    #[test]
    fn conversation_usage_adds_up_across_replies() {
        let mut chat = Conversation::default();
        for tokens in [10u64, 25] {
            chat.entries.push(Entry {
                role: Role::Assistant,
                usage: Usage { input_tokens: tokens * 10, output_tokens: tokens, ..Usage::default() },
                ..Entry::default()
            });
        }
        let total = chat.total_usage();
        assert_eq!(total.input_tokens, 350);
        assert_eq!(total.output_tokens, 35);
    }
}
