-- #1926: a too-short drawing dimension's label sits on the line, and dragging
-- it along the line hangs it past the other end.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 5, depth = 40, height = 60 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
-- Top front edge along X: 5 mm, too short for "5.0 mm".
bearcad.drawing_dimension{ drawing = d, view = 0, a = {-2.5, -20, 60}, b = {2.5, -20, 60} }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local function dim()
  return bearcad.get{ kind = "edge_dimension", drawing = d, view = 0, index = 0 }
end
assert(dim() and dim().offset == 0.0, "a fresh edge dim sits at its default gap")
assert(dim().side == nil, "a fresh overflow label uses the auto far end")

local vp = bearcad.ui.viewport()
local card = assert(bearcad.ui.drawing_view_rect(0), "the front card is on the page")
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

local function find_label()
  bearcad.ui.click(cx + 160, cy + 160)
  bearcad.ui.wait(3)
  for dy = -90, 90, 6 do
    for dx = -90, 90, 6 do
      local x, y = cx + dx, cy + dy
      bearcad.ui.move(x, y)
      bearcad.ui.wait(1)
      bearcad.ui.click(x, y)
      bearcad.ui.wait(2)
      if selected_kind() == "drawing_dimension" then
        return x, y
      end
    end
  end
  return nil
end

local label_x, label_y = find_label()
assert(label_x, "clicking the short dimension's label selects it")

-- Drag along the (horizontal) dimension line, well past the other end.
bearcad.ui.drag(label_x, label_y, label_x - 80, label_y)
bearcad.ui.wait(6)
local left = dim().side
label_x, label_y = find_label()
assert(label_x, "the label is still clickable after the first drag")
bearcad.ui.drag(label_x, label_y, label_x + 160, label_y)
bearcad.ui.wait(6)
local right = dim().side
assert(left ~= nil and right ~= nil, "dragging a side-placed label stores an end")
assert(left ~= right, "dragging to the other end flips the side, got "
  .. tostring(left) .. " then " .. tostring(right))

print("ok: short drawing dimension labels drag to either side")
bearcad.quit()
