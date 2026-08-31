-- #1413/#1414: Free Move's three rotation gizmo handles float on deterministic, non-overlapping
-- references around the body, and rotating one ring swings the others' handles along with the
-- moving object's preview. The gizmos() introspection exposes each handle's world position so
-- the spread and the follow can be asserted headlessly.
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
bearcad.ui.camera{ target = {10, 10, 0}, distance = 320 }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)
bearcad.ui.begin_move{ bodies = {0} }
bearcad.ui.wait(5)

-- Collect the three rotation gizmos and their world-space handle positions.
local rot = {}
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name:find("^move_r") then
    assert(g.position ~= nil, g.name .. " should expose its handle position")
    rot[g.name] = g.position
  end
end
assert(rot.move_rx and rot.move_ry and rot.move_rz,
  "Free Move should show three rotation gizmos, got " .. (next(rot) and "some" or "none"))

-- #1413: the handles spread around the body — starting positions don't overlap.
local function len(a) return math.sqrt(a.x*a.x + a.y*a.y + a.z*a.z) end
local function dist(a, b)
  return len{ x = a.x - b.x, y = a.y - b.y, z = a.z - b.z }
end
assert(dist(rot.move_rx, rot.move_ry) > 0.1, "move_rx and move_ry handles overlap")
assert(dist(rot.move_rx, rot.move_rz) > 0.1, "move_rx and move_rz handles overlap")
assert(dist(rot.move_ry, rot.move_rz) > 0.1, "move_ry and move_rz handles overlap")

-- #1414: turning one ring (here -30° about Z) swings the other handles with the preview, so
-- the X-ring and Y-ring handles change where they sit.
local rx_before = rot.move_rx
bearcad.ui.set_gizmo{ name = "move_rz", value = -30 }   -- degrees (#1657)
bearcad.ui.wait(8)
for _, g in ipairs(bearcad.ui.gizmos()) do if g.name == "move_rx" then rot.move_rx = g.position end end
assert(dist(rot.move_rx, rx_before) > 0.1,
  "move_rx handle should follow the Z-ring turn that rotated the preview")
assert(not (bearcad.status() or ""):find("error", 1, true),
  "no error while the rotation gizmos follow the preview")

print("ok: rotation gizmo handles float spread-out and follow the preview")
bearcad.quit()
