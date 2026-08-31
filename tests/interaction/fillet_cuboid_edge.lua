-- #1329: a Shape-tool cuboid's edges are filletable. Hover highlighted them but a click
-- left the Edges picker empty, because treatable edges only listed extrusions.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 50, height = 22 }
bearcad.ui.tool("fillet")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("corner", "frt")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 11}, distance = 200 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local edges = picker("Edges")
assert(edges, "the Fillet tool should show an Edges picker")
assert(#edges.items == 0, "starting empty, got " .. #edges.items)

-- The cuboid's top +Y edge sits at y = 25, z = 22. A click on that edge (or the top
-- face that owns it) must land at least one treatable edge in the picker.
bearcad.ui.click_ground(0, 25)
bearcad.ui.wait(5)
local picked = #picker("Edges").items
assert(picked >= 1,
  "clicking a cuboid edge should fill the Fillet picker, got " .. picked)

print("ok: fillet tool picks a cuboid primitive edge")
bearcad.quit()
