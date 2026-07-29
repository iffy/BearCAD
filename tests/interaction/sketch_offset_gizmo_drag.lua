-- #939: the in-sketch Offset tool's push-pull handle is a real, grabbable gizmo — dragging it
-- sets the signed distance, and Enter commits the parallel copy at the dragged distance.
bearcad.new()
-- A single line along +U; its offset side is +V (left of the stored direction).
bearcad.line{ x = -10, y = 0, x1 = 10, y1 = 0 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 120 }
bearcad.ui.wait(8)

bearcad.ui.tool("offset")
bearcad.ui.wait(3)
-- Click the line into the offset set. The handle then sits at the midpoint offset along +V by
-- the default 5 mm distance, i.e. at (0, 5).
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)

-- Grab the handle and pull it out to v = 14.
bearcad.ui.drag_ground(0, 5, 0, 14)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

assert(bearcad.count("line") == 2,
  "expected the source line plus its offset copy, got " .. bearcad.count("line"))
local _, y0, _, y1 = bearcad.line_endpoints(1)
assert(math.abs(y0 - 14) < 0.6 and math.abs(y1 - 14) < 0.6,
  string.format("dragging the handle to v=14 should offset there, got y=(%.2f, %.2f)", y0, y1))

print("ok: the offset push-pull handle drags the distance")
bearcad.quit()
