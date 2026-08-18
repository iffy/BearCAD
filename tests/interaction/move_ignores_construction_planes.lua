-- #1459: the Move tool cannot move construction planes, so it must not hover-highlight
-- them or offer them in the exploder.
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Same pose as front_plane_wins_over_buried_plane.lua: interior of the XY datum
-- (gap 5..105), clear of the 20×20 cuboid and the world axes.
bearcad.ui.camera{
  target = {55, 55, 20},
  distance = 380,
  yaw = 0.9,
  pitch = -0.55,
}
bearcad.ui.wait(8)

local function is_plane(h)
  return h and (h.kind == "construction_plane" or h.kind == "plane")
end

local function fan_kinds(x, y)
  bearcad.ui.move_ground(x, y)
  bearcad.ui.wait(3)
  bearcad.ui.key("space")
  bearcad.ui.wait(5)
  local seen = {}
  for _, leaf in ipairs(bearcad.exploder()) do
    seen[leaf.kind] = true
  end
  bearcad.ui.key("escape")
  bearcad.ui.wait(4)
  return seen
end

-- Select still takes the XY floor, so the spot is a real plane pick.
bearcad.ui.tool("select")
bearcad.ui.wait(5)
bearcad.ui.move_ground(70, 70)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(is_plane(h), "Select should hover the construction plane, got " .. tostring(h and h.kind))
local seen = fan_kinds(70, 70)
assert(seen["construction_plane"] or seen["plane"],
  "Select's fan should offer the construction plane")

-- Same spot on the Move tool: nothing to pick, so nothing to highlight or fan.
bearcad.ui.tool("move")
bearcad.ui.wait(5)
bearcad.ui.move_ground(70, 70)
bearcad.ui.wait(8)
h = bearcad.hovered()
assert(not is_plane(h),
  "Move must not hover-highlight a construction plane, got " .. tostring(h and h.kind))

seen = fan_kinds(70, 70)
assert(not seen["construction_plane"] and not seen["plane"],
  "Move's fan must not offer a construction plane")

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
end
bearcad.ui.click_ground(70, 70)
bearcad.ui.wait(8)
local bodies = picker("Bodies")
assert(bodies, "the Move tool's Bodies picker should be visible")
for _, item in ipairs(bodies.items) do
  assert(item.kind ~= "construction_plane" and item.kind ~= "plane",
    "clicking the plane must not add it to the Move set, got " .. item.kind)
end

-- Face Snap's face picker also used to take datum planes. Still no.
bearcad.ui.tool_mode("face_snap")
bearcad.ui.wait(5)
bearcad.ui.move_ground(70, 70)
bearcad.ui.wait(8)
h = bearcad.hovered()
assert(not is_plane(h),
  "Face Snap must not hover-highlight a construction plane, got " .. tostring(h and h.kind))
seen = fan_kinds(70, 70)
assert(not seen["construction_plane"] and not seen["plane"],
  "Face Snap's fan must not offer a construction plane")

print("ok: move tool ignores construction planes on hover and in the exploder")
bearcad.quit()
