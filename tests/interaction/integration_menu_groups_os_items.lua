-- Interaction regression (#1622/#1632/#1633): the Integration menu holds Install
-- "bearcad" Command in PATH, and an AI submenu with the two ways an agent drives
-- BearCAD -- MCP Server… first, Install AI Agent Skill… last.
--
-- The native OS menu bar can't be driven by pointer input, so the menu's shape is
-- asserted through the `bearcad.ui.menu_structure` scripting hook instead.
bearcad.new()
bearcad.ui.tool("select")

local bars = bearcad.ui.menu_structure()
local titles = {}
for title in pairs(bars) do titles[#titles + 1] = tostring(title) end
table.sort(titles)
local integration = bars["Integration"]
assert(integration ~= nil,
  "the menu bar should have an Integration menu, it has: " .. table.concat(titles, ", "))

local labels = {}
for _, label in ipairs(integration) do labels[label] = true end
assert(labels['Install "bearcad" Command in PATH'],
  "Integration should hold Install \"bearcad\" Command in PATH")
assert(labels["AI"], "Integration should hold an AI submenu")

local ai = bars["Integration ▸ AI"]
assert(ai ~= nil, "the AI submenu should report its own items")
assert(ai[1] == "MCP Server…",
  "MCP Server… comes first, got '" .. tostring(ai[1]) .. "'")
assert(ai[2] == "Install AI Agent Skill…",
  "Install AI Agent Skill… comes last, got '" .. tostring(ai[2]) .. "'")
assert(#ai == 2, "and nothing else -- BearCAD talks to no AI service (#1633), got " .. #ai)

-- The MCP Server item opens the AI pane at its MCP Server section.
bearcad.ui.ai_mcp("show")
assert(bearcad.status():find("MCP Server"),
  "MCP Server… should open the pane at the MCP Server section, got: " .. bearcad.status())

print("ok: the Integration menu groups the OS items and nests the AI ones")
bearcad.quit()
