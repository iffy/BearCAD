-- #1367: while a Face Snap destination pick is armed, the cursor should light up the front
-- (moving) body it is over — not fall through to the geometry behind it. The destination
-- *click* still goes through the moving body (that's #1336), but the hover shows the front
-- body itself.
bearcad.new()
-- A stationary slab and a smaller block that overhangs it, so the block has a face with
-- nothing behind it. From the top, the block's overhang is pure block.
bearcad.rect{ x = 0, y = 0, width = 40, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.rect{ x = 30, y = 10, width = 30, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("move")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 20, 0}, distance = 160 }
bearcad.ui.wait(5)

-- Start moving the block. Same geometry as #1336's click-through so the destination stage is
-- reachable, but a Face Snap (not point snap) move so the MateFace hover path is exercised.
bearcad.ui.begin_move{ bodies = {1}, from = { body = 1, vertex = {0, 0, 0} } }
bearcad.ui.tool_mode("face_snap")
bearcad.ui.wait(8)

-- The block overhangs the slab on the x=40..60 strip. With the destination picker armed the
-- moving bodies are dropped from the *click*, but the front mock body must still glow on hover.
bearcad.ui.move_ground(50, 20)
bearcad.ui.wait(8)
local h = bearcad.ui.hovered()
assert(h, "hovering the overhanging front body should highlight it, got none")
assert(h.kind == "face" or h.kind == "body_face" or h.kind == "body_vertex" or h.kind == "body",
  "the front (moving) body's face should be highlighted, got " .. tostring(h.kind))

print("ok: face snap destination hover shows the front moving body")
bearcad.quit()