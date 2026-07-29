-- #938: the Offset tool takes a whole face — clicking inside a closed sketch profile adds
-- every line of that loop to the offset set, so one click offsets a rectangle's outline.
bearcad.new()
bearcad.rect{ x = -30, y = -15, width = 60, height = 30 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(8)

bearcad.ui.tool("offset")
bearcad.ui.wait(3)
-- One click well inside the rectangle, away from every edge.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
assert(bearcad.status():find("4 entities"),
  "a click inside the profile should take all four lines, got: " .. bearcad.status())

bearcad.ui.key("Enter")
bearcad.ui.wait(10)
assert(bearcad.count("line") == 8,
  "expected 4 source lines plus 4 offset copies, got " .. bearcad.count("line"))

print("ok: the offset tool takes a whole face's edges in one click")
bearcad.quit()
