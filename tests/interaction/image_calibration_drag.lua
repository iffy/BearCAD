-- #1547: selecting only an image with the select tool enters calibration mode —
-- the default top-middle → bottom-middle line is already there, and dragging an
-- endpoint moves it without rescaling the image.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- 410×1144 px at 1 px = 1 mm; default endpoints sit at (0, ±572).
bearcad.ui.camera{ target = {0, 0, 0}, distance = 2800 }
bearcad.ui.wait(5)

bearcad.select{ kind = "image", index = 0 }
bearcad.ui.wait(5)

local img = bearcad.get{ kind = "image", index = 0 }
assert(img.from and img.to, "selecting the image should expose the calibration line")
assert(math.abs(img.from[1]) < 1 and math.abs(img.from[2] - 572) < 2,
  string.format("default top-middle, got (%.1f, %.1f)", img.from[1], img.from[2]))
assert(math.abs(img.to[1]) < 1 and math.abs(img.to[2] + 572) < 2,
  string.format("default bottom-middle, got (%.1f, %.1f)", img.to[1], img.to[2]))
local height_before = img.height

-- Drag the top point off to the right, still in the image plane.
bearcad.ui.drag_ground(0, 572, 80, 500)
bearcad.ui.wait(10)

img = bearcad.get{ kind = "image", index = 0 }
assert(math.abs(img.height - height_before) < 1e-3,
  "dragging a calibration point must not rescale, height became " .. img.height)
assert(math.abs(img.from[1] - 80) < 8 and math.abs(img.from[2] - 500) < 8,
  string.format("top point should follow the drag, got (%.1f, %.1f)", img.from[1], img.from[2]))

print("ok: image calibration line appears on select and endpoints drag")
bearcad.quit()
