-- #903: with the Select tool and **nothing selected**, press-and-drag a jointed part by an
-- edge or a face and it moves through its joint — no select-first step.
bearcad.new()
-- A 50×10 rail and a 10×10 slab beside it, sharing the corner (50, 0), on a +Y slider.
bearcad.rect{ width = 50, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.rect{ x = 50, y = 0, width = 10, height = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
bearcad.exit_sketch()
bearcad.joint{
  a = 0, b = 1, kind = "slider",
  from   = { body = 1, vertex = {50, 0, 0} },
  to     = { body = 0, vertex = {50, 0, 0} },
  from_b = { body = 1, vertex = {50, 10, 0} },
  to_b   = { body = 0, vertex = {50, 10, 0} },
  slide_max = 30,
}
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 5, 0}, distance = 220 }
bearcad.ui.wait(5)

-- Nothing selected: the drag still slides the slab through its joint.
assert(#bearcad.selection() == 0, "the scene starts with nothing selected")
local before = bearcad.body_stats(1).bbox.min[2]
bearcad.ui.drag_ground(55, 5, 55, 25)
bearcad.ui.wait(8)
local moved = bearcad.body_stats(1).bbox.min[2] - before
assert(moved > 19 and moved < 21,
  "dragging the unselected slab's face should slide it 20 mm, moved " .. moved)
assert(bearcad.status():find("Edited joint"),
  "the drag should land as one edit, status: " .. bearcad.status())

-- Its edge drags it too — here the far edge at x = 60, back toward the start.
bearcad.clear_selection()
before = bearcad.body_stats(1).bbox.min[2]
bearcad.ui.drag_ground(60, 25, 60, 10)
bearcad.ui.wait(8)
moved = bearcad.body_stats(1).bbox.min[2] - before
assert(moved < -14 and moved > -16,
  "dragging the slab's edge should slide it back 15 mm, moved " .. moved)

-- A plain click still just selects: no stray joint edit from a press-and-release.
bearcad.clear_selection()
before = bearcad.body_stats(1).bbox.min[2]
-- The slab now spans y = 5..15 (20 mm out, 15 back), so its middle is (55, 10).
bearcad.ui.click_ground(55, 10)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body",
  "the click should select the slab, got " .. (#sel > 0 and sel[1].kind or "nothing"))
assert(bearcad.body_stats(1).bbox.min[2] == before, "a click must not move the part")

print("ok: an unselected jointed part drags through its joint by face or edge")
bearcad.quit()
