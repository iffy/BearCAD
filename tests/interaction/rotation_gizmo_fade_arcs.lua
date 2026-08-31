-- #1405: Free Move's rotation gizmos draw fading arcs instead of full circles, with the
-- handle floating on a reference and a direction arrow each side. Drive a Free Move rotation
-- (rings + a live turn) through the viewport so the fade arcs and the yellow sweep all render
-- without error, and the turn stays signed (a -30° isn't wrapped to 330°).
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.camera{ target = {10, 10, 0}, distance = 320 }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
bearcad.ui.wait(5)
bearcad.ui.begin_move{ bodies = {0} }
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)

-- The three world-axis rotation gizmos are present with the fade-arc dial.
local seen_r = 0
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name:find("^move_r") then seen_r = seen_r + 1 end
end
assert(seen_r == 3, "Free Move arms three rotation gizmos, got " .. seen_r)

-- Set a -30° turn about z and let the fade arcs, sweep and handle render a few frames.
bearcad.ui.set_gizmo{ name = "move_rz", value = -30 }   -- degrees (#1657)
bearcad.ui.wait(8)
bearcad.ui.screenshot("/tmp/rotation_gizmo_fade_arcs.png", true)
bearcad.ui.wait(4)
assert(not (bearcad.status() or ""):find("error", 1, true),
  "no error while rendering the fading-arc rotation gizmos with a turn")

-- The signed angle is preserved (negative, not wrapped to 330°).
local rz
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name == "move_rz" then rz = g.value end
end
assert(type(rz) == "number" and rz < 0, "move_rz should stay signed negative, got " .. tostring(rz))

print("ok: rotation gizmos draw signed fading arcs instead of full circles")
bearcad.quit()