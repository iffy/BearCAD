-- #1565: clicking a face with Chamfer picks every adjacent edge of that side.
-- A rectangle with one sketch-corner cut off has five top-cap edges; the picker
-- must take all five, not a pair of opposites.
bearcad.new()
bearcad.rect{ x = 0, y = 0, width = 40, height = 40 }
bearcad.chamfer_vertex{
  point = { kind = "line", index = 1, endpoint = "end" },
  distance = 5,
}
bearcad.extrude{ polygon = {0, 4, 6, 5, 3}, distance = 10 }
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

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("chamfer")
bearcad.ui.wait(5)
assert(picker("Edges"), "the Chamfer tool should show an Edges picker")
assert(#picker("Edges").items == 0, "starting empty")

-- Top cap of the cutoff box. click_ground at the face centre.
bearcad.ui.click_ground(18, 18)
bearcad.ui.wait(5)
local picked = #picker("Edges").items
assert(picked == 5,
  "a face click should pick all five top edges, got " .. picked)

print("ok: clicking the cutoff-box top face picks all five edges")
bearcad.quit()
