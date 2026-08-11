-- #1225: right-click a projection → "Create aligned view" switches to the Aligned-view
-- tool with that card as the base. Scriptable path is the same: select the projection
-- (adding a view selects it) then arm drawing_align — the Base view picker carries it.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(5)

-- Select tool first so entering drawing_align is a real tool switch that seeds the base
-- from the lone selected projection.
bearcad.ui.tool("select")
bearcad.ui.wait(3)
bearcad.ui.tool("drawing_align")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local base = picker("Base view")
assert(base, "Aligned-view tool should register a Base view picker")
assert(#base.items == 1,
  "Create-aligned-view path should seed the base from the selected projection, got "
    .. #base.items)
assert(base.items[1].kind == "projection",
  "base should be a projection, got " .. tostring(base.items[1].kind))

print("ok: Create aligned view seeds the Aligned-view tool's base from the selected projection")
bearcad.quit()
