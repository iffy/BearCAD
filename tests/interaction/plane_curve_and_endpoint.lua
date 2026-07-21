-- #483: pick a curve then its endpoint → plane through the endpoint, normal = curve
-- tangent at that end (not the "line lies in the plane" axis mode).
bearcad.new()
-- Curve from (6,4) toward (26,14); near handle at start (6,12) => outward tangent at
-- start is -Y. Away from the origin so the origin marker doesn't steal the pick.
bearcad.line{ x = 6, y = 4, x1 = 26, y1 = 14, bezier = { { 6, 12 }, { 18, 14 } } }
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

-- Curve body first (midpoint ~16,9), then the start endpoint.
bearcad.ui.click_ground(16, 9)
bearcad.ui.wait(8)
bearcad.ui.click_ground(6, 4)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(8)

local plane = bearcad.get{ kind = "construction_plane", index = 1 }
assert(plane, "a plane should have been committed")
local n = plane.normal
local o = plane.origin
-- Outward at start = -Y (away from the (6,12) handle).
assert(math.abs(n[1]) < 1e-3 and math.abs(math.abs(n[2]) - 1.0) < 1e-3 and math.abs(n[3]) < 1e-3,
  string.format("plane normal should be ±Y (end tangent), got (%.3f, %.3f, %.3f)", n[1], n[2], n[3]))
assert(math.abs(o[1] - 6.0) < 0.5 and math.abs(o[2] - 4.0) < 0.5,
  string.format("plane origin should be near (6, 4), got (%.3f, %.3f, %.3f)", o[1], o[2], o[3]))
print("ok: curve + endpoint → plane normal to curve at end")
bearcad.quit()
