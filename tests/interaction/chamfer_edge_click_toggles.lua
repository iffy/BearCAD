-- #1504: 3D Chamfer clicks toggle. A second plain click on the same face drops its edges;
-- it does not replace-with-the-same-set (the old restart-on-plain-click rule).
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
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
assert(bearcad.picker("Edges"), "the Chamfer tool should show an Edges picker")
assert(#bearcad.picker("Edges").items == 0, "starting empty")

-- Top cap: a face click fills the picker with that face's edges (#960).
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(5)
local first = #bearcad.picker("Edges").items
assert(first >= 3, "a face click should fill the picker, got " .. first)

-- Same face again: toggle removes those edges.
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(5)
local second = #bearcad.picker("Edges").items
assert(second == 0,
  "re-clicking the face should toggle its edges out, got " .. second)

print("ok: 3D Chamfer clicks toggle edges")
bearcad.quit()
