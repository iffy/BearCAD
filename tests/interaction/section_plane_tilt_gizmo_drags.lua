-- #1765: the cutting plane's tilt rings grab at the handles the viewport draws.
-- Before the fix the grab target was mirrored to the far side of the ring, so
-- pulling the visible Turn handle did nothing.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- An oblique view: straight down the U ring is edge-on and its drag ray never
-- crosses the ring's plane, which is exactly the view the bug was reported from.
bearcad.ui.camera{ target = {30, 30, 10}, distance = 320, yaw = 30, pitch = 55 }
bearcad.ui.wait(5)

bearcad.cross_section{ name = "Cut" }
bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

-- Click the top face of the block; its centroid anchors the cutting plane.
bearcad.ui.click_ground(30, 30)
bearcad.ui.wait(5)

local function gizmo(name)
  for _, g in ipairs(bearcad.gizmos()) do
    if g.name == name then return g end
  end
end

local tilt = gizmo("tilt_u")
assert(tilt, "a face pick shows the tilt gizmos")
assert(tilt.position, "tilt_u exposes its drawn handle position so scripts can grab it")

-- The handle sits 25 mm out from the ring centre (30, 30, 20) along +Y at Turn 0.
local hx, hy, hz = tilt.position.x, tilt.position.y, tilt.position.z
local dr = math.sqrt((hx - 30)^2 + (hy - 30)^2 + (hz - 20)^2)
assert(math.abs(dr - 25) < 1,
  string.format("handle sits on the 25 mm ring, got %.2f", dr))

-- Rotate the drawn handle 60° about the ring's U axis (world X): drag_world in
-- world space, like a person pulling the handle around the ring.
local c, s = math.cos(math.rad(-60)), math.sin(math.rad(-60))
local ty, tz = 25 * c, 25 * s
bearcad.ui.drag_world(hx, hy, hz, hx, 30 + ty, 20 + tz)
bearcad.ui.wait(8)

local after = gizmo("tilt_u")
assert(after, "tilt_u still live after the drag")
local deg = after.value -- rotation gizmos read back in degrees (#1657)
assert(math.abs(math.abs(deg) - 60) < 5,
  string.format("dragging the drawn Turn handle 60° should turn ~60°, got %.2f", deg))
assert(math.abs(gizmo("tilt_v").value) < 1e-3,
  "dragging the Turn handle must not disturb Tilt")

print("ok: cutting plane tilt gizmo grabs at its drawn handle")
bearcad.quit()
