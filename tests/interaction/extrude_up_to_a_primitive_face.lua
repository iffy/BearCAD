-- Interaction regression (#1492): Extrude's "Up to" refused every face that has no
-- extrusion behind it — a Shape primitive's face, a moved/shelled/boolean body's mesh
-- face. Clicking one resolved to no target at all, so the pick silently disarmed and
-- focus snapped back to the Faces picker with nothing picked.
--
-- Here the target is a cuboid primitive's bottom face, named through the Selection
-- Exploder because it is buried behind the solid (the same route as #988's test).
bearcad.new()
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
-- The top-face sketch origin is a corner; (20, 15) is the cuboid centre, which
-- `click_ground(0, 0)` hits from a top view through the world origin.
bearcad.circle{ x = 20, y = 15, r = 6 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 10}, distance = 260 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)
-- Pick the circle on the cuboid's top face as the profile.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
assert(#picker("Faces").items == 1, "the circle should be the profile")
assert(#picker("Up to").items == 0, "and nothing is the target yet")

bearcad.ui.picker_focus("Up to")
bearcad.ui.wait(6)
assert(picker("Up to").focused, "the Up to picker should be armed")

bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(4)
bearcad.ui.key("space")
bearcad.ui.wait(8)
local leaves = bearcad.ui.exploder()
assert(#leaves > 0, "Space should open the fan while Up to is armed, got " .. #leaves .. " leaves")

local bottom
for _, l in ipairs(leaves) do
  if l.label and l.label:find("bottom") and l.x then bottom = l end
end
assert(bottom, "the fan should offer the cuboid's bottom face")

bearcad.ui.click(bottom.x, bottom.y)
bearcad.ui.wait(12)

assert(#picker("Up to").items == 1,
  "clicking the cuboid's bottom face should fill the Up to picker, got "
    .. #picker("Up to").items .. " item(s)")
assert(not picker("Up to").focused,
  "a taken pick disarms Up to; it must not bounce back with nothing picked")

-- The bore runs the full 20 mm from the top face down to the bottom one.
bearcad.ui.key("enter")
bearcad.ui.wait(12)
local ext = bearcad.get{ kind = "extrusion", index = 0 }
assert(ext, "the extrusion should commit")
assert(math.abs(math.abs(ext.distance) - 20) < 0.05,
  "the target should drive a 20 mm depth, got " .. tostring(ext.distance))

print("ok: Extrude's Up to takes a primitive's buried bottom face")
bearcad.quit()
