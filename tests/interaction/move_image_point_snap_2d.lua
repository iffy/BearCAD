-- #1601: the 2D Point Snap moves a tracing image by its own box points — click a corner
-- of the image, then where it should land, and the image slides in its plane.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
-- 410x1144 px at 1 px = 1 mm, centred on the origin: the bottom-left corner is at
-- (-205, -572). Orthographic top so a ground click lands on the in-plane point.
bearcad.ui.camera{ target = {0, 0, 0}, distance = 3000, projection = "orthographic" }
bearcad.ui.wait(5)

bearcad.ui.tool("select")
bearcad.ui.tool("move")
bearcad.ui.wait(8)
bearcad.ui.tool_mode("point_snap")
bearcad.ui.wait(5)
assert(bearcad.ui.tool_mode() == "point_snap", "Point Snap is a mode for an image (#1601)")

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end
assert(not picker("Moving face") and not picker("Fixed face"),
  "an image move has no Face Snap rows (#1601)")
assert(not picker("Start point C") and not picker("End point C"),
  "the 2D snap stops at the B pair (#1601)")

-- Start point A: the image's own bottom-left box point.
bearcad.ui.picker_focus("Start point A")
bearcad.ui.wait(5)
bearcad.ui.click_ground(-205, -572)
bearcad.ui.wait(8)
local start_a = picker("Start point A")
assert(start_a and #start_a.items == 1,
  "clicking the image's corner should fill Start point A (#1601), got "
    .. tostring(start_a and #start_a.items))

-- End point A: the world origin.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
local end_a = picker("End point A")
assert(end_a and #end_a.items == 1,
  "clicking the origin should fill End point A, got " .. tostring(end_a and #end_a.items))

bearcad.ui.key("Enter")
bearcad.ui.wait(10)
local img = bearcad.get{ kind = "image", index = 0 }
assert(math.abs(img.origin_x) < 0.5 and math.abs(img.origin_y) < 0.5,
  string.format("the corner should land on the origin, got %.3f, %.3f",
    img.origin_x, img.origin_y))

print("ok: the 2D point snap moves an image by its own box points")
bearcad.quit()
