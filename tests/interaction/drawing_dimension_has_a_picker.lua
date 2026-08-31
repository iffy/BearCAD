-- #1714: the drawing Dimension tool has a real ElementPicker in the Context pane,
-- taking projected edges and corners the way the modelling Dimension tool takes
-- scene geometry.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("dimension")
bearcad.ui.wait(5)
local dim = picker("Selection")
assert(dim, "the drawing Dimension tool should register a Selection picker")
assert(dim.focused, "and it should be armed")
assert(dim.limit == 2, "it takes an edge or two points, got limit " .. tostring(dim.limit))
local takes = {}
for _, k in ipairs(dim.accepts) do takes[k] = true end
assert(takes["edge"] and takes["vertex"],
  "it takes projected edges and corners")
assert(not takes["body"] and not takes["projection"],
  "and nothing else from the model or the page")

print("ok: drawing Dimension has an element picker")
bearcad.quit()
