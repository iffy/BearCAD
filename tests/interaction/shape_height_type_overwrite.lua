-- #1271: while the Shape tool's height rides the pointer, the Height field stays focused
-- with its value selected so typing overwrites the live number (no click needed).
-- #1274: after setting the cylinder radius, Height (not Radius) takes the keyboard.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 280 }
bearcad.ui.wait(5)
bearcad.ui.tool("shape")
bearcad.ui.wait(4)

-- Cuboid base, then move the pointer so height is live, then type multi-digit height.
bearcad.ui.click_ground(-20, -10)
bearcad.ui.wait(3)
bearcad.ui.click_ground(20, 10)
bearcad.ui.wait(3)
bearcad.ui.move_ground(20, 10)
bearcad.ui.wait(2)
bearcad.ui.move_ground(25, 15)
bearcad.ui.wait(2)
bearcad.ui.type("20")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 1, "cuboid body missing, got " .. bearcad.count("body"))
local stats = bearcad.body_stats(0)
local h = stats.bbox.max.z - stats.bbox.min.z
assert(math.abs(h - 20) < 0.15, "typed height 20 should overwrite live value, got " .. h)

-- Cylinder: centre + radius click, then type height (must not stay stuck on Radius).
bearcad.ui.tool("shape")
bearcad.ui.wait(3)
bearcad.ui.key("b")
bearcad.ui.wait(3)
bearcad.ui.click_ground(80, 40)
bearcad.ui.wait(3)
bearcad.ui.click_ground(90, 40)
bearcad.ui.wait(3)
bearcad.ui.move_ground(90, 45)
bearcad.ui.wait(2)
bearcad.ui.type("15")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 2, "cylinder body missing, got " .. bearcad.count("body"))
stats = bearcad.body_stats(1)
local ch = stats.bbox.max.z - stats.bbox.min.z
assert(math.abs(ch - 15) < 0.15, "cylinder height 15 after base click, got " .. ch)

print("ok: shape height type-overwrites while the top rides the pointer")
bearcad.quit()
