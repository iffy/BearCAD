-- #870: clicking a circle's centre with the Dimension tool selects the **centre point**, not
-- the circle (which is its diameter) — so a centre-to-edge distance can be built at all.
bearcad.new()
bearcad.line{ x = 20, y = 10, x1 = 90, y1 = 10 }
bearcad.circle{ x = 45, y = 40, r = 6 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {50, 40, 0}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("dimension")
bearcad.ui.wait(3)

bearcad.ui.click_ground(45, 40)
bearcad.ui.wait(8)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "point",
  "the centre click should select the centre point, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

-- Shift+click the edge: the pair is a point-to-line distance, ready to place.
bearcad.ui.click_ground(55, 10, { shift = true })
bearcad.ui.wait(8)
sel = bearcad.selection()
assert(#sel == 2, "centre + edge should both be selected, got " .. #sel)
assert(bearcad.status():find("place the dimension"),
  "expected a placeable distance, got: " .. bearcad.status())

print("ok: a circle's centre dimensions against an edge")
bearcad.quit()
