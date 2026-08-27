-- #1780/#1781: a curve in a technical drawing is drawn as tessellation facets, but those
-- facets — and their faux vertices — are rendering artifacts, not picks. On a sectioned
-- cylinder's Front view the cut edge is a curve: sweeping the whole card with the Dimension
-- tool must surface only the real corners (where the curve meets the straight edges) and
-- the whole straight edges, never the little facet lines or their midpoints.
bearcad.new()
bearcad.cylinder{ radius = 20, height = 40 }
bearcad.cross_section{}
bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 1, 0}, offset = 5, flip = true }
bearcad.edit_section_plane{ cut = 0, roll = 14 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view_section{ drawing = d, view = 0, cross_section = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
assert(vp.width > 100 and vp.height > 100, "expected a real sheet area")
local cx, cy = vp.width / 2, vp.height / 2

bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

-- Sweep the card on a grid, fanning at each stop. A corner's loupe sits on the corner
-- itself, so distinct positions are distinct corners; an edge's loupe follows the cursor,
-- so edges are counted per fan (how many stack under one spot).
local corners, max_edges, max_corner_fan = {}, 0, 0
for gx = -180, 180, 12 do
  for gy = -160, 160, 12 do
    bearcad.ui.move(cx + gx, cy + gy)
    bearcad.ui.wait(2)
    bearcad.ui.key("space")
    bearcad.ui.wait(4)
    local fan_edges, fan_corners = 0, 0
    for _, l in ipairs(bearcad.exploder()) do
      if l.x then
        if l.kind == "projected_corner" then
          fan_corners = fan_corners + 1
          corners[string.format("%d,%d", math.floor(l.x), math.floor(l.y))] = true
        elseif l.kind == "projected_edge" then
          fan_edges = fan_edges + 1
        end
      end
    end
    max_edges = math.max(max_edges, fan_edges)
    max_corner_fan = math.max(max_corner_fan, fan_corners)
    bearcad.ui.key("space")
    bearcad.ui.wait(2)
  end
end

local ncorners = 0
for _ in pairs(corners) do ncorners = ncorners + 1 end

-- The cut face's real corners (where its curved edge meets the wall silhouettes, plus the
-- rim lines' ends) and the straight edges. Before the fix the curve's facets alone added
-- dozens of faux corners, and one hover over the curve stacked a dozen sliver edges.
assert(ncorners > 0, "the real corners must still be pickable")
assert(ncorners <= 8,
  string.format("only real corners should be pickable, got %d faux+real", ncorners))
assert(max_edges > 0 and max_edges <= 4,
  string.format("a hover should offer whole edges only, got %d stacked at one spot", max_edges))
assert(max_corner_fan <= 4,
  string.format("a hover should stack only real corners, got %d at one spot", max_corner_fan))

print(string.format("ok: %d corners total, worst fan %d edges / %d corners — no facets offered",
  ncorners, max_edges, max_corner_fan))
bearcad.quit()
