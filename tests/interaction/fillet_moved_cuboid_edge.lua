-- #1463: a moved body's edges are filletable. The fillet tool used the cuboid's
-- un-moved analytic edges, so a click on the body you can see missed them.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 50, height = 22 }
bearcad.move_bodies{ bodies = {0}, x = 80 }
bearcad.ui.tool("fillet")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("corner", "frt")
bearcad.ui.wait(5)
-- The live body now sits at x = 80. Same top +Y edge as fillet_cuboid_edge.lua,
-- translated with the move.
bearcad.ui.camera{ target = {80, 0, 11}, distance = 200 }
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

bearcad.ui.click_ground(80, 25)
bearcad.ui.wait(5)
local picked = #picker("Edges").items
assert(picked >= 1,
  "clicking a moved cuboid's visible edge should fill the Fillet picker, got " .. picked)

print("ok: fillet tool picks a moved cuboid edge")
bearcad.quit()
