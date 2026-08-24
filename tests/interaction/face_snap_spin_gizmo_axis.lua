-- #1426/#1427/#1428: Face Snap's spin gizmo is yellow (same as the connector), sits on a
-- world axis at 0°, and the A→A connector is a bezier even with no extra turn. Scripted
-- through begin_move so the live preview (gizmo + connector) is what we inspect.
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
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {45, 15, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("move")
bearcad.ui.wait(5)

-- 0° turn: the connector must still curve (leave each face along its normal).
bearcad.begin_move{
  bodies = {1},
  from = { body = 1, on_face = {70, 10, 10}, normal = {0, 0, 1} },
  to = { body = 0, on_face = {15, 15, 10}, normal = {0, 0, 1} },
}
bearcad.ui.wait(8)

local spin
for _, g in ipairs(bearcad.gizmos()) do
  if g.name == "move_spin" then spin = g end
end
assert(spin, "Face Snap exposes a move_spin gizmo")
assert(math.abs(spin.value) < 1e-4, "resting turn is 0")
assert(spin.position, "the handle has a world position")
-- Landing face is +Z at (15, 15, 10); the 0° handle is in XY and on a world axis.
local dx = spin.position.x - 15
local dy = spin.position.y - 15
local dz = spin.position.z - 10
assert(math.abs(dz) < 1.0, "handle stays in the landing plane, dz=" .. dz)
assert(math.abs(dx) < 1.0 or math.abs(dy) < 1.0,
  "0° handle sits on a world axis, offset=(" .. dx .. "," .. dy .. "," .. dz .. ")")

-- The gizmo is scriptable: a 20° turn writes through.
bearcad.set_gizmo{ name = "move_spin", value = 20 }   -- degrees (#1657)
bearcad.ui.wait(6)
local after
for _, g in ipairs(bearcad.gizmos()) do
  if g.name == "move_spin" then after = g.value end
end
assert(type(after) == "number" and math.abs(after - 20) < 1e-3,
  "set_gizmo writes the Face Snap turn, got " .. tostring(after))

bearcad.ui.screenshot("/tmp/face_snap_spin_gizmo_axis.png", true)
assert(not (bearcad.status() or ""):find("error", 1, true),
  "no error while rendering the axis-aligned Face Snap spin gizmo")

print("ok: Face Snap spin gizmo is axis-aligned and scriptable")
bearcad.quit()
