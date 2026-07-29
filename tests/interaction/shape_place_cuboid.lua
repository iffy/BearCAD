-- #912: the Shape tool places a cuboid with three clicks — one ground corner, the
-- opposite corner, then the height — and a typed size beats the cursor. B then cycles to
-- the cylinder, placed by centre + radius + height.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.tool("shape")
bearcad.ui.wait(4)

-- Click 1: the anchor corner on the ground. Click 2: the opposite corner, 40 x 20 away.
bearcad.ui.click_ground(-20, -10)
bearcad.ui.wait(4)
bearcad.ui.click_ground(20, 10)
bearcad.ui.wait(4)
-- The height phase takes a typed size, then Enter commits.
bearcad.ui.type("12")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 1,
  "the three clicks should land one body, got " .. bearcad.count("body"))
local stats = bearcad.body_stats(0)
local size = {
  stats.bbox.max[1] - stats.bbox.min[1],
  stats.bbox.max[2] - stats.bbox.min[2],
  stats.bbox.max[3] - stats.bbox.min[3],
}
assert(math.abs(size[1] - 40) < 1.5, "40 mm wide, got " .. size[1])
assert(math.abs(size[2] - 20) < 1.5, "20 mm deep, got " .. size[2])
assert(math.abs(size[3] - 12) < 0.1, "the typed 12 mm height, got " .. size[3])

-- B cycles the shape: back to the tool, once for the cylinder.
bearcad.ui.tool("shape")
bearcad.ui.wait(3)
bearcad.ui.key("b")
bearcad.ui.wait(3)
-- Centre, then a click 15 mm out for the radius, then a typed height. Away from the
-- datum planes, which are pickable anchors of their own.
bearcad.ui.click_ground(80, 40)
bearcad.ui.wait(4)
bearcad.ui.click_ground(95, 40)
bearcad.ui.wait(4)
bearcad.ui.type("10")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 2,
  "the cylinder should be the second body, got " .. bearcad.count("body"))
stats = bearcad.body_stats(1)
local diameter = stats.bbox.max[1] - stats.bbox.min[1]
assert(math.abs(diameter - 30) < 1.5, "30 mm across, got " .. diameter)
assert(math.abs((stats.bbox.max[3] - stats.bbox.min[3]) - 10) < 0.1,
  "the typed 10 mm height, got " .. (stats.bbox.max[3] - stats.bbox.min[3]))

-- A sphere takes two clicks, and lands on whatever it's clicked on: here the cuboid's
-- top face, 12 mm up, so the ball rests on it.
bearcad.ui.tool("shape")
bearcad.ui.wait(3)
-- The tool remembers the cylinder, so one press reaches the sphere.
bearcad.ui.key("b")
bearcad.ui.wait(3)
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(4)
bearcad.ui.type("6")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 3,
  "the sphere should be the third body, got " .. bearcad.count("body"))
stats = bearcad.body_stats(2)
assert(math.abs(stats.bbox.min[3] - 12) < 0.2,
  "the sphere rests on the cuboid's 12 mm top face, got " .. stats.bbox.min[3])
assert(math.abs((stats.bbox.max[3] - stats.bbox.min[3]) - 12) < 0.2,
  "and is 2 x 6 mm across, got " .. (stats.bbox.max[3] - stats.bbox.min[3]))

print("ok: the shape tool places cuboids, cylinders and spheres")
bearcad.quit()
