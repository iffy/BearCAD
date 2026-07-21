-- #473: tangency is explicit joint state. Clicking the vertex (selected) or a handle
-- toggles it; dragging a handle at a tangent joint mirrors the partner handle.
bearcad.new()
bearcad.line{ x = -20, y = 0, x1 = 0, y1 = 0, bezier = { { -15, -5 }, { -5, 5 } } }
bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 0, bezier = { { 5, -5 }, { 15, 5 } } }
bearcad.select{ kind = "line", index = 0, ["end"] = "end" }
bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
bearcad.add_geometric_constraint("coincident")
bearcad.clear_selection()
bearcad.ui.tool("select")
-- Hide the side panes: under CI's WM-less Xvfb the window can't maximize, and with
-- all three panes open the 3D viewport is too narrow for the ground-coordinate
-- clicks below to land inside it.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("top")
-- Auto-zoom would reframe (animated) when the handle drag pushes the curve past the
-- fitted view, moving the camera under the queued synthetic clicks.
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Click the shared vertex once to select it, again to toggle tangency ON.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
assert(bearcad.status():find("Tangent: on"), "vertex click should toggle on, got: " .. bearcad.status())

-- Drag line0's near-vertex handle somewhere new: the partner must mirror (stay
-- collinear through the vertex, opposite side).
local m0 = bearcad.get{ kind = "line", index = 0 }.bezier[2]
bearcad.ui.drag_ground(m0[1], m0[2], -8, 2)
bearcad.ui.wait(20)
local moved = bearcad.get{ kind = "line", index = 0 }.bezier[2]
local mx, my = moved[1], moved[2]
local h = bearcad.get{ kind = "line", index = 1 }.bezier[1]
local hx, hy = h[1], h[2]
assert(mx < 0 and hx > 0, string.format("handles must straddle the vertex, got (%.2f,%.2f)/(%.2f,%.2f)", mx, my, hx, hy))
local cross = hx * my - hy * mx
assert(math.abs(cross) < 0.02 * math.sqrt(hx*hx+hy*hy) * math.sqrt(mx*mx+my*my),
  string.format("partner (%.3f, %.3f) should mirror the dragged handle (%.3f, %.3f)", hx, hy, mx, my))

-- Click the handle: toggles tangency OFF.
local mx2 = bearcad.get{ kind = "line", index = 0 }.bezier[2]
bearcad.ui.click_ground(mx2[1], mx2[2])
bearcad.ui.wait(8)
assert(bearcad.status():find("Tangent: off"), "handle click should toggle off, got: " .. bearcad.status())

-- Now dragging that handle must leave the partner alone.
local before = bearcad.get{ kind = "line", index = 1 }.bezier[1]
local m2 = bearcad.get{ kind = "line", index = 0 }.bezier[2]
bearcad.ui.drag_ground(m2[1], m2[2], -4, 6)
bearcad.ui.wait(20)
local after = bearcad.get{ kind = "line", index = 1 }.bezier[1]
assert(math.abs(before[1] - after[1]) < 1e-3 and math.abs(before[2] - after[2]) < 1e-3,
  "independent joint's partner must not move")

-- Click the handle again: tangency back ON.
local m3 = bearcad.get{ kind = "line", index = 0 }.bezier[2]
bearcad.ui.click_ground(m3[1], m3[2])
bearcad.ui.wait(8)
assert(bearcad.status():find("Tangent: on"), "handle click should toggle back on, got: " .. bearcad.status())

print("ok: tangent toggle + tangent-preserving drag")
bearcad.quit()
