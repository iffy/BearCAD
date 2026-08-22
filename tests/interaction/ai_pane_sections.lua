-- Interaction regression (#1620/#1621/#1633): the AI pane holds the two ways an agent
-- drives BearCAD -- the local MCP server and the agent skill -- and nothing else. There
-- is no chat pane: BearCAD does not talk to models, agents talk to it.
bearcad.new()
bearcad.ui.tool("select")

-- Off by default; showing it reports a rect and its two sections, in draw order.
bearcad.ui.pane("ai", "show")
bearcad.ui.wait(30)
local config = bearcad.ui.pane_rect("ai")
assert(config, "an open AI pane reports its rect")
local sections = bearcad.ui.ai_pane_sections()
assert(#sections == 2, "the AI pane has two sections, got " .. #sections)
assert(sections[1] == "MCP Server",
  "first section is the MCP server, got '" .. tostring(sections[1]) .. "'")
assert(sections[2] == "Agent Skill",
  "second section is the agent skill, got '" .. tostring(sections[2]) .. "'")

-- #1633: the AI Chat pane is gone, by every name it had.
for _, name in ipairs({ "ai_chat", "aichat", "chat" }) do
  assert(not pcall(bearcad.ui.pane, name, "show"),
    "there should be no '" .. name .. "' pane any more")
end

bearcad.ui.pane("ai", "hide")
bearcad.ui.wait(10)
assert(bearcad.ui.pane_rect("ai") == nil, "a hidden pane reports no rect")

print("ok: the AI pane holds the MCP server and the agent skill, and there is no chat pane")
bearcad.quit()
