-- #1296: gizmo pull handles quantize onto a 0.1-of-unit grid. With the document in inches,
-- dragging the Offset handle past an off-grid mm position lands on the nearest 0.1 in
-- (2.54 mm) — not the old hardcoded 0.1 mm step.
bearcad.new()
bearcad.set_units{ length = "in" }
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
-- Click the line into the offset set. Default distance is 5 mm → handle at (0, 5).
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)

-- Pull to v ≈ 14.37 mm. 0.1 in = 2.54 mm → nearest is 6 × 2.54 = 15.24 mm.
-- (A fixed 0.1 mm step would have landed on 14.4 instead.)
bearcad.ui.drag_ground(0, 5, 0, 14.37)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

assert(bearcad.count("line") == 2,
  "expected the source line plus its offset copy, got " .. bearcad.count("line"))
local _, y0, _, y1 = bearcad.line_endpoints(1)
local want = 15.24
assert(math.abs(y0 - want) < 0.15 and math.abs(y1 - want) < 0.15,
  string.format(
    "inch-unit gizmo should snap 14.37 mm → 15.24 mm (0.6 in), got y=(%.3f, %.3f)",
    y0, y1))

print("ok: gizmo drag steps by 0.1 of the document unit")
bearcad.quit()
