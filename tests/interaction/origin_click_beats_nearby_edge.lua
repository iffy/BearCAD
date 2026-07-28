-- #852: the sketch origin is clickable even with a reference line right beside it — the
-- datum plane's own border runs 5mm from the origin, and it used to take the click, so
-- "click the corner, Shift+click the origin" quietly selected an edge instead.
bearcad.new()
bearcad.line{ x = 10, y = 10, x1 = 60, y1 = 12 }
bearcad.line{ x = 60, y = 12, x1 = 30, y1 = 40 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- Frame the origin comfortably inside the viewport, as the tutorial's own framing does.
bearcad.ui.camera{ target = {22, 25, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.tool("constraint")
bearcad.ui.wait(3)

bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "origin",
  "a click on the origin should select the origin, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

-- And it joins a vertex on a Shift+click, which is what the coincident step asks for.
bearcad.clear_selection()
bearcad.ui.wait(3)
bearcad.ui.click_ground(10, 10)
bearcad.ui.wait(8)
bearcad.ui.click_ground(0, 0, { shift = true })
bearcad.ui.wait(8)
sel = bearcad.selection()
assert(#sel == 2, "corner + origin should both be selected, got " .. #sel)
local kinds = {}
for _, e in ipairs(sel) do kinds[e.kind] = true end
assert(kinds["origin"] and kinds["point"],
  "expected the origin and a sketch point in the selection")

print("ok: the origin takes the click, not the line beside it")
bearcad.quit()
