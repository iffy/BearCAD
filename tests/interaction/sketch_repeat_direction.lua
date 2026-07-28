-- #835: the in-sketch Repeat tool collects entities by clicking and takes its direction from
-- a Shift+clicked line, and Enter commits a repeat running along that line.
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 4 }
-- A line along +Y, well away from the circle, used as the repeat direction.
bearcad.line{ x = 30, y = 0, x1 = 30, y1 = 40 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

bearcad.ui.tool("repeat")
bearcad.ui.wait(3)
-- Click the circle's rim into the entity set.
bearcad.ui.click_ground(0, 4)
bearcad.ui.wait(8)
-- Shift+click the line: direction, not a fourth entity.
bearcad.ui.click_ground(30, 20, { shift = true })
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

-- Three instances = the original circle plus two copies.
assert(bearcad.count("circle") == 3,
  "expected 2 copies of the circle, got " .. bearcad.count("circle"))
-- The copies run along the Shift+clicked line (+V), not the default U axis.
for i = 1, 2 do
  local c = bearcad.get{ kind = "circle", index = i }
  assert(c, "copy " .. i .. " should exist")
  assert(math.abs(c.x) < 1e-2 and c.y > 1.0,
    string.format("copy %d should sit up the +V axis, got (%.2f, %.2f)", i, c.x, c.y))
end

print("ok: in-sketch repeat takes its direction from a Shift+clicked line")
bearcad.quit()
