-- #1612: Enter in the Context pane's Real length field applies image calibration.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)

local img = bearcad.get{ kind = "image", index = 0 }
assert(math.abs(img.height - 1144) < 2, "unexpected default height " .. img.height)

bearcad.ui.focus_calibrate()
bearcad.ui.wait(4)
bearcad.ui.type("50")
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)

img = bearcad.get{ kind = "image", index = 0 }
assert(math.abs(img.length - 50) < 0.05,
  "Enter in Real length should apply, span is " .. tostring(img.length))
assert(math.abs(img.height - 50) < 0.05,
  "image should rescale to 50 mm tall, height is " .. tostring(img.height))

print("ok: Enter in Real length applies calibration")
bearcad.quit()
