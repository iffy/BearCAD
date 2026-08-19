-- Interaction regression (#1600): Lua the AI suggests runs only when asked, applies to the
-- active document, is undoable, and reports a broken block instead of swallowing it.
--
-- Run with BEARCAD_AI_CONFIG pointing at a throwaway file (CI does).
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")

-- A canned reply: no backend, no network, no key.
bearcad.ai.seed_reply("make it wider", [[
Here you go:

```lua
bearcad.rect{ width = 120, height = 40 }
```

And one that will not parse:

```lua
bearcad.rect{ width =
```
]])

local blocks = bearcad.ai.blocks()
assert(#blocks == 2, "both fenced blocks are offered, got " .. #blocks)
assert(blocks[1].source:find("width = 120"), "the first block is the good one")
assert(not blocks[1].ran, "nothing runs on its own — that is the whole point")

-- The document is untouched until the block is run.
assert(not pcall(function() return bearcad.line_endpoints(0) end),
  "no geometry before the block is run")

local status = bearcad.ai.run_block(1)
assert(status and status:find("rectangle"), "running should report what it did, got: " .. tostring(status))
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
assert(math.abs(x1 - x0) > 119 and math.abs(x1 - x0) < 121,
  "the rectangle should be 120 wide, got " .. math.abs(x1 - x0))
assert(bearcad.ai.blocks()[1].ran, "the block records that it ran")

-- It went through the ordinary action path, so Undo takes it back.
bearcad.undo()
assert(not pcall(function() return bearcad.line_endpoints(0) end),
  "undo should remove geometry a suggested block created")

-- A block that does not parse fails loudly and leaves the document alone.
local ok, err = pcall(function() return bearcad.ai.run_block(2) end)
assert(not ok, "a broken block should fail")
assert(tostring(err):find("syntax error"), "the error should say what was wrong, got: " .. tostring(err))
assert(bearcad.ai.blocks()[2].error ~= nil, "the failure is recorded against that block")

print("ok: suggested Lua runs only on request, applies to the document, and undoes")
bearcad.quit()
