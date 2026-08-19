-- #1561 / #1563: the Select tool picks a tracing image by clicking its quad, and
-- selecting it shows the calibration line (top-middle → bottom-middle by default).
bearcad.new()
bearcad.import_image("rectangle_preview.png")
-- Import selects the image (#1582); clear so this test is about clicking the quad.
bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- 410×1144 px at 1 px = 1 mm, centered on the origin. Click in the -X/-Y quadrant,
-- well clear of the 5..105 datum-plane quads and the world axes.
bearcad.ui.camera{ target = {-80, -200, 0}, distance = 2800 }
-- Give the first frame after the camera move time to land before picking.
bearcad.ui.wait(10)

bearcad.ui.click_ground(-80, -200)
bearcad.ui.wait(5)

local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "image",
  "clicking the image should select it, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

local img = bearcad.get{ kind = "image", index = 0 }
assert(img.from and img.to, "selecting the image should expose the calibration line")
assert(math.abs(img.from[1]) < 1 and math.abs(img.from[2] - 572) < 2,
  string.format("default top-middle, got (%.1f, %.1f)", img.from[1], img.from[2]))
assert(math.abs(img.to[1]) < 1 and math.abs(img.to[2] + 572) < 2,
  string.format("default bottom-middle, got (%.1f, %.1f)", img.to[1], img.to[2]))

print("ok: select tool picks a tracing image and shows its calibration line")
bearcad.quit()
