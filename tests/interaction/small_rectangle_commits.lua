-- Interaction regression (#1852): the drag-drawn floor was half a millimetre, so ordinary
-- small detail on a zoomed-in face was refused with "Rectangle too small". Anything above a
-- hundredth of a millimetre is a shape someone meant to draw.
bearcad.new()
bearcad.begin_sketch{ kind = "construction_plane", index = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- Zoomed right in, so a 0.3 x 0.2 mm drag is still a long way across the screen.
bearcad.ui.camera{ target = {0, 0, 0}, distance = 3 }
bearcad.ui.wait(8)
bearcad.ui.tool("rectangle")
bearcad.ui.wait(5)

bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
bearcad.ui.move_ground(0.3, 0.2)
bearcad.ui.wait(6)
bearcad.ui.click_ground(0.3, 0.2)
bearcad.ui.wait(8)

assert(bearcad.count("line") == 4,
  "a 0.3 x 0.2 mm rectangle should commit its four lines, got " .. bearcad.count("line")
  .. " (status: " .. bearcad.status() .. ")")
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
assert(math.abs(math.abs(x1 - x0) - 0.3) < 0.05,
  string.format("bottom edge should be 0.3 mm, got %.4f", math.abs(x1 - x0)))

print("ok: a sub-millimetre rectangle commits")
bearcad.quit()
