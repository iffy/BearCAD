-- #960: when a picker takes edges and not faces, clicking a face means "all of that face's
-- edges". Before, an edges-only picker simply refused a face and the click did nothing, with
-- nothing on screen to say why.
--
-- The 3D Chamfer tool's Edges picker is the case: selecting a body face from the Elements pane
-- should fill it with that face's four edges, not leave it empty.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("corner", "frt")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

bearcad.ui.tool("chamfer")
bearcad.ui.wait(5)
local edges = bearcad.ui.picker("Edges")
assert(edges, "the Chamfer tool should show an Edges picker")
assert(#edges.items == 0, "starting empty, got " .. #edges.items)
assert(edges.accepts[1] == "edge", "it takes edges, got " .. tostring(edges.accepts[1]))

-- Click the body's top cap. It's a face, which the picker can't hold — so its edges go in.
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(5)
local picked = #bearcad.ui.picker("Edges").items
assert(picked >= 3,
  "a face click should fill the picker with that face's edges, got " .. picked)

print("ok: clicking a face fills an edges picker with the face's edges")
bearcad.quit()
