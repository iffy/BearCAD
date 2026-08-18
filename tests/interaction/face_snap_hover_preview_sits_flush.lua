-- #1458: Face Snap hover over a point on the already-picked fixed face must preview
-- the moving face *against* that face — the same pose a click commits. Hovering an
-- edge midpoint used to fill a bare vertex, so the ghost only translated and sliced
-- through the target.
bearcad.new()
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ origin = {60, 0, 0}, normal = {0, 0, 1} }
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 20 }
bearcad.exit_sketch()
bearcad.clear_selection()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {45, 15, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("move")
bearcad.ui.wait(5)
bearcad.begin_move{ bodies = {1} }
bearcad.ui.wait(5)
bearcad.ui.tool_mode("face_snap")
bearcad.ui.wait(8)

-- Moving side: the block's top face, then its centre.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
-- Fixed side: the slab's top face. Leave the point unpicked — hover is the bug.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)

-- Middle of the slab's near top edge (15, 0, 10). A click here lands the block
-- upside-down on the slab; a translation-only hover drops it through the slab.
bearcad.ui.move_ground(15, 0)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h and h.kind == "body_vertex",
  "the edge midpoint should highlight, got " .. tostring(h and h.kind))

local p = bearcad.move_preview()
assert(p and p.bbox, "hovering a Face Snap point should preview the move")
assert(p.bbox.min[3] > 9.5,
  "the moving face should sit against the slab top (z = 10), not slice through, min.z="
  .. p.bbox.min[3])

print("ok: Face Snap hover preview sits the moving face against the fixed face")
bearcad.quit()
