-- #1587: the Move tool picks a tracing image by clicking its quad, then Free-mode
-- translation gizmos slide it on its host plane.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.ui.tool("move")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- 410×1144 px at 1 px = 1 mm, centered on the origin. Click in the -X/-Y quadrant,
-- well clear of the 5..105 datum-plane quads and the world axes.
bearcad.ui.camera{ target = {-80, -200, 0}, distance = 2800 }
bearcad.ui.wait(10)

bearcad.ui.click_ground(-80, -200)
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end
local bodies = picker("Bodies")
assert(bodies and #bodies.items == 1 and bodies.items[1].kind == "image",
  "clicking the image should add it to the Move set, got " ..
  tostring(bodies and bodies.items[1] and bodies.items[1].kind))

-- Image-only moves have no snap points, so the tool should be in Free mode with gizmos.
bearcad.ui.wait(5)
local gizmos = {}
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name:find("^move_") then gizmos[g.name] = g end
end
assert(gizmos.move_x and gizmos.move_x.position,
  "Free Move should arm a translation gizmo on the image")

local before = bearcad.get{ kind = "image", index = 0 }
bearcad.ui.set_gizmo{ name = "move_x", value = 25 }
bearcad.ui.wait(3)
bearcad.ui.key("Enter")
bearcad.ui.wait(8)

local after = bearcad.get{ kind = "image", index = 0 }
assert(math.abs((after.origin_x - before.origin_x) - 25) < 0.2,
  string.format("image should slide +25 mm in x, origin %.3f → %.3f",
    before.origin_x, after.origin_x))
assert(math.abs(after.origin_y - before.origin_y) < 0.2,
  "in-plane y should stay put, got " .. after.origin_y)

print("ok: move tool picks a tracing image and Free translation slides it")
bearcad.quit()
