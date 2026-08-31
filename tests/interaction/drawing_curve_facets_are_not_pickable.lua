-- #1780/#1781: a curve in a technical drawing is drawn as tessellation facets, but those
-- facets — and their faux vertices — are rendering artifacts, not picks. A fan over the
-- sectioned cylinder's cut curve used to stack a dozen sliver edges and facet corners;
-- now any stop offers at most the whole logical edges and the real corners. Two scan
-- lines through the viewport centre (the card sits near it) cross the curve, the straight
-- edges, and stay cheap enough for software-rendered CI.
bearcad.new()
bearcad.cylinder{ radius = 44.774, height = 80.79 }
bearcad.cross_section{}
bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 1, 0}, offset = 23.6, flip = true }
bearcad.edit_section_plane{ cut = 0, roll = 14.1 }
bearcad.drawing{}
bearcad.drawing_view{ drawing = 0, body = 0, orientation = "front" }
bearcad.drawing_view_section{ drawing = 0, view = 0, cross_section = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
local cx, cy = vp.width / 2, vp.height / 2

bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

-- Scan a vertical and a horizontal line through the centre, fanning at each stop.
local corners, max_edges, max_fan, edges_seen = {}, 0, 0, 0
local function fan_at(x, y)
  bearcad.ui.move(x, y)
  bearcad.ui.wait(2)
  bearcad.ui.key("space")
  bearcad.ui.wait(4)
  local fan_edges, fan_corners = 0, 0
  for _, l in ipairs(bearcad.ui.exploder()) do
    if l.x then
      if l.kind == "projected_corner" then
        fan_corners = fan_corners + 1
        corners[string.format("%d,%d", math.floor(l.x), math.floor(l.y))] = true
      elseif l.kind == "projected_edge" then
        fan_edges = fan_edges + 1
      end
    end
  end
  bearcad.ui.key("space")
  bearcad.ui.wait(2)
  max_edges = math.max(max_edges, fan_edges)
  max_fan = math.max(max_fan, fan_edges + fan_corners)
  if fan_edges > 0 then edges_seen = edges_seen + 1 end
end

for dy = -160, 160, 8 do
  fan_at(math.floor(cx), math.floor(cy + dy))
end
for dx = -200, 200, 8 do
  fan_at(math.floor(cx + dx), math.floor(cy))
end

local ncorners = 0
for _ in pairs(corners) do ncorners = ncorners + 1 end

-- Geometry is pickable (some stop fanned an edge), but nothing stacks: a facet pile-up
-- on the curve used to fan a dozen leaves at one spot, and dozens of distinct faux
-- corners along the scan lines.
assert(edges_seen > 0, "the straight edges must still be pickable")
assert(max_edges <= 2,
  string.format("a hover should offer whole edges only, got %d stacked at one spot", max_edges))
assert(max_fan <= 4,
  string.format("no stop should stack a facet pile, got %d leaves at one spot", max_fan))
assert(ncorners <= 8,
  string.format("only real corners should be pickable, got %d distinct along the scan", ncorners))

print(string.format("ok: worst fan %d leaves, %d distinct corners — no facets offered",
  max_fan, ncorners))
bearcad.quit()
