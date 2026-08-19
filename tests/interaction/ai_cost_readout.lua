-- Interaction regression (#1599): the app tracks what the chat costs — per conversation and
-- per backend, all-time — and never invents a price for a model it does not know.
--
-- Run with BEARCAD_AI_CONFIG pointing at a throwaway file (CI does).
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

assert(#bearcad.ai.backends() == 0, "a fresh config has no backends")

-- Nothing has been spent yet.
local usage = bearcad.ai.usage()
assert(usage.conversation.tokens == 0, "a new conversation has cost nothing")

-- A local server is free whatever model it runs, so its cost is a number, not nil.
bearcad.ai.add_backend{ provider = "local", name = "Local", base_url = "http://127.0.0.1:1",
                        model = "llama3.2" }
assert(bearcad.ai.usage().conversation.cost == 0, "a local backend costs nothing")

-- A model this build has no rate for reports tokens but no cost — never a guess.
bearcad.ai.add_backend{ provider = "openai", name = "Unknown model", model = "not-a-real-model",
                        key = "sk-test" }
bearcad.ai.set_backend("unknown-model")
assert(bearcad.ai.usage().conversation.cost == nil,
  "an unknown model must report no price rather than inventing one")

-- A model it does know prices at the shipped rate...
bearcad.ai.add_backend{ provider = "anthropic", name = "Known", model = "claude-opus-5",
                        key = "sk-test" }
bearcad.ai.set_backend("known")
assert(bearcad.ai.usage().conversation.cost == 0, "no tokens used yet, but the rate is known")

-- ...and a backend can state its own rate when the published one moves.
bearcad.ai.update_backend("known", { input_price = 2.0, output_price = 8.0 })

-- Every backend carries an all-time total, starting empty.
local backends = bearcad.ai.usage().backends
assert(backends["known"].tokens == 0, "nothing spent yet")
assert(backends["known"].exchanges == 0, "no replies yet")

-- Resetting is scriptable (the pane's Reset button does the same thing).
bearcad.ai.reset_usage("known")
assert(bearcad.status():find("Reset"), "reset should report itself, got: " .. bearcad.status())

print("ok: costs are tracked per conversation and per backend, and never invented")
bearcad.quit()
