-- #879: after the click that places a dimension's label, the value field already has the
-- keyboard — a parameter name typed straight away lands in it, with no click on the field.
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

print("ok: a placed dimension takes typing without a click on its field")
bearcad.quit()
