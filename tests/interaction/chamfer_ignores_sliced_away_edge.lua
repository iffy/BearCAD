-- #1543: after a slice, Chamfer must highlight and pick the remaining visible
-- body edge, not the original cuboid edge that now runs through the cut.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 50, height = 22 }
bearcad.plane{ origin = {0, 0, 0}, normal = {1, 0, 0} }
bearcad.slice{
  bodies = {0},
  cutters = {{ kind = "construction_plane", index = 3 }},
}

-- Hide the x < 0 fragment so the original top +Y edge's left half is gone.
for i = 0, 8 do
  local s = bearcad.body_stats(i)
  if s and s.bbox and s.bbox.max[1] < 1 then
    bearcad.set_body_shadow{ body = i, shadow = true }
  end
end

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 11}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("chamfer")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

assert(picker("Edges"), "the Chamfer tool should show an Edges picker")
assert(#picker("Edges").items == 0, "starting empty")

-- The original cuboid occupied x = −20..20. Its uncut edges used to run through
-- this empty half; a click here must not take those vanished edges.
bearcad.ui.click_ground(-15, 0)
bearcad.ui.wait(5)
assert(#picker("Edges").items == 0,
  "clicking the sliced-away original body must not pick an edge, got "
    .. #picker("Edges").items)

-- The remaining visible top +Y edge (x > 0, y = 25) is still a real edge.
bearcad.ui.click_ground(10, 25)
bearcad.ui.wait(5)
local picked = #picker("Edges").items
assert(picked >= 1,
  "clicking the remaining visible edge should fill the Chamfer picker, got " .. picked)

print("ok: chamfer tool ignores a sliced-away original body edge")
bearcad.quit()
