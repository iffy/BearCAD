-- #1432: dragging Face Snap's Turn gizmo a short way must stay a short signed angle
-- (negative when the handle moves clockwise about the landing normal), never a wrapped
-- ~299°. Top orthographic so drag_ground lands on the handle disc.
bearcad.new()
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ origin = {60, 0, 0}, normal = {0, 0, 1} }
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.camera{ target = {45, 15, 10}, distance = 320, projection = "orthographic" }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
bearcad.ui.wait(5)
bearcad.ui.begin_move{
  bodies = {1},
  from = { body = 1, on_face = {70, 10, 10}, normal = {0, 0, 1} },
  to = { body = 0, on_face = {15, 15, 10}, normal = {0, 0, 1} },
}
bearcad.ui.wait(8)

local function spin()
  for _, g in ipairs(bearcad.ui.gizmos()) do
    if g.name == "move_spin" then return g end
  end
end

local g = spin()
assert(g and g.position, "Face Snap should expose move_spin with a handle")
local hx, hy = g.position.x, g.position.y
local cx, cy = 15, 15
local dx, dy = hx - cx, hy - cy
-- Clockwise 60° about +Z: (x, y) → (x, y) rotated by −60°.
local c, s = math.cos(math.rad(-60)), math.sin(math.rad(-60))
local tx, ty = cx + dx * c - dy * s, cy + dx * s + dy * c
bearcad.ui.drag_ground(hx, hy, tx, ty)
bearcad.ui.wait(8)

local after = spin()
assert(after, "move_spin still live after the drag")
local deg = after.value   -- rotation gizmos read back in degrees (#1657)
assert(deg < 0,
  "clockwise Turn drag should be a negative angle, got " .. deg)
assert(math.abs(deg) < 180,
  "Turn must stay the short signed path, not wrap toward 299°, got " .. deg)
assert(math.abs(deg + 60) < 15,
  "a 60° clockwise drag should land near −60°, got " .. deg)

print("ok: Face Snap Turn drag stays a short signed angle")
bearcad.quit()
