-- #1948: a hinge follows the pointer. The turn is read where the joint actually turns —
-- the cursor ray cast onto the plane through the joint origin perpendicular to its axis —
-- instead of as a screen angle about the projected pivot, which only matched the real
-- motion when the axis pointed at the camera. The angle is also unwrapped against where
-- the drag already was, so crossing atan2's branch cut never jumps a whole turn and a
-- drag pushed past a limit parks there instead of snapping to the far end.
bearcad.new()
bearcad.add_parameter("swing", "0")
local jamb = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {20, 1.5, 0} }
local door = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {-20, 1.5, 0} }
bearcad.joint{ a = jamb, b = door, kind = "revolute", frame_axis = { axis = "z" },
               position = "swing" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("iso")
bearcad.ui.zoom_fit()
bearcad.ui.tool("select")
bearcad.ui.wait(6)

-- Grab a point on the leaf that lies in the joint's rotation plane (z = 0) and drop it on
-- a target at the same radius: the leaf should turn by exactly the angle the pointer
-- swept about the pin.
local gx, gy = -20, 1.5
local radius = math.sqrt(gx * gx + gy * gy)
local grabbed = math.deg(math.atan(gy, gx))
for _, target in ipairs({ 90, 45, -60 }) do
  bearcad.set_parameter("swing", "0")
  bearcad.ui.wait(3)
  local tx = radius * math.cos(math.rad(target))
  local ty = radius * math.sin(math.rad(target))
  bearcad.ui.drag_world(gx, gy, 0, tx, ty, 0)
  bearcad.ui.wait(3)
  local want = target - grabbed
  while want > 180 do want = want - 360 end
  while want < -180 do want = want + 360 end
  local got = bearcad.parameter_value("swing")
  assert(math.abs(got - want) < 3,
         string.format("drag to %d deg: the leaf should land at %.1f, got %.1f",
                       target, want, got))
end

-- Limits: a drag pushed well past the open stop parks at it rather than wrapping round to
-- the closed one. Reversing the sign of the turn like that is the "snaps back" of #1948.
bearcad.new()
bearcad.add_parameter("swing", "20")
local a = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {20, 1.5, 0} }
local b = bearcad.cuboid{ width = 40, depth = 3, height = 40, at = {-20, 1.5, 0} }
bearcad.joint{ a = a, b = b, kind = "revolute", frame_axis = { axis = "z" },
               position = "swing", turn_min = 0, turn_max = 110 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("iso")
bearcad.ui.zoom_fit()
bearcad.ui.tool("select")
bearcad.ui.wait(6)
-- Sweep the grab point 150 degrees round the pin — far past the 110 degree stop. The
-- grab point is where (-20, 1.5) sits once the leaf is 20 degrees open.
bearcad.ui.drag_world(-19.31, -5.43, 0, 19.44, -4.95, 0)
bearcad.ui.wait(4)
local pushed = bearcad.parameter_value("swing")
assert(pushed > 100 and pushed <= 110,
       "over-driving the hinge must hold it at the open limit, got " .. pushed)
