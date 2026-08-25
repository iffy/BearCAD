-- #1708: with the Select tool armed, the arrow keys move whatever is selected — a nudge a
-- millimetre at a time, or ten with Shift held. On a drawing page that's the selected view;
-- in a sketch it's the selected geometry.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.5, y = 0.5 }
bearcad.ui.tool("select")
bearcad.ui.wait(8)

local card = bearcad.ui.drawing_view_rect(0)
assert(card, "the view is on the page")
local vp = bearcad.ui.viewport()
bearcad.ui.click(card.x + 12 - vp.x, card.y + card.h / 2 - vp.y)
bearcad.ui.wait(6)

local before = bearcad.drawing_views(d)[1]
bearcad.ui.key("ArrowRight")
bearcad.ui.wait(6)
local one = bearcad.drawing_views(d)[1]
assert(one.pos_x > before.pos_x,
  string.format("→ should nudge the view right, %.4f → %.4f", before.pos_x, one.pos_x))
local step = one.pos_x - before.pos_x

bearcad.ui.key("ArrowRight", { shift = true })
bearcad.ui.wait(6)
local two = bearcad.drawing_views(d)[1]
local big = two.pos_x - one.pos_x
assert(big > step * 5,
  string.format("Shift+→ should jump further: %.4f vs %.4f", big, step))

bearcad.ui.key("ArrowDown")
bearcad.ui.wait(6)
local three = bearcad.drawing_views(d)[1]
assert(three.pos_y > two.pos_y,
  string.format("↓ should nudge the view down the page, %.4f → %.4f", two.pos_y, three.pos_y))

-- In a sketch it moves the picked geometry instead.
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
bearcad.ui.tool("select")
bearcad.ui.wait(6)
bearcad.select{ kind = "line", index = 0 }
bearcad.ui.wait(4)
local x0 = select(1, bearcad.line_endpoints(0))
bearcad.ui.key("ArrowRight")
bearcad.ui.wait(6)
local x1 = select(1, bearcad.line_endpoints(0))
assert(x1 > x0 + 0.5,
  string.format("→ should nudge the sketch line, %.2f → %.2f", x0, x1))

print("ok: the arrow keys nudge the selection, and Shift jumps further")
bearcad.quit()
