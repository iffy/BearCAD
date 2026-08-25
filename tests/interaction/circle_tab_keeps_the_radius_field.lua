-- #1718: a circle has exactly one value field, so Tab has nowhere to go. It must stay on
-- the diameter input rather than handing focus to the toolbar, where typing is lost.
bearcad.new()
bearcad.parameter("add", "bore", "18mm")
-- A sketch to draw in: the Circle tool needs one open before it takes a ground click.
bearcad.line{ x = -60, y = -60, x1 = -50, y1 = -60 }
bearcad.open_sketch(0)
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("circle")
bearcad.ui.wait(3)

-- Pin the centre, then Tab off and back to nothing: focus must not leave the field.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)
bearcad.ui.key("tab")
bearcad.ui.wait(5)
bearcad.ui.type("bore")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("circle") == 1,
  "expected the typed diameter to commit one circle, got " .. bearcad.count("circle"))
assert(bearcad.status():find("18"),
  "expected an 18 mm circle from `bore` typed after Tab, got: " .. bearcad.status())

print("ok: Tab leaves the circle's only value field focused")
bearcad.quit()
