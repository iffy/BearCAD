-- Interaction regression (#1597/#1598): the chat pane assembles the document context,
-- sends it to the selected backend, and reports a failure on the reply instead of
-- swallowing it. Also #1614: this sequence must not log egui "changed id between
-- passes" on the Elements pane.
--
-- Run with BEARCAD_AI_CONFIG pointing at a throwaway file (CI does): this test adds a
-- backend, and the first assertion fails rather than touching a real ai.json.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

assert(#bearcad.ai.backends() == 0, "a fresh config has no backends")

-- Draw something, so the context has geometry to describe.
bearcad.rect{ width = 80, height = 50 }

-- The default scope is the one document in front of you.
assert(bearcad.ai.context_scope() == "document",
  "default scope should be 'document', got " .. bearcad.ai.context_scope())

local context = bearcad.ai.context_preview()
assert(context.documents == 1, "one open document, got " .. context.documents)
assert(context.text:find("## Document:"), "context should describe the document")
assert(context.text:find("bearcad.rect"), "the document is described as its Lua export")
assert(context.tokens > 0, "context should have an estimated token count")
assert(not context.truncated, "a small document is not truncated")

-- Two documents once a second tab is open, but only with the wider scope.
bearcad.ui.new_tab()
bearcad.ui.wait(1)
assert(bearcad.ai.context_preview().documents == 1, "'document' scope stays at one")
bearcad.ai.context_scope("all")
assert(bearcad.ai.context_scope() == "all", "scope switches")
assert(bearcad.ai.context_preview().documents == 2, "'all' scope covers both tabs")
bearcad.ai.context_scope("document")

-- Nothing can be sent before a backend exists.
local ok, err = pcall(function() return bearcad.ai.ask("hello?") end)
assert(not ok, "sending without a backend should fail")
assert(tostring(err):find("No AI backend"),
  "the reason should be said out loud, got: " .. tostring(err))

-- Point a backend at a port nothing listens on: the whole path runs, and the failure
-- lands on the reply rather than being swallowed.
bearcad.ai.add_backend{ provider = "local", name = "Offline",
                        base_url = "http://127.0.0.1:1", model = "test-model" }
ok, err = pcall(function() return bearcad.ai.ask("how wide is the rectangle?") end)
assert(not ok, "an unreachable backend should surface as an error")
assert(tostring(err):find("127.0.0.1:1"), "the error should name the host, got: " .. tostring(err))

local messages = bearcad.ai.messages()
assert(#messages >= 2, "both turns are recorded, got " .. #messages)
local last = messages[#messages]
assert(last.role == "assistant", "the last turn is the reply slot")
assert(last.error ~= nil, "the reply carries the failure")
assert(not last.streaming, "a failed reply is not still streaming")
assert(messages[#messages - 1].text == "how wide is the rectangle?", "the question is kept")

bearcad.ai.clear()
assert(#bearcad.ai.messages() == 0, "clear empties the conversation")

-- One more frame so a last-pass warning is counted before we read it (#1614).
bearcad.ui.wait(1)
local id_warnings = bearcad.ui.widget_id_warnings()
assert(id_warnings == 0,
  "Elements pane must not log 'changed id between passes' during this sequence, got "
    .. tostring(id_warnings))

print("ok: chat builds document context, sends it, and reports failures on the reply")
bearcad.quit()
