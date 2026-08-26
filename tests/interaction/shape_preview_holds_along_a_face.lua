-- #1748: the Shape cuboid preview (and a click that freezes it) must not flop
-- its in-plane axis as the pointer moves along one face. A cuboid's +Y wall
-- used to answer two frames (analytic first-edge vs mesh plane_u_axis).
bearcad.new()
bearcad.cuboid{ width = 40, depth = 30, height = 50 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Look from +Y at the +Y wall (y = 15).
bearcad.ui.view("back")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 25}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.snapping(false)

local function place_on_plus_y(x, z)
  bearcad.ui.tool("shape")
  bearcad.ui.wait(3)
  bearcad.ui.click_world(x, 15, z)
  bearcad.ui.wait(4)
  bearcad.ui.click_world(x + 8, 15, z + 8)
  bearcad.ui.wait(4)
  bearcad.ui.type("6")
  bearcad.ui.wait(3)
  bearcad.ui.key("enter")
  bearcad.ui.wait(8)
end

place_on_plus_y(-8, 16)
place_on_plus_y(4, 28)

assert(bearcad.count("shape") == 3,
  "base cuboid plus two placed on the wall, got " .. bearcad.count("shape"))

local u
for i = 1, 2 do
  local s = bearcad.get{ kind = "shape", index = i }
  assert(s, "shape " .. i)
  assert(math.abs(s.normal[2]) > 0.9,
    "sits on the +Y wall, normal y=" .. tostring(s.normal[2]))
  if not u then
    u = s.u_axis
  else
    local dot = u[1] * s.u_axis[1] + u[2] * s.u_axis[2] + u[3] * s.u_axis[3]
    assert(dot > 0.99,
      string.format("u_axis flopped: first=(%.3f,%.3f,%.3f) later=(%.3f,%.3f,%.3f)",
        u[1], u[2], u[3], s.u_axis[1], s.u_axis[2], s.u_axis[3]))
  end
  -- World-axis convention, not the polygon first-edge (−X).
  assert(s.u_axis[1] > 0.9, "width along +X, got " .. s.u_axis[1])
end

print("ok: cuboids placed along a face share one orientation")
bearcad.quit()
