-- #1915: bodies selected in the Elements pane seed the Projection tool, so activating
-- it immediately starts placing a view of those bodies — no extra pane clicks.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
bearcad.cuboid{ at = { 40, 0, 0 }, width = 20, depth = 20, height = 10 }
local d = bearcad.drawing{}
bearcad.ui.wait(8)
assert(#bearcad.drawing_views(d) == 0, "an empty page to start")

local row0 = bearcad.ui.elements_row_rect("Body 0")
local row1 = bearcad.ui.elements_row_rect("Body 1")
assert(type(row0) == "table" and type(row1) == "table", "both body rows are on the Elements pane")
bearcad.ui.click(row0)
bearcad.ui.wait(3)
bearcad.ui.click(row1, { shift = true })
bearcad.ui.wait(3)
local sel = bearcad.selection()
assert(#sel == 2, "both bodies should be selected, got " .. #sel)

bearcad.ui.tool("drawing_add")
bearcad.ui.wait(5)

local views = bearcad.drawing_views(d)
assert(#views == 1, "activating Projection should place one view, got " .. #views)
local bodies = views[1].bodies
assert(type(bodies) == "table", "drawing_views should report the bodies")
table.sort(bodies)
assert(bodies[1] == 0 and bodies[2] == 1 and #bodies == 2,
  "the view should project both selected bodies, got " .. table.concat(bodies, ","))

-- The view rides the cursor until a click drops it, same as a pane click (#1706).
local function card()
  local c = bearcad.ui.drawing_view_rect(0)
  assert(c, "the new view has a card on the page")
  return c
end
local c0 = card()
bearcad.ui.move(c0)
local a = bearcad.drawing_views(d)[1]
bearcad.ui.move({ x = c0.x + c0.w / 2 + 60, y = c0.y + c0.h / 2 + 45 })
local b = bearcad.drawing_views(d)[1]
assert(b.pos_x > a.pos_x + 0.005 and b.pos_y > a.pos_y + 0.005,
  string.format("the view should follow the cursor, (%.3f,%.3f) → (%.3f,%.3f)",
    a.pos_x, a.pos_y, b.pos_x, b.pos_y))

print("ok: Projection starts placing the preselected bodies")
bearcad.quit()
