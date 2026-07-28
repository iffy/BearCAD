-- #833: a selected construction plane carries a grip on two opposite corners; dragging one
-- with the Select tool resizes the plane's rectangle, and the whole drag is one undo step.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.tool("select")
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

local function extent()
  return bearcad.get{ kind = "construction_plane", index = 0 }.extent
end

local before = extent()
assert(math.abs(before.u_max - 100.0) < 1e-2 and math.abs(before.v_max - 100.0) < 1e-2,
  string.format("the XY datum plane starts 100x100, got %.1f x %.1f", before.u_max, before.v_max))

-- Drag the far (u_max, v_max) grip: wider in u, shorter in v.
bearcad.ui.drag_ground(100, 100, 140, 60)
bearcad.ui.wait(20)
local after = extent()
assert(math.abs(after.u_max - 140.0) < 1.0 and math.abs(after.v_max - 60.0) < 1.0,
  string.format("far grip should follow the pointer, got u_max %.1f v_max %.1f",
                after.u_max, after.v_max))
assert(math.abs(after.u_min) < 1e-2 and math.abs(after.v_min) < 1e-2,
  "the opposite corner should stay put")

-- The drag is a single undo step.
bearcad.undo()
bearcad.ui.wait(10)
local undone = extent()
assert(math.abs(undone.u_max - 100.0) < 1e-2 and math.abs(undone.v_max - 100.0) < 1e-2,
  string.format("one undo should restore the whole drag, got %.1f x %.1f",
                undone.u_max, undone.v_max))

-- The low (u_min, v_min) grip drags the other way.
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(3)
bearcad.ui.drag_ground(0, 0, -30, 20)
bearcad.ui.wait(20)
local low = extent()
assert(math.abs(low.u_min + 30.0) < 1.0 and math.abs(low.v_min - 20.0) < 1.0,
  string.format("low grip should follow the pointer, got u_min %.1f v_min %.1f",
                low.u_min, low.v_min))

print("ok: construction plane corner grips resize the plane")
bearcad.quit()
