-- Regression (#1360/#1361/#1362): the Face Snap move's spin gizmo, the moving/fixed face
-- colors, and the curved start→finish connector all render without error when the move is
-- armed with a non-zero turn. Drives the live preview (`begin_move` never commits) so the
-- gizmo, the yellow arc, and the curved A→A connector all get drawn this frame.
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

-- The moving block's top face centre onto the slab's top face centre, with a 20° spin.
bearcad.ui.begin_move{
  bodies = {1},
  spin = "20",
  from = { body = 1, on_face = {70, 10, 10}, normal = {0, 0, 1} },
  to = { body = 0, on_face = {15, 15, 10}, normal = {0, 0, 1} },
}
bearcad.ui.wait(8)

-- The spin gizmo should be live: Face Snap's two face rows are up, and the turn is set.
local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end
assert(picker("Moving face"), "Face Snap leads with a moving face row")
assert(picker("Fixed face"), "and the fixed face it lands on")

-- A couple of frames with the gizmo / arc / curved connector on screen.
bearcad.ui.wait(6)
bearcad.ui.screenshot("/tmp/move_face_spin_gizmo.png", true)
print("status: " .. tostring(bearcad.status()))
assert(not (bearcad.status() or ""):find("error", 1, true),
  "no error while rendering the Face Snap spin gizmo")

print("ok: Face Snap spin gizmo renders with a turn")
bearcad.quit()