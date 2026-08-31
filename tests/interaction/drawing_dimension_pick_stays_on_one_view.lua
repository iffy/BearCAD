-- #1714: half of a point-to-point dimension is armed, so both points have to come off the
-- same view. The Selection Exploder's fan only offers that view's geometry — a loupe from
-- another card would toggle a dimension there instead of finishing this one.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "right" }
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.3, y = 0.5 }
bearcad.drawing_move_view{ drawing = d, view = 1, x = 0.7, y = 0.5 }
bearcad.ui.tool("dimension")
bearcad.ui.wait(8)

local card0 = bearcad.ui.drawing_view_rect(0)
local card1 = bearcad.ui.drawing_view_rect(1)
assert(card0 and card1, "both cards are on the page")
local vp = bearcad.ui.viewport()

-- Fan over the second card with nothing armed: it offers that card's own things.
local function fan_at(x, y)
  bearcad.ui.move(x - vp.x, y - vp.y)
  bearcad.ui.wait(4)
  bearcad.ui.key("space")
  bearcad.ui.wait(6)
  local n = #bearcad.ui.exploder()
  -- Esc with the fan closed leaves the Dimension tool.
  if n > 0 then
    bearcad.ui.key("escape")
    bearcad.ui.wait(4)
  end
  return n
end

-- The Dimension picker takes projected edges and corners, not the card, so scan the
-- second card for geometry rather than fanning its empty middle.
local hit_x, hit_y
for dy = -160, 160, 8 do
  local n = fan_at(card1.x + card1.w / 2, card1.y + card1.h / 2 + dy)
  if n > 0 then
    hit_x, hit_y = card1.x + card1.w / 2, card1.y + card1.h / 2 + dy
    break
  end
end
assert(hit_x, "the second card fans projected geometry with nothing armed")

-- Arm the first point of a free dimension on the *first* card: click empty paper inside it.
bearcad.ui.click(card0.x + card0.w / 2 - vp.x, card0.y + card0.h / 2 - vp.y)
bearcad.ui.wait(8)

assert(bearcad.status():find("second point"),
  "the first point is armed, status=" .. tostring(bearcad.status()))
-- Now the fan over the second card offers nothing: the pending pick owns view 0.
assert(fan_at(hit_x, hit_y) == 0,
  "with a point armed on the front view, the side view should offer nothing")

print("ok: a half-made point dimension keeps the fan on its own view")
bearcad.quit()
