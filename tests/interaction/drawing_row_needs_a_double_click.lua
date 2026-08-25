-- #1712: a single click on a Drawing row jumped straight into the drawing workbench. Re-entering
-- a drawing takes a double-click, the same as reopening a sketch.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.workbench("model")
bearcad.ui.tool("select")
bearcad.ui.wait(8)

local row = bearcad.ui.elements_row_rect("Drawing 0")
assert(type(row) == "table", "the Drawing row is on the Elements pane")
local vp = bearcad.ui.viewport()
local x, y = row.x + row.w / 2 - vp.x, row.y + row.h / 2 - vp.y

bearcad.ui.click(x, y)
bearcad.ui.wait(8)
assert(bearcad.ui.workbench() ~= "drawing",
  "one click should not enter the drawing, workbench=" .. tostring(bearcad.ui.workbench()))

bearcad.ui.double_click(x, y)
bearcad.ui.wait(8)
assert(bearcad.ui.workbench() == "drawing",
  "a double-click opens it, workbench=" .. tostring(bearcad.ui.workbench()))

print("ok: re-entering a drawing takes a double-click")
bearcad.quit()
