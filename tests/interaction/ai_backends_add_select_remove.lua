-- Interaction regression (#1595): AI backends can be added, selected and removed from a
-- script, and a stored key is never readable back out of the app.
--
-- Run with BEARCAD_AI_CONFIG pointing at a throwaway file (CI does) — this test adds and
-- removes real backends, and the first assertion below fails rather than touching an
-- ai.json that already has some.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

assert(#bearcad.ai.backends() == 0, "a fresh config has no backends")
assert(bearcad.ai.backend() == nil, "nothing is selected until something is added")

bearcad.ai.add_backend{ provider = "claude", name = "Claude", model = "claude-opus-5",
                        key = "sk-secret-never-readable" }
bearcad.ai.add_backend{ provider = "ollama", name = "Local" }

local list = bearcad.ai.backends()
assert(#list == 2, "expected 2 backends, got " .. #list)
assert(list[1].provider == "anthropic", "first backend should be anthropic, got " .. list[1].provider)
assert(list[1].model == "claude-opus-5", "model should round-trip, got " .. list[1].model)
assert(list[1].selected, "the first backend added becomes the selected one")

-- The key is described, never handed back.
assert(list[1].key == "stored", "a pasted key reads back as 'stored', got " .. list[1].key)
for _, b in ipairs(list) do
  for field, value in pairs(b) do
    assert(type(value) ~= "string" or not value:find("sk%-secret"),
      "field " .. field .. " leaked the API key")
  end
end

-- A local backend needs no key at all, and is usable as-is.
assert(list[2].key == "none", "a local backend defaults to no key, got " .. list[2].key)
assert(list[2].usable, "a keyless local backend is usable")

bearcad.ai.set_backend(list[2].id)
assert(bearcad.ai.backend() == list[2].id, "set_backend switches the conversation's backend")

bearcad.ai.update_backend(list[2].id, { model = "llama3.3" })
assert(bearcad.ai.backends()[2].model == "llama3.3", "update_backend edits in place")
assert(bearcad.ai.backend() == list[2].id, "editing a backend keeps it selected")

-- Removing the selected backend moves the selection rather than leaving it dangling.
bearcad.ai.remove_backend(list[2].id)
assert(#bearcad.ai.backends() == 1, "remove_backend drops it")
assert(bearcad.ai.backend() == list[1].id, "selection falls back to what remains")

bearcad.ai.remove_backend(list[1].id)
assert(bearcad.ai.backend() == nil, "no backends left, nothing selected")

print("ok: AI backends add, select, edit and remove without ever exposing a key")
bearcad.quit()
