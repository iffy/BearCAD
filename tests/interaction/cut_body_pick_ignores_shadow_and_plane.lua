-- #1219/#1220/#1221: after a slice, Select on a live fragment picks body geometry (not a
-- datum plane), and Line-tool hover arms a live face rather than a plane.
bearcad.new()
bearcad.rect{ width = 50, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.plane{ offset = 10 }
bearcad.slice{
  bodies = {0},
  cutters = {{ kind = "construction_plane", index = 3 }},
}

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- Same framing as select_body_by_face.lua; upper fragment still has a top at z = 20.
bearcad.ui.camera{ target = {25, 15, 0}, distance = 220 }
bearcad.ui.wait(10)

bearcad.clear_selection()
bearcad.ui.click_ground(25, 15)
bearcad.ui.wait(8)
local sel = bearcad.selection()
assert(#sel >= 1, "clicking a cut fragment should select something, got nothing")
local kind = sel[1].kind
assert(kind ~= "construction_plane" and kind ~= "plane",
  "datum plane must not win over the cut body, got " .. tostring(kind))
assert(kind == "body" or kind == "body_edge" or kind == "body_vertex",
  "Select over a cut body should pick body geometry, got " .. tostring(kind))

bearcad.ui.tool("line")
bearcad.ui.wait(5)
bearcad.ui.move_ground(25, 15)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h, "hovering a cut-body face with the Line tool should highlight something")
assert(h.kind ~= "construction_plane" and h.kind ~= "plane",
  "Line tool must not arm the datum plane over a cut body, got " .. tostring(h.kind))

print("ok: cut-body pick ignores shadow faces and datum planes")
bearcad.quit()
