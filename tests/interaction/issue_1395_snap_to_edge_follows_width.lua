-- Regression (#1395): snapping a sketch point to a base cuboid's edge while drawing on its
-- cap must pin it to the edge LINE (a point-on-FaceEdge coincident), not to one specific
-- corner (a point-on-FaceVertex coincident). The line tracks the face as it reshapes, so the
-- snapped corner follows when the base's width changes. (A corner coincident is fixed to a
-- single corner — here the cap's top edge corner is what the user snapped to, and the sketch
-- must ride that edge up as width grows.)
bearcad.new()
bearcad.parameter("add", "W", "40")
bearcad.rect{ x = 0, y = 0, width = 100, height = "W", name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                      lines = {0, 1, 2, 3}, top = true }
-- The cap's sketch-local frame matches the ground frame: it spans (0,0)..(100,W).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {50, 25, 20}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.tool("rectangle")
bearcad.ui.wait(3)

-- Draw a small rect on the cap. First corner clicks near the cap's TOP edge (deliberately a
-- few mm off, as anyone actually clicks) and snaps onto that edge line.
bearcad.ui.click_ground(6, 38)
bearcad.ui.wait(6)
bearcad.ui.move_ground(30, 20)
bearcad.ui.wait(3)
bearcad.ui.click_ground(30, 20)
bearcad.ui.wait(5)
bearcad.exit_sketch()

local function top_y()
  -- The rect's two top corners sit on the cap's top edge; read the max corner y.
  local _, _, ay, by = 0
  local maxy = -1e9
  for i = 4, 7 do
    local x0, y0, x1, y1 = bearcad.line_endpoints(i)
    maxy = math.max(maxy, y0, y1)
  end
  return maxy
end

-- Before the change the rect's top edge rides the cap's top edge at y = 40.
local before = top_y()
assert(math.abs(before - 40) < 0.5, string.format("rect top edge starts on cap at y=40, got %.1f", before))

-- Grow the base's width: the cap's top edge rises to y = 60 and the snapped corner must follow.
bearcad.parameter("value", 0, "60")
bearcad.ui.wait(4)
local after = top_y()
assert(math.abs(after - 60) < 0.5,
  string.format("snapped corner should follow the cap's top edge to y=60, got %.1f", after))

print("ok: a corner snapped to the cuboid's edge follows when the width changes")
bearcad.quit()