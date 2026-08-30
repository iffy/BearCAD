-- Interaction regression (#1849): a loupe magnifies detail so it can be *read*, but there was
-- no way to dimension what it shows — the Dimension tool only ever saw the card's own edges.
-- Clicking a magnified edge inside a loupe now puts the dimension on the loupe, labelled with
-- the edge's real length.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 60, depth = 40, height = 30 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.wait(8)

-- Sit the detail circle on the middle of a long horizontal edge, so its magnified copy runs
-- across the middle of the big circle and the centre of that circle is a hit on it.
local lines = bearcad.drawing_view_lines{ drawing = d, view = 0 }
local edge
for _, l in ipairs(lines) do
  if math.abs(l.y1 - l.y2) < 1e-3 and math.abs(l.x2 - l.x1) > 30 then edge = l end
end
assert(edge, "the front view has a long horizontal edge to dimension")
local mx, my = (edge.x1 + edge.x2) / 2, edge.y1
bearcad.drawing_loupe{ drawing = d, view = 0, at = {mx, my}, radius = 6,
                       to = {mx + 45, my - 45}, to_radius = 26 }
bearcad.clear_selection()
bearcad.ui.wait(10)

local l = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(#l.dimensions == 0, "a new loupe carries no dimensions")

local vp = bearcad.ui.viewport()
local rect = assert(bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true })
local cx, cy = rect.x + rect.w / 2 - vp.x, rect.y + rect.h / 2 - vp.y

bearcad.ui.tool("dimension")
bearcad.ui.wait(5)
bearcad.ui.move(cx, cy)
bearcad.ui.wait(5)
bearcad.ui.click(cx, cy)
bearcad.ui.wait(10)

local dims = bearcad.drawing_loupes{ drawing = d, view = 0 }[1].dimensions
assert(#dims == 1, "clicking a magnified edge dimensions it on the loupe, got " .. #dims)
-- It went on the loupe, not on the card: the view's own dimension list is untouched.
assert(bearcad.drawing_views(d)[1].dimensions == 0,
  "the card keeps its own dimensions, got " .. bearcad.drawing_views(d)[1].dimensions)
-- The dimension names the real edge, at its real length.
local a, b = dims[1].a, dims[1].b
local length = math.sqrt((a[1]-b[1])^2 + (a[2]-b[2])^2 + (a[3]-b[3])^2)
assert(math.abs(length - 60) < 0.1,
  string.format("the dimension measures the real 60 mm edge, got %.3f", length))

-- Clicking it again takes it off.
bearcad.ui.click(cx, cy)
bearcad.ui.wait(10)
assert(#bearcad.drawing_loupes{ drawing = d, view = 0 }[1].dimensions == 0,
  "a second click hides it again")

-- Scripts set it the same way.
bearcad.drawing_loupe_dimension{ drawing = d, view = 0, index = 0, a = a, b = b }
assert(#bearcad.drawing_loupes{ drawing = d, view = 0 }[1].dimensions == 1,
  "the loupe's dimensions are scriptable")

print("ok: a loupe's magnified detail can be dimensioned")
bearcad.quit()
