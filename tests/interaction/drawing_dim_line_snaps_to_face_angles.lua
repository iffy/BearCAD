-- #1916: dragging a drawing dimension line snaps its offset direction to the
-- measured edge's perpendicular or to either face that makes that edge.
-- An isometric cuboid's top Y-edge: perp is not vertical; dragging the label
-- straight up lands on the vertical side-face snap.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 50 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front-right-top" }
-- Top +X edge, along Y: (20, -15, 50) → (20, 15, 50).
bearcad.drawing_dimension{ drawing = d, view = 0, a = {20, -15, 50}, b = {20, 15, 50} }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local function dim()
  return bearcad.get{ kind = "edge_dimension", drawing = d, view = 0, index = 0 }
end
assert(dim() and dim().offset == 0.0, "a fresh edge dim sits at its default gap")
assert(dim().angle == nil, "a fresh edge dim uses the auto perpendicular")

local vp = bearcad.ui.viewport()
local card = assert(bearcad.ui.drawing_view_rect(0), "the isometric card is on the page")
local cx = card.x + card.w / 2 - vp.x
local cy = card.y + card.h / 2 - vp.y

bearcad.ui.tool("select")
bearcad.ui.wait(5)
bearcad.ui.click(cx + 160, cy + 160)
bearcad.ui.wait(5)
local function selected_kind()
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == "Selection" then
      for _, it in ipairs(p.items) do return it.kind end
    end
  end
  return nil
end
assert(selected_kind() == nil, "clicking blank paper clears the selection")

local label_x, label_y
for dy = -80, 80, 8 do
  for dx = -80, 80, 8 do
    local x, y = cx + dx, cy + dy
    bearcad.ui.move(x, y)
    bearcad.ui.wait(1)
    bearcad.ui.click(x, y)
    bearcad.ui.wait(2)
    if selected_kind() == "drawing_dimension" then
      label_x, label_y = x, y
      break
    end
  end
  if label_x then break end
end
assert(label_x, "clicking the edge dimension's label selects it")

-- Drag straight up: the side face of this edge is vertical, so the line should
-- snap off the auto perpendicular onto that face.
bearcad.ui.drag(label_x, label_y, label_x, label_y - 70)
bearcad.ui.wait(8)
local after = dim()
assert(after.angle ~= nil, "dragging toward vertical stores a snap angle, offset="
  .. tostring(after.offset))
-- Vertical in projected millimetres is ±π/2.
local ang = after.angle
local dist = math.min(math.abs(ang - math.pi / 2), math.abs(ang + math.pi / 2))
assert(dist < 0.2,
  string.format("snap angle should be vertical, got %.3f rad (%.1f deg)", ang, ang * 180 / math.pi))

print("ok: drawing dimension lines snap to face angles")
bearcad.quit()
