-- Interaction regression (#459): a dimensioned-but-unpinned rect must still drag —
-- the shape translates, dimensions intact. (The DOF analysis once counted the
-- solver's weak gauge-hold pins as constraints, freezing every dimensioned shape.)
bearcad.new()
bearcad.rect{ x = 15, y = 10, width = 40, height = 20 }
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.click_ground(55, 10)      -- the corner at (x+width, y): select
bearcad.ui.wait(5)
bearcad.ui.drag_ground(55, 10, 75, 25)
bearcad.ui.wait(10)
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
local width = math.abs(x1 - x0)
assert(math.abs(width - 40) < 0.1,
  string.format("width must stay dimensioned at 40, got %.2f", width))
assert(math.abs(x1 - 55) > 5 or math.abs(y1 - 10) > 5,
  string.format("the rect must translate under the drag, corner still at (%.1f, %.1f)", x1, y1))
print("ok: dimensioned rect translates")
bearcad.quit()
