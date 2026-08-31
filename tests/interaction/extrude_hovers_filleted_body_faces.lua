-- #1325: Extrude must hover and pick every flat face of a live body — including faces
-- that have no analytic cap/side identity after a fillet (or any other consume).
--
-- A fillet shadows the extrusion body. The remaining flats are mesh faces of the
-- EdgeTreated output. Extrude's hover/click used to accept only analytic ExtrudeCap/
-- ExtrudeSide/… so those flats never highlighted and a click started nothing.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.fillet_edge{
  extrusion = 0,
  edges = {
    { kind = "cap", face = 0, edge = 0, top = true },
    { kind = "cap", face = 0, edge = 1, top = true },
    { kind = "cap", face = 0, edge = 2, top = true },
    { kind = "cap", face = 0, edge = 3, top = true },
  },
  radius = 5,
}
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- Remaining top after a 5 mm fillet on a 40×30 cap is 30×20, still centred at (20, 15).
bearcad.ui.camera{ target = {20, 15, 0}, distance = 260 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)

bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(8)
local h = bearcad.ui.hovered()
assert(h, "hovering a remaining flat on the filleted body should highlight a face")
assert(h.kind == "face",
  "Extrude should hover the flat face, got " .. tostring(h.kind))
-- #1871: identity is the body + centroid, not a dummy index=0 + display label.
assert(h.body ~= nil and h.face ~= nil and h.normal ~= nil,
  "hovered face should carry a stable body/centroid/normal identity")

bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(10)
local faces = picker("Faces")
assert(faces and #faces.items == 1,
  "clicking the remaining top should start an extrusion, Faces has "
    .. tostring(faces and #faces.items))

print("ok: Extrude hovers and picks a remaining flat on a filleted body")
bearcad.quit()
