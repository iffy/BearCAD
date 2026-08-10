-- #879/#1201: after the click that places a dimension's label, the value field already has
-- the keyboard — a parameter name typed straight away lands in it, with no click on the field.
bearcad.new()
bearcad.parameter("add", "hole", "5mm")
bearcad.line{ x = 0, y = 0, x1 = 60, y1 = 0 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 0, 0}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("dimension")
bearcad.ui.wait(3)

bearcad.ui.click_ground(30, 0)
bearcad.ui.wait(5)
assert(bearcad.status():find("place the dimension"),
  "the edge click should arm a placeable length, got: " .. bearcad.status())

-- Drop the label clear of the line; the field opens focused.
bearcad.ui.click_ground(30, -12)
bearcad.ui.wait(5)
bearcad.ui.type("hole")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(6)

assert(bearcad.count("constraint") == 1,
  "expected the one length dimension, got " .. bearcad.count("constraint"))
assert(bearcad.status():find("dimension"),
  "expected the dimension to commit, got: " .. bearcad.status())

-- The typed expression must actually land (#1201): Enter alone used to commit the default
-- measured 60 mm, so the old count/status assertions passed while typing was ignored.
-- `hole` is 5 mm — the line must solve to that length.
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
local len = math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
assert(math.abs(len - 5) < 0.05,
  string.format(
    "line should be 5 mm after dimensioning with hole=5mm (typed without field click), got %.3f",
    len
  ))

print("ok: a placed dimension takes typing without a click on its field")
bearcad.quit()
