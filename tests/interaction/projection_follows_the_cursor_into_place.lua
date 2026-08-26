-- #1706: the Projection tool dropped its view at a canned spot on the page. The view rides
-- the cursor instead, until a click drops it — or Escape takes it back off the sheet.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.ui.wait(8)
assert(#bearcad.drawing_views(d) == 0, "an empty page to start")

bearcad.ui.tool("drawing_add")
bearcad.ui.wait(5)

-- Click the body row in the Elements pane: that starts the placement.
local row = bearcad.ui.elements_row_rect("Body 0")
assert(type(row) == "table", "the body row is on the Elements pane")
local vp = bearcad.ui.viewport()
bearcad.ui.click(row.x + row.w / 2 - vp.x, row.y + row.h / 2 - vp.y)
bearcad.ui.wait(8)
assert(#bearcad.drawing_views(d) == 1, "the view is on the page, riding the cursor")

-- Move the pointer: the view follows it.
-- The card is drawn centred on the cursor while it rides, so its own centre is a point that
-- is certainly on the sheet — whatever size the window is. Nudge from there.
local function card_centre(i)
  local c = bearcad.ui.drawing_view_rect(i)
  assert(c, "view " .. i .. " has a card on the page")
  return c.x + c.w / 2 - vp.x, c.y + c.h / 2 - vp.y
end

local cx, cy = card_centre(0)
bearcad.ui.move(cx, cy)
bearcad.ui.wait(6)
local a = bearcad.drawing_views(d)[1]
bearcad.ui.move(cx + 60, cy + 45)
bearcad.ui.wait(6)
local b = bearcad.drawing_views(d)[1]
assert(b.pos_x > a.pos_x + 0.005 and b.pos_y > a.pos_y + 0.005,
  string.format("the view should follow the cursor, (%.3f,%.3f) → (%.3f,%.3f)",
    a.pos_x, a.pos_y, b.pos_x, b.pos_y))

-- A click drops it there, and it stops following.
bearcad.ui.click(cx + 60, cy + 45)
bearcad.ui.wait(8)
local dropped = bearcad.drawing_views(d)[1]
bearcad.ui.move(cx, cy)
bearcad.ui.wait(6)
local after = bearcad.drawing_views(d)[1]
assert(math.abs(after.pos_x - dropped.pos_x) < 0.005 and math.abs(after.pos_y - dropped.pos_y) < 0.005,
  "once dropped it stays put")

-- Escape while placing takes the projection back off the page.
bearcad.ui.tool("drawing_add")
bearcad.ui.wait(5)
bearcad.ui.click(row.x + row.w / 2 - vp.x, row.y + row.h / 2 - vp.y)
bearcad.ui.wait(8)
assert(#bearcad.drawing_views(d) == 2, "a second view starts placing")
local ex, ey = card_centre(1)
bearcad.ui.move(ex, ey + 30)
bearcad.ui.wait(5)
bearcad.ui.key("escape")
bearcad.ui.wait(8)
assert(#bearcad.drawing_views(d) == 1,
  "Escape should cancel it, got " .. #bearcad.drawing_views(d) .. " views")

print("ok: a new projection follows the cursor until it is placed")
bearcad.quit()
