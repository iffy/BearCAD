-- #1763: after the cuboid's two base corners, height follows the cursor on
-- either side of the placement face — including behind it. The stored height
-- stays positive; the growth normal flips so the solid occupies the cursor's side.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Isometric (the default). Top view looks along the ground normal, so free-cursor
-- height is undefined.
bearcad.ui.camera{ target = {60, 50, 0}, distance = 280 }
bearcad.ui.wait(5)
bearcad.ui.snapping(false)

local function place(c1, c2, tip)
  bearcad.ui.tool("shape")
  bearcad.ui.wait(3)
  bearcad.ui.click_world(c1[1], c1[2], c1[3])
  bearcad.ui.wait(4)
  bearcad.ui.click_world(c2[1], c2[2], c2[3])
  bearcad.ui.wait(4)
  bearcad.ui.move_world(tip[1], tip[2], tip[3])
  bearcad.ui.wait(3)
  bearcad.ui.click_world(tip[1], tip[2], tip[3])
  bearcad.ui.wait(8)
end

-- On the ground (XY, +Z). Away from the YZ/XZ walls so the first two clicks
-- stay on XY. One cuboid grows up; the next grows down, behind the plane.
place({40, 40, 0}, {80, 60, 0}, {60, 50, 20})
place({40, 80, 0}, {80, 100, 0}, {60, 90, -20})

assert(bearcad.count("shape") == 2,
  "two cuboids, got " .. bearcad.count("shape"))

local up = bearcad.get{ kind = "shape", index = 0 }
assert(up.normal[3] > 0.9, "grows +Z, normal z=" .. tostring(up.normal[3]))
assert(math.abs(up.height - 20) < 1.5, "20 mm up, got " .. up.height)
local stats = bearcad.body_stats(0)
assert(stats.bbox.min[3] > -1.5,
  "up cuboid stays on the ground, min.z=" .. stats.bbox.min[3])
assert(stats.bbox.max[3] > 15,
  "and reaches the cursor, max.z=" .. stats.bbox.max[3])

local down = bearcad.get{ kind = "shape", index = 1 }
assert(down.normal[3] < -0.9,
  "grows -Z (behind the ground), normal z=" .. tostring(down.normal[3]))
assert(math.abs(down.height - 20) < 1.5, "20 mm down, got " .. down.height)
stats = bearcad.body_stats(1)
assert(stats.bbox.max[3] < 1.5,
  "down cuboid's base stays on the ground, max.z=" .. stats.bbox.max[3])
assert(stats.bbox.min[3] < -15,
  "and reaches behind it, min.z=" .. stats.bbox.min[3])

-- Same on a vertical face: the YZ plane (normal +X), matching the report.
-- Off-axis enough that a behind-the-face click still resolves a signed height.
bearcad.ui.tool("select")
bearcad.ui.wait(2)
bearcad.ui.camera{ target = {0, 40, 30}, yaw = 30, pitch = 25, distance = 250 }
bearcad.ui.wait(8)
place({0, 30, 20}, {0, 50, 40}, {-20, 40, 30})
bearcad.ui.tool("select")
bearcad.ui.wait(2)
bearcad.ui.camera{ target = {0, 80, 30}, yaw = 30, pitch = 25, distance = 250 }
bearcad.ui.wait(8)
place({0, 70, 20}, {0, 90, 40}, {20, 80, 30})

assert(bearcad.count("shape") == 4,
  "two more on YZ, got " .. bearcad.count("shape"))

local inn = bearcad.get{ kind = "shape", index = 2 }
assert(inn.normal[1] < -0.9,
  "grows -X (behind YZ), normal x=" .. tostring(inn.normal[1]))
assert(math.abs(inn.height - 20) < 1.5, "20 mm behind, got " .. inn.height)
stats = bearcad.body_stats(2)
assert(stats.bbox.max[1] < 1.5,
  "behind cuboid's base stays on YZ, max.x=" .. stats.bbox.max[1])
assert(stats.bbox.min[1] < -15,
  "and reaches behind it, min.x=" .. stats.bbox.min[1])

local out = bearcad.get{ kind = "shape", index = 3 }
assert(out.normal[1] > 0.9,
  "grows +X off YZ, normal x=" .. tostring(out.normal[1]))
assert(math.abs(out.height - 20) < 1.5, "20 mm out, got " .. out.height)
stats = bearcad.body_stats(3)
assert(stats.bbox.min[1] > -1.5,
  "out cuboid stays on YZ, min.x=" .. stats.bbox.min[1])
assert(stats.bbox.max[1] > 15,
  "and reaches the cursor, max.x=" .. stats.bbox.max[1])

print("ok: cuboid height follows the cursor on either side of the face")
bearcad.quit()
