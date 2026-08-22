-- #1638: with a variable's name typed in full, the rectangle's width field still shows the
-- fuzzy-variable dropdown. The first Tab accepts the name and closes the dropdown *without*
-- leaving the field; only the second Tab moves to the other dimension.
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

-- `foo` typed in full: the dropdown is still up, so this Tab only closes it.
bearcad.ui.type("foo")
bearcad.ui.wait(4)
bearcad.ui.key("tab")
bearcad.ui.wait(4)
-- Still on width — typing extends the same expression.
bearcad.ui.type("*2")
bearcad.ui.wait(4)
-- Now Tab leaves width and arms height.
bearcad.ui.key("tab")
bearcad.ui.wait(2)
bearcad.ui.type("10")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

local found_width = false
local found_height = false
for i = 0, bearcad.count("constraint") - 1 do
  local c = bearcad.get{ kind = "constraint", index = i }
  if c and c.kind == "distance" then
    if c.expression == "foo*2" then found_width = true end
    if c.expression == "10" then found_height = true end
  end
end
assert(found_width,
  "the first Tab should have closed the dropdown and kept the caret in width, so `*2` extends `foo`")
assert(found_height, "height should be the 10 typed after the second Tab")

local function side_len(i)
  local x0, y0, x1, y1 = bearcad.line_endpoints(i)
  return math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
end
local found_40 = 0
local found_10 = 0
for i = 1, bearcad.count("line") - 1 do
  local len = side_len(i)
  if math.abs(len - 40) < 0.15 then found_40 = found_40 + 1 end
  if math.abs(len - 10) < 0.15 then found_10 = found_10 + 1 end
end
assert(found_40 >= 2, string.format("expected two 40 mm sides from foo*2, got %d", found_40))
assert(found_10 >= 2, string.format("expected two 10 mm sides from the typed height, got %d", found_10))

print("ok: Tab closes the variable dropdown before it switches rectangle dimensions")
bearcad.quit()
