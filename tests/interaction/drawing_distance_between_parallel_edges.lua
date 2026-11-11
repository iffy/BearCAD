-- #1970: with the Dimension tool on a drawing, Shift+clicking two parallel edges shows the
-- distance between them (two edges that meet still show their angle).
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.tool("dimension")
bearcad.ui.wait(8)

local card = bearcad.ui.drawing_view_rect(0)
assert(card, "the card is on the page")
local vp = bearcad.ui.viewport()
local cy = card.y + card.h / 2
local function dims() return bearcad.drawing_views(d)[1].dimensions end

-- Find the box's left and right sides along the card's middle row: a plain click on an edge
-- toggles its length, so a click that adds a dimension found one (click again to undo it).
local sides = {}
for x = card.x + 4, card.x + card.w - 4, 3 do
  local before = dims()
  bearcad.ui.click(x - vp.x, cy - vp.y)
  bearcad.ui.wait(3)
  if dims() > before then
    bearcad.ui.click(x - vp.x, cy - vp.y)
    bearcad.ui.wait(3)
    if #sides == 0 or x - sides[#sides] > 20 then sides[#sides + 1] = x end
  end
end
assert(#sides == 2, "expected the box's two vertical sides, found " .. #sides)

bearcad.ui.click(sides[1] - vp.x, cy - vp.y, { shift = true })
bearcad.ui.wait(3)
bearcad.ui.click(sides[2] - vp.x, cy - vp.y, { shift = true })
bearcad.ui.wait(5)
local v = bearcad.drawing_views(d)[1]
assert(v.distance_dimensions == 1,
  "Shift+clicking two parallel edges should show their distance, got "
  .. tostring(v.distance_dimensions) .. " (angles: " .. tostring(v.angle_dimensions) .. ")")
assert(v.angle_dimensions == 0, "parallel edges make no angle dimension")

print("ok: two parallel edges dimension their distance")
bearcad.quit()
