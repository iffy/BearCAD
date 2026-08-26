-- #1714: half of a point-to-point dimension is armed, so the second point has to come
-- off the same body. A multi-body view's fan over the other body offers nothing.
bearcad.new()
bearcad.cuboid{ width = 20, depth = 20, height = 20 }
bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {40, 0, 0} }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local card = bearcad.ui.drawing_view_rect(0)
assert(card, "the multi-body card is on the page")
local vp = bearcad.ui.viewport()

local function fan_at(x, y)
  bearcad.ui.move(x - vp.x, y - vp.y)
  bearcad.ui.wait(4)
  bearcad.ui.key("space")
  bearcad.ui.wait(6)
  local leaves = bearcad.exploder()
  local kinds = {}
  for _, l in ipairs(leaves) do kinds[l.kind] = (kinds[l.kind] or 0) + 1 end
  return leaves, kinds
end

local function close_fan()
  if #bearcad.exploder() > 0 then
    bearcad.ui.key("escape")
    bearcad.ui.wait(4)
  end
end

bearcad.ui.tool("dimension")
bearcad.ui.wait(6)

-- Front view of two side-by-side boxes: body 0 on the left of the card, body 1 on the right.
local left_x = card.x + card.w * 0.28
local right_x = card.x + card.w * 0.72
local mid_y = card.y + card.h / 2

local right_hit
local kinds_right = {}
for dx = -30, 30, 8 do
  for dy = -40, 40, 8 do
    local leaves, kinds = fan_at(right_x + dx, mid_y + dy)
    if (kinds["projected_edge"] or 0) + (kinds["projected_corner"] or 0) > 0 then
      right_hit = { x = right_x + dx, y = mid_y + dy }
      kinds_right = kinds
      close_fan()
      break
    end
    close_fan()
  end
  if right_hit then break end
end
assert(right_hit,
  "Dimension's idle fan offers projected geometry on the right body")
local _ = kinds_right

-- Arm a corner of the left body via the fan, so the pick is a real ProjectedCorner.
local hit
for dx = -40, 40, 8 do
  for dy = -40, 40, 8 do
    local leaves = fan_at(left_x + dx, mid_y + dy)
    for _, l in ipairs(leaves) do
      if l.kind == "projected_corner" and l.x then
        hit = l
        break
      end
    end
    if hit then break end
    close_fan()
  end
  if hit then break end
end
assert(hit, "expected a projected corner to fan out on the left body")
bearcad.ui.click(hit.x, hit.y)
bearcad.ui.wait(8)
assert(bearcad.status():find("second point"),
  "the first point is armed, status=" .. tostring(bearcad.status()))

-- With a point armed on body 0, the fan over body 1 must not offer that body's geometry.
local leaves_after, kinds_after = fan_at(right_hit.x, right_hit.y)
close_fan()
local n_after = #leaves_after
local shown = {}
for k, n in pairs(kinds_after) do shown[#shown + 1] = k .. "=" .. n end
assert(n_after == 0,
  "with a point armed on the left body, the right body should offer nothing, got "
    .. n_after .. " leaves (" .. table.concat(shown, ",") .. ")")
assert(not kinds_after["projected_edge"],
  "an armed point must not still fan edges of the other body")

print("ok: a half-made point dimension keeps the fan on its own body")
bearcad.quit()
