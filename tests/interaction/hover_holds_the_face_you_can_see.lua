-- Interaction regression (#987): the hover thrashed frame to frame between the face under the
-- cursor and a hidden face behind it.
--
-- The crowd of pickable things was deduped through a `HashMap`, whose iteration order is
-- randomly seeded per instance, and the sort by screen distance is stable — so the faces the
-- cursor sits *inside* (all of them at distance 0) came back in a different order on every
-- single call. The normal pick takes the first, so it kept changing its mind. The crowd is
-- ordered nearest-the-camera-first now, and that order is total: only the face you can see is
-- reachable by an ordinary hover, and the buried one needs the exploder.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- The reported case: the Rectangle tool outside a sketch, picking the face to sketch on.
bearcad.ui.tool("rectangle")
bearcad.ui.wait(5)
-- Looking straight down at the top cap: the cursor is inside the top face (z = 20) and the
-- bottom face (z = 0) at once, which is the tie the crowd could not settle.
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 15, 0}, distance = 260 }
bearcad.ui.wait(5)

bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(6)

local first = bearcad.hovered()
assert(first, "hovering the body's cap should highlight something")
assert(first.kind == "face", "and it should be a face, got " .. tostring(first.kind))
assert(first.label, "a hovered face should report which face it is")

-- Hold still for many frames. The hover must not change its mind even once.
for i = 1, 25 do
  bearcad.ui.wait(2)
  local h = bearcad.hovered()
  assert(h and h.label == first.label,
    "the hover must hold still, but frame " .. i .. " changed from "
      .. first.label .. " to " .. tostring(h and h.label))
end

-- And what it settled on is the face you can actually see — the top cap, not the bottom one
-- hidden under the solid.
assert(first.label:find("top") or first.label:find("cap"),
  "the hover should take the cap facing the camera, got " .. first.label)
assert(not first.label:find("bottom"),
  "never the face hidden behind the solid, got " .. first.label)

print("ok: the hover holds still on the face facing the camera")
bearcad.quit()
