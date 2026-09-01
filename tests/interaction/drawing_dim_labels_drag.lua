-- #1774: a drawing dimension's label drags with the pointer. A free point-to-point
-- dimension's label starts at its auto-placed gap and dragging it lands an offset override —
-- live under the pointer, one undo step on release. Driven through real pointer input.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
local cx, cy = vp.width / 2, vp.height / 2

-- Two clicks off the edges add a free dimension (#1645), like the two-clicks test.
bearcad.ui.tool("dimension")
bearcad.ui.wait(4)
bearcad.ui.move(cx - 30, cy - 40)
bearcad.ui.wait(3)
bearcad.ui.click(cx - 30, cy - 40)
bearcad.ui.wait(5)
assert(bearcad.status():find("second point"), "the first click arms the second")
bearcad.ui.move(cx + 30, cy - 60)
bearcad.ui.wait(3)
bearcad.ui.click(cx + 30, cy - 60)
bearcad.ui.wait(6)
assert(bearcad.status():find("dimension"), "the second click adds the dimension")

local function point_dim_offset()
  local dim = bearcad.get{ kind = "point_dimension", drawing = d, view = 0, index = 0 }
  return dim.offset
end
assert(point_dim_offset() == 0.0, "a fresh dimension sits at its default gap")

-- Find the label: with Select, clicking it selects the dimension. The new dimension is
-- still selected from being added, so clear the selection on blank paper first — the scan
-- must see the click *cause* the selection, not inherit it.
bearcad.ui.tool("select")
bearcad.ui.wait(5)
bearcad.ui.click(cx + 120, cy + 120)   -- blank page: clears the selection
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
local label_x, label_y = nil, nil
for dy = -36, 60, 8 do
  for dx = -60, 60, 10 do
    local x, y = cx + dx, cy - 50 + dy
    bearcad.ui.move(x, y)
    bearcad.ui.wait(1)
    bearcad.ui.click(x, y)
    bearcad.ui.wait(3)
    if selected_kind() == "drawing_dimension" then
      label_x, label_y = x, y
      break
    end
  end
  if label_x then break end
end
assert(label_x, "clicking the point dimension's label selects it")

-- Dragging the label outward lands an offset override; the card must not move with it.
bearcad.ui.drag(label_x, label_y, label_x, label_y + 40)
bearcad.ui.wait(6)
local offset = point_dim_offset()
assert(offset > 2.0, "dragging the label outward moves the dimension, got offset " .. offset)

-- Re-find the label: a too-short one sits on the line (#1926), so it does not
-- stay under the pointer the way an offset-above-the-line label does.
bearcad.ui.click(cx + 120, cy + 120)
bearcad.ui.wait(3)
label_x, label_y = nil, nil
for dy = -36, 80, 8 do
  for dx = -60, 60, 10 do
    local x, y = cx + dx, cy - 50 + dy
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
assert(label_x, "the label is still clickable after dragging it")

-- Dragging it back lands a smaller offset (the drag is live, not a one-shot jump).
bearcad.ui.drag(label_x, label_y, label_x, label_y - 34)
bearcad.ui.wait(6)
local back = point_dim_offset()
assert(back < offset, "dragging back reduces the offset, got " .. back .. " after " .. offset)

print("ok: drawing dimension labels drag")
bearcad.quit()
