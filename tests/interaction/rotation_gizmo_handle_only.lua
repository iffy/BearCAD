-- #1418: rotation gizmos grab only at their handles. From the top, the red (X) handle
-- sits on the blue (Z) ring, so a ring hit prefers the face-on blue circle and dragging
-- red turns Z. Handle-only picking turns the handle you grabbed.
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
-- Orthographic top: handle XY and ground XY share a screen point, so drag_ground
-- lands on the disc. Perspective would miss the handle sitting at z = 5.
bearcad.ui.camera{ target = {10, 10, 5}, distance = 320, projection = "orthographic" }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)
bearcad.ui.begin_move{ bodies = {0} }
bearcad.ui.wait(5)

local rot = {}
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name:find("^move_r") then rot[g.name] = g end
end
assert(rot.move_rx and rot.move_rz, "Free Move should show rotation gizmos")

local function gizmo(name)
  return bearcad.ui.gizmo(name)
end

-- A point on the Z ring at ~45°, away from every handle, must not start a turn.
local rxh = rot.move_rx.position
local rzh = rot.move_rz.position
local c = { x = (rxh.x + rzh.x) * 0.5, y = (rxh.y + rzh.y) * 0.5 }
local r = math.sqrt((rzh.x - c.x)^2 + (rzh.y - c.y)^2)
local p45x, p45y = c.x + r * 0.7071, c.y + r * 0.7071
bearcad.ui.drag_ground(p45x, p45y, p45x + 4, p45y - 4)
bearcad.ui.wait(6)
local rz_after_ring = gizmo("move_rz").value
local rx_after_ring = gizmo("move_rx").value
assert(math.abs(rz_after_ring) < 1e-3 and math.abs(rx_after_ring) < 1e-3,
  string.format("dragging the ring (not a handle) must not turn, got rx=%.4f rz=%.4f",
    rx_after_ring, rz_after_ring))

-- Drag the red (X) handle around the centre - that must turn X, not Z.
bearcad.ui.drag_ground(rxh.x, rxh.y, rxh.x + 10, rxh.y - 6)
bearcad.ui.wait(8)
local rx = gizmo("move_rx").value
local rz = gizmo("move_rz").value
assert(math.abs(rx) > 1e-3,
  "dragging the red handle should turn move_rx, got " .. tostring(rx))
assert(math.abs(rz) < 1e-3,
  "dragging the red handle must not turn the blue ring, got move_rz=" .. tostring(rz))

print("ok: rotation gizmos drag only by their handles")
bearcad.quit()
