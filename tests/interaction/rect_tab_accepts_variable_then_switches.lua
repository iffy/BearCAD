-- #1573: while drawing a rectangle, Tab first accepts the highlighted variable
-- name, then a second Tab moves to the other dimension.
bearcad.new()
bearcad.parameter("add", "foo", "20mm")
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

bearcad.ui.click_ground(10, 10)
bearcad.ui.wait(5)
bearcad.ui.move_ground(50, 30)
bearcad.ui.wait(4)

-- Prefix of `foo`; first Tab completes it and stays on width.
bearcad.ui.type("fo")
bearcad.ui.wait(4)
bearcad.ui.key("tab")
bearcad.ui.wait(4)
-- Second Tab leaves width and arms height.
bearcad.ui.key("tab")
bearcad.ui.wait(2)
bearcad.ui.type("10")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("line") >= 5,
  "expected seed line + closed rectangle, got " .. bearcad.count("line"))

local found_foo = false
local found_10 = false
for i = 0, bearcad.count("constraint") - 1 do
  local c = bearcad.get{ kind = "constraint", index = i }
  if c and c.kind == "distance" then
    if c.expression == "foo" then found_foo = true end
    if c.expression == "10" then found_10 = true end
  end
end
assert(found_foo, "width should commit as parameter foo after Tab accepted the completion")
assert(found_10, "height should be the 10 typed after the second Tab")

local function side_len(i)
  local x0, y0, x1, y1 = bearcad.line_endpoints(i)
  return math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
end
local found_20 = 0
local found_h10 = 0
for i = 1, bearcad.count("line") - 1 do
  local len = side_len(i)
  if math.abs(len - 20) < 0.15 then found_20 = found_20 + 1 end
  if math.abs(len - 10) < 0.15 then found_h10 = found_h10 + 1 end
end
assert(found_20 >= 2,
  string.format("expected two 20 mm sides from foo, got %d", found_20))
assert(found_h10 >= 2,
  string.format("expected two 10 mm sides from the typed height, got %d", found_h10))

print("ok: rectangle Tab accepts a variable then switches to the other dim")
bearcad.quit()
