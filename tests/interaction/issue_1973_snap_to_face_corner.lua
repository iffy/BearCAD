-- Regression (#1973): drawing on an extrusion cap, a click on one of the cap's corners
-- snaps to that corner (not just onto an edge line), and the point follows the corner
-- when the base reshapes.
bearcad.new()
bearcad.add_parameter("W", "40")
bearcad.rect{ x = 0, y = 0, width = 100, height = "W", name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                      lines = {0, 1, 2, 3}, top = true }
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

-- First corner: slightly off the cap's top-left corner (0, W).
bearcad.ui.click_sketch(1, 39)
bearcad.ui.wait(6)
bearcad.ui.move_sketch(30, 20)
bearcad.ui.wait(3)
bearcad.ui.click_sketch(30, 20)
bearcad.ui.wait(5)
bearcad.exit_sketch()

local function nearest_corner(tx, ty)
  local best, bx, by = 1e9, 0, 0
  for i = 4, 7 do
    local x0, y0, x1, y1 = bearcad.line_endpoints(i)
    for _, p in ipairs({{x0, y0}, {x1, y1}}) do
      local d = math.abs(p[1] - tx) + math.abs(p[2] - ty)
      if d < best then best, bx, by = d, p[1], p[2] end
    end
  end
  return bx, by
end

local x, y = nearest_corner(0, 40)
assert(math.abs(x) < 1e-3 and math.abs(y - 40) < 1e-3,
  string.format("first corner should snap exactly to the cap corner (0,40), got (%.3f,%.3f)", x, y))

bearcad.set_parameter("W", "60")
bearcad.ui.wait(4)
x, y = nearest_corner(0, 60)
assert(math.abs(x) < 1e-3 and math.abs(y - 60) < 1e-3,
  string.format("snapped corner should follow the cap corner to (0,60), got (%.3f,%.3f)", x, y))

print("ok: a click on a cap corner snaps to it and follows it")
bearcad.quit()
