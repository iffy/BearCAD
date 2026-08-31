-- #913: with Snapping on, a shape's placement clicks land on body corners and edge
-- midpoints instead of the raw point under the cursor.
bearcad.new()
bearcad.rect{ width = 40, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 6 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 10, 0}, distance = 200 }
bearcad.ui.wait(5)
bearcad.ui.tool("shape")
bearcad.ui.wait(3)
-- Cylinder: cuboid → cylinder is one press.
bearcad.ui.key("b")
bearcad.ui.wait(3)

-- Click 1.5 mm off the block's (40, 20) corner: snapping lands the centre on it.
bearcad.ui.click_ground(41.5, 21.5)
bearcad.ui.wait(4)
bearcad.ui.type("5")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(4)
bearcad.ui.type("8")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 2, "one block plus the cylinder, got " .. bearcad.count("body"))
local stats = bearcad.body_stats(1)
local cx = (stats.bbox.min.x + stats.bbox.max.x) / 2
local cy = (stats.bbox.min.y + stats.bbox.max.y) / 2
assert(math.abs(cx - 40) < 0.2, "the centre snapped to x = 40, got " .. cx)
assert(math.abs(cy - 20) < 0.2, "the centre snapped to y = 20, got " .. cy)

-- With snapping off a click lands where the cursor really is. Well clear of both bodies,
-- so the ground is what it anchors on.
bearcad.ui.snapping(false)
bearcad.ui.tool("shape")
bearcad.ui.wait(3)
bearcad.ui.click_ground(41.5, -20)
bearcad.ui.wait(4)
bearcad.ui.type("5")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(4)
bearcad.ui.type("8")
bearcad.ui.wait(4)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == 3, "a third body, got " .. bearcad.count("body"))
stats = bearcad.body_stats(2)
cx = (stats.bbox.min.x + stats.bbox.max.x) / 2
assert(math.abs(cx - 41.5) < 0.3, "unsnapped, the centre stays at 41.5, got " .. cx)

print("ok: shape placement snaps to a body corner, and doesn't when snapping is off")
bearcad.quit()
