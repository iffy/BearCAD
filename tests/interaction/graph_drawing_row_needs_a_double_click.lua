-- #1927 / #1712: re-entering a drawing from the Graph view takes a double-click, the
-- same as the List-view Drawing row — a single click must not switch workbenches.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.workbench("model")
bearcad.ui.tool("select")
bearcad.ui.elements_view("graph")
bearcad.ui.wait(8)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local drawing = row_named("drawing")
assert(drawing and drawing.x, "the graph shows the drawing row")

bearcad.ui.click({ x = drawing.x + drawing.w * 0.6, y = drawing.y + drawing.h / 2 })
assert(bearcad.ui.workbench() ~= "drawing",
  "one click should not enter the drawing, workbench=" .. tostring(bearcad.ui.workbench()))

bearcad.ui.double_click({ x = drawing.x + drawing.w * 0.6, y = drawing.y + drawing.h / 2 })
assert(bearcad.ui.workbench() == "drawing",
  "a double-click opens it, workbench=" .. tostring(bearcad.ui.workbench()))

print("ok: re-entering a drawing from the graph takes a double-click")
bearcad.quit()
