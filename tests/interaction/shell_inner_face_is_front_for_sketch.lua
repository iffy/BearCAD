-- Interaction regression (#1173): starting a sketch on the inner floor of a hollowed cuboid
-- must pick that floor, not the outer bottom face buried a wall-thickness behind it.
--
-- Looking straight down into an open-top shell, the cursor sits inside both the inner floor
-- and the outer bottom; the face nearest the camera (the inner floor) wins.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
bearcad.shell{
  bodies = {0},
  faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
  thickness = "4"
}
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("rectangle")
bearcad.ui.wait(5)

bearcad.ui.view("top")
bearcad.ui.wait(5)
-- Cuboid is centred on the origin: footprint in x,y ≈ ±20, ±15. Inner floor is at z = 4.
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(8)

bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(8)

local h = bearcad.hovered()
assert(h, "hovering the shell floor should highlight a face")
assert(h.kind == "face", "expected a face hover, got " .. tostring(h.kind))
assert(h.label, "hovered face should report a label, got nil")
-- Outer bottom labels as "Primitive 0 bottom face"; the front surface is the mesh inner floor.
assert(not h.label:find("bottom"),
  "must not highlight the outer bottom behind the floor, got " .. h.label)

bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(12)
assert(bearcad.count("sketch") >= 1,
  "click should open a sketch on the front (inner) face")

print("ok: shell inner floor is the front face for sketch-on-face")
bearcad.quit()
