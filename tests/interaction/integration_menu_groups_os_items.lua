-- Interaction regression (#1622): the developer/integration items — Install "bearcad"
-- Command in PATH, Install AI Agent Skill…, AI MCP Server… — live together under one
-- Integration menu, and the AI MCP Server item opens the AI pane at its MCP section.
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
assert(labels["Install AI Agent Skill…"],
  "Integration should hold Install AI Agent Skill…")
assert(labels["AI MCP Server…"],
  "Integration should hold AI MCP Server…")

-- The AI MCP Server item opens the configuration pane at the MCP Server section.
bearcad.ui.ai_mcp("show")
assert(bearcad.status():find("MCP Server"),
  "AI MCP Server… should open the pane at the MCP Server section, got: " .. bearcad.status())

print("ok: the Integration menu groups the integration items, and AI MCP Server… lands on its section")
bearcad.quit()