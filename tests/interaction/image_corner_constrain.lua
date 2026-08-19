-- #1589: the Constraint tool picks an image's box corner and can pin it to a
-- sketch vertex. (A default import's centre sits on the origin, so the origin
-- itself is not a distinct click target.)
bearcad.new()
bearcad.import_image("rectangle_preview.png")
bearcad.line{ x = 50, y = 50, x1 = 80, y1 = 50 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.tool("constraint")
bearcad.clear_selection()
bearcad.ui.wait(3)

-- Zoom onto the image's bottom-left (−205, −572) like origin_click frames (0, 0).
bearcad.ui.camera{ target = {-205, -572, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.click_ground(-205, -572)
bearcad.ui.wait(8)
local sel = bearcad.selection()
local got = (#sel > 0 and sel[1].kind or "nothing")
assert(#sel == 1 and sel[1].kind == "point",
  "clicking the image's bottom-left should select that box point, got " .. got)

-- Then onto the line start.
bearcad.ui.camera{ target = {50, 50, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.click_ground(50, 50, { shift = true })
bearcad.ui.wait(8)
sel = bearcad.selection()
assert(#sel == 2, "corner + line start should both be selected, got " .. #sel)

bearcad.add_geometric_constraint("coincident")
bearcad.ui.wait(5)

local img = bearcad.get{ kind = "image", index = 0 }
assert(math.abs(img.origin_x - 50) < 8 and math.abs(img.origin_y - 50) < 8,
  string.format("bottom-left pinned to (50, 50), got (%.1f, %.1f)", img.origin_x, img.origin_y))
assert(math.abs(img.width - 410) < 1e-3 and math.abs(img.height - 1144) < 1e-3,
  "pinning a corner must not rescale")
local l = bearcad.get{ kind = "line", index = 0 }
assert(math.abs(l.x0 - 50) < 1e-2 and math.abs(l.y0 - 50) < 1e-2,
  "the line stays put; the image is the mover")

print("ok: image corner constrains to a sketch vertex")
bearcad.quit()
