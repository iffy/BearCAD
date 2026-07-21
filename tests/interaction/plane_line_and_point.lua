-- #483: Plane tool Anchor accepts line + point together — plane through the point,
-- normal along the line (not the axis "line lies in the plane" mode).
bearcad.new()
-- Horizontal line along +X (the normal direction). Midpoint (15, 5) is far from ends.
bearcad.line{ x = 0, y = 5, x1 = 30, y1 = 5 }
-- Separate short segment whose start is the point we want the plane through.
bearcad.line{ x = 10, y = 20, x1 = 12, y1 = 22 }
bearcad.exit_sketch()
bearcad.ui.tool("construction_plane")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Line first (axis mode), then the other line's start point upgrades to line+point.
bearcad.ui.click_ground(15, 5)
bearcad.ui.wait(8)
bearcad.ui.click_ground(10, 20)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(8)

local plane = bearcad.get{ kind = "construction_plane", index = 1 }
assert(plane, "a plane should have been committed")
local n = plane.normal
local o = plane.origin
-- Normal along the first line (±X); origin at the second pick (10, 20, 0) plus offset 0.
assert(math.abs(math.abs(n[1]) - 1.0) < 1e-3 and math.abs(n[2]) < 1e-3 and math.abs(n[3]) < 1e-3,
  string.format("plane normal should be ±X, got (%.3f, %.3f, %.3f)", n[1], n[2], n[3]))
assert(math.abs(o[1] - 10.0) < 1e-2 and math.abs(o[2] - 20.0) < 1e-2 and math.abs(o[3]) < 1e-2,
  string.format("plane origin should be near (10, 20, 0), got (%.3f, %.3f, %.3f)", o[1], o[2], o[3]))
print("ok: plane through point is normal to the line")
bearcad.quit()
