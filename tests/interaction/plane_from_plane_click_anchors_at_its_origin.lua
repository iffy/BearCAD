-- #1980: clicking an existing plane with the Plane tool anchors the new plane at that
-- plane's origin, not at the (far-off) ground point under the cursor.
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 20}, distance = 200 }
bearcad.ui.wait(5)
bearcad.ui.tool("plane")
bearcad.ui.wait(3)
-- The XZ plane faces the camera; click well away from its origin.
bearcad.ui.move_world(40, 0, 40)
bearcad.ui.wait(4)
bearcad.ui.click_world(40, 0, 40)
bearcad.ui.wait(5)
bearcad.ui.key("enter")
bearcad.ui.wait(5)
local n = bearcad.count("plane")
local p = bearcad.get{ kind = "plane", index = n - 1 }
assert(n == 4, "the click should have created a plane, have " .. n)
local o = p.anchor_origin
assert(math.abs(o[1]) < 1e-3 and math.abs(o[2]) < 1e-3 and math.abs(o[3]) < 1e-3,
  string.format("anchor should be the plane's origin, got (%.2f, %.2f, %.2f)", o[1], o[2], o[3]))
