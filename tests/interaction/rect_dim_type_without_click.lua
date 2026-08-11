-- #1278: after the first corner of a rectangle, the width field already has the keyboard
-- with its live value selected — typing overwrites without clicking the moving input.
bearcad.new()
-- Seed a ground sketch (line API auto-opens one); keep it clear of the rectangle area.
bearcad.line{ x = -20, y = -20, x1 = -20, y1 = -15 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 20, 0}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("rectangle")
bearcad.ui.wait(3)

-- First corner; then move the free corner so the live dims are non-zero and moving.
bearcad.ui.click_ground(10, 10)
bearcad.ui.wait(5)
assert(bearcad.status():find("type to lock"),
  "first click should arm the free corner, got: " .. bearcad.status())
bearcad.ui.move_ground(40, 25)
bearcad.ui.wait(4)
-- Move again: the floating field rides the cursor; focus + select-all must hold (#1278).
bearcad.ui.move_ground(50, 30)
bearcad.ui.wait(3)

-- Type width without clicking the field, Tab to height, type height, Enter to place.
bearcad.ui.type("10")
bearcad.ui.wait(3)
bearcad.ui.key("tab")
bearcad.ui.wait(2)
bearcad.ui.type("10")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

-- Seed line + 4 rect edges.
assert(bearcad.count("line") >= 5,
  "expected seed line + closed rectangle, got " .. bearcad.count("line"))

-- The four newest lines are the rectangle; both extents should be the typed 10 mm.
local function side_len(i)
  local x0, y0, x1, y1 = bearcad.line_endpoints(i)
  return math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
end
local found_10 = 0
for i = 1, bearcad.count("line") - 1 do
  if math.abs(side_len(i) - 10) < 0.15 then
    found_10 = found_10 + 1
  end
end
assert(found_10 >= 4,
  string.format("expected four 10 mm sides after type-without-click, got %d near 10 mm", found_10))

print("ok: rectangle dims accept type-to-overwrite without clicking the moving field")
bearcad.quit()
