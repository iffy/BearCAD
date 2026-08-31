-- Interaction regression (#1277): hovering the small XY floor when a bigger XZ plane also
-- covers that screen spot must highlight the front plane (XY), not the buried red-blue one.
--
-- Construction-plane picks used to keep only the first zero-screen-distance hit, and the
-- reverse-iteration order preferred later planes — so a large XZ that the cursor was "inside"
-- of won over the floor in front of it. Depth at the point under the cursor breaks the tie.
bearcad.open("tests/fixtures/issue_1277.json")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.wait(5)

-- Diagonal look at the floor, similar to the report: the enlarged XZ fills the view and the
-- small XY sits on the ground in front of it under the cursor.
bearcad.ui.camera{
  target = {55, 55, 20},
  distance = 380,
  yaw = 51.6,   -- degrees (#1657)
  pitch = -31.5,
}
bearcad.ui.wait(8)

-- Interior of the XY datum (gap 5..105), clear of the world axes.
bearcad.ui.move_ground(70, 70)
bearcad.ui.wait(8)

local h = bearcad.hovered()
assert(h, "hovering the XY floor should highlight a plane")
assert(h.kind == "plane",
  "expected a construction plane, got " .. tostring(h.kind))
assert(h.index == 0,
  "front XY plane (index 0) must win over the big XZ behind it, got index "
    .. tostring(h.index))

-- Hold still: the hover must not flip to the buried plane.
for i = 1, 10 do
  bearcad.ui.wait(2)
  local again = bearcad.hovered()
  assert(again and again.index == 0,
    "hover must hold the front plane, frame " .. i .. " got "
      .. tostring(again and again.index))
end

print("ok: front construction plane wins hover over a buried overlapping one")
bearcad.quit()
