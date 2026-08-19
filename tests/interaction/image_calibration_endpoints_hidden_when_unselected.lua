-- #1586: calibration-line endpoints are not hoverable or selectable while the
-- tracing image is not selected. Clicking one takes the image itself.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
-- Park the endpoints on the image, well clear of the world axes and the
-- 5..105 datum-plane quads, so a miss would select the image (or nothing)
-- rather than the Y axis.
bearcad.calibration_point{ image = 0, index = 0, x = -80, y = -200 }
bearcad.calibration_point{ image = 0, index = 1, x = -80, y = -400 }
bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {-80, -200, 0}, distance = 2800 }
bearcad.ui.wait(10)

bearcad.ui.move_ground(-80, -200)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(not h or h.kind ~= "point",
  "an unselected image's calibration endpoint must not hover as a point, got "
    .. (h and h.kind or "nothing"))
assert(h and h.kind == "image",
  "hovering the endpoint should light the image, got "
    .. (h and h.kind or "nothing"))

bearcad.ui.click_ground(-80, -200)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "image",
  "clicking the endpoint should select the image, got "
    .. (#sel > 0 and sel[1].kind or "nothing"))

print("ok: unselected image calibration endpoints are neither hovered nor selected")
bearcad.quit()
