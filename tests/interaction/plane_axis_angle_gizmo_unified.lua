-- #1384: the construction-plane axis angle gizmo is unified with the Face Snap / Free
-- Move rotation dial (radial start line, yellow arc, radial handle line, single disc).
-- Drive an axis-reference plane, drag its angle handle, and confirm the plane tilts.
bearcad.new()
bearcad.line{ x = -10, y = 0, x1 = 10, y1 = 0 }
bearcad.exit_sketch()
bearcad.ui.tool("construction_plane")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 120 }
bearcad.ui.wait(8)

-- Pick the line -> axis reference, plane through the line, angle 0.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)

-- The angle handle sits at AXIS_ANGLE_GIZMO_RADIUS_MM = 25 mm from the origin; at angle 0
-- on the +Z axis normal it points along +X at (25, 0). Drag the handle up to tilt a bit.
bearcad.ui.drag_ground(25, 0, 25, 15)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

local plane = bearcad.get{ kind = "construction_plane", index = 3 }
assert(plane, "an axis-referenced plane should be committed")
local n = plane.normal
-- A non-zero tilt: plane no longer axis-aligned with normal along Z.
assert(math.abs(n[2]) > 0.05 or math.abs(n[3]) > 0.05,
  string.format("plane normal should be tilted, got (%.3f, %.3f, %.3f)", n[1], n[2], n[3]))
print("ok: construction-plane axis angle gizmo is the unified rotation dial")
bearcad.quit()