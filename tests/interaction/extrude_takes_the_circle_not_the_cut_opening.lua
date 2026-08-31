-- Interaction regression (#1848): with a circle sketched on the inner wall of a pocket that
-- breaks out through a side, the Extrude tool took the *opening* — the analytic face the cut
-- removed, nearest the camera — instead of the circle under the cursor, and opened a
-- throwaway sketch on it. Same root cause as #1847, reached through the extrude face picker.
--
-- A 40³ cuboid with a 20 x 16 pocket 20 deep, flush with the +X wall. The pocket's back wall
-- is at x = 0; the opening it left is at x = 20, right in front of it from this camera.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 40, height = 40 }
-- The top face's frame puts local (0, 0) at world (-20, -20, 40), u along +X, v along +Y.
bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
bearcad.rect{ x = 20, y = 12, width = 20, height = 16 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = -20, body = "cut" }
bearcad.exit_sketch()
bearcad.clear_selection()

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("right")
bearcad.ui.wait(6)
bearcad.ui.camera{ target = {0, 0, 30}, distance = 200 }
bearcad.ui.wait(8)

-- A Ø10 circle on the pocket's back wall (the cut prism's side at x = 0). The sketch is
-- opened by name so the pick under test is only the Extrude tool's, and the circle is drawn
-- with the pointer so it lands on the wall's centre whatever corner the frame hangs from.
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = {0, 1, 2, 3}, edge = 3 }
bearcad.ui.tool("circle")
bearcad.ui.wait(5)
bearcad.ui.click_world(0, 0, 30)
bearcad.ui.wait(8)
bearcad.ui.move_world(0, 5, 30)
bearcad.ui.wait(6)
bearcad.ui.click_world(0, 5, 30)
bearcad.ui.wait(10)
assert(bearcad.count("circle") == 1, "the pointer should have drawn one circle, got "
  .. bearcad.count("circle"))
bearcad.exit_sketch()
bearcad.clear_selection()
local sketches = bearcad.count("sketch")

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)
bearcad.ui.move_world(0, 0, 30)
bearcad.ui.wait(8)
local h = bearcad.ui.hovered()
assert(h and h.kind == "face", "the circle should highlight as a face, got " .. tostring(h and h.kind))
assert(h.label == "Circle face 0",
  "expected the circle under the cursor, got " .. tostring(h.label))

bearcad.ui.click_world(0, 0, 30)
bearcad.ui.wait(10)
local faces = -1
for _, p in ipairs(bearcad.ui.pickers()) do
  if p.name == "Faces" then faces = #p.items end
end
assert(faces == 1, "clicking the circle should pick exactly one face, got " .. faces)
-- Taking a *body* face instead opens a throwaway sketch of its outline; the circle doesn't.
assert(bearcad.count("sketch") == sketches,
  "extruding the circle must not open a sketch on the opening, got "
  .. bearcad.count("sketch") .. " sketches")

print("ok: extrude takes the circle on the pocket wall, not the cut's opening")
bearcad.quit()
