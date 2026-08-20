-- Interaction regression (#1620/#1621): the AI affordance is two panes. An "AI" config
-- pane (chat-backend configuration, plus agents/skills/MCP) and an "AI Chat" pane that
-- opens at the bottom of the app and spans it, the way the command palette does.
--
-- Both exist (their rects report), the config pane reports its two sections by name,
-- and the chat pane -- being a full-width bottom console -- reads wider than the
-- confined config pane.
bearcad.new()
bearcad.ui.tool("select")

-- The AI config pane. Off by default; showing it reports a rect and its two sections.
bearcad.ui.pane("ai", "show")
bearcad.ui.wait(30)
local config = bearcad.ui.pane_rect("ai")
assert(config, "an open AI config pane reports its rect")
local sections = bearcad.ui.ai_pane_sections()
assert(#sections == 2, "the AI config pane has two sections, got " .. #sections)
assert(sections[1] == "Use AI inside BearCAD",
  "first section is the chat-backend configuration, got '" .. tostring(sections[1]) .. "'")
assert(sections[2] == "Have AI use BearCAD",
  "second section holds agents/skills and MCP, got '" .. tostring(sections[2]) .. "'")

-- The AI Chat pane. Same, but it is a full-width bottom console, so it reads wider than
-- the confined config pane.
bearcad.ui.pane("ai_chat", "show")
bearcad.ui.wait(30)
local chat = bearcad.ui.pane_rect("ai_chat")
assert(chat, "an open AI Chat pane reports its rect")
assert(chat.w > 0 and chat.h > 0, "the chat pane has a size, got " ..
  chat.w .. "x" .. chat.h)
assert(chat.w > config.w,
  "a bottom console should span wider than the config pane, got " ..
  chat.w .. " vs " .. config.w)
assert(chat.w > chat.h, "the chat pane is a bottom console (wider than tall), got " ..
  chat.w .. "x" .. chat.h)

-- Hiding the Chat pane forgets its rect.
bearcad.ui.pane("ai_chat", "hide")
bearcad.ui.wait(10)
assert(bearcad.ui.pane_rect("ai_chat") == nil, "a hidden chat pane reports no rect")

print("ok: the AI pane splits into a config pane and a bottom AI Chat pane")
bearcad.quit()