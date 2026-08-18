-- #1506: letter keys only arm a tool the current workbench toolbar would show.
-- Opening a drawing used to leave E/R/L switching to Extrude/Rectangle/Line.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
local d = bearcad.drawing{}
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(5)

assert(bearcad.tool_row().space == "drawing",
  "drawing should be open, space is " .. tostring(bearcad.tool_row().space))

local allowed = {}
for _, name in ipairs(bearcad.ui.toolbar_tools()) do
  allowed[name] = true
end
assert(allowed.select and allowed.dimension and allowed.text,
  "drawing toolbar should list Select/Dimension/Text")
assert(not allowed.extrude and not allowed.rectangle and not allowed.line,
  "3D tools must stay off the drawing toolbar")

for _, key in ipairs({ "e", "r", "l" }) do
  bearcad.ui.tool("select")
  bearcad.ui.wait(2)
  bearcad.ui.key(key)
  bearcad.ui.wait(2)
  local tool = bearcad.tool_row().tool
  assert(allowed[tool],
    "after " .. key .. " the tool must stay a drawing-workbench tool, got " .. tool)
end

-- D is on the drawing bar and must still arm Dimension.
bearcad.ui.tool("select")
bearcad.ui.wait(2)
bearcad.ui.key("d")
bearcad.ui.wait(2)
assert(bearcad.tool_row().tool == "dimension",
  "D should still arm Dimension in a drawing, got " .. bearcad.tool_row().tool)

print("ok: drawing letter keys stay on the workbench")
bearcad.quit()
