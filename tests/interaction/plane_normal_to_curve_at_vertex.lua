-- #474: the Plane tool anchored on a curve's vertex makes a plane normal to the
-- curve at that point (its normal = the curve's tangent there).
bearcad.new()
-- A curve rising from (0,0) toward (20,10); tangent at the start points at its
-- near handle (0,8) => straight +Y in sketch/world terms.
bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 10, bezier = { { 0, 8 }, { 12, 10 } } }
bearcad.exit_sketch()
bearcad.ui.tool("construction_plane")
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Click the curve's start vertex to anchor, then Enter commits at offset 0.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(8)

local plane = bearcad.get{ kind = "construction_plane", index = 1 }
assert(plane, "a plane should have been committed")
local n = plane.normal
-- Curve tangent at the vertex is +Y (toward the (0,8) handle); outward normal is -Y —
-- either sign is the same plane.
assert(math.abs(n[1]) < 1e-3 and math.abs(math.abs(n[2]) - 1.0) < 1e-3 and math.abs(n[3]) < 1e-3,
  string.format("plane normal should be ±Y (curve tangent), got (%.3f, %.3f, %.3f)", n[1], n[2], n[3]))
print("ok: plane normal follows the curve tangent at the vertex")
bearcad.quit()
