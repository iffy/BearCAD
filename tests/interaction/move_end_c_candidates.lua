-- #914: with the Move tool's A and B pairs set, end point C can only ride a circle — the
-- tool offers four quarter-turn spots on it, pickable even in mid-air where no geometry is.
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.rect{ x = 60, y = 0, width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Move the first block onto the second, aimed along +X, with the C pair pending.
bearcad.ui.begin_move{
  bodies = {0},
  from = {
    { body = 0, vertex = {0, 0, 0} },
    { body = 0, vertex = {20, 0, 0} },
    { body = 0, vertex = {0, 20, 0} },
  },
  to = {
    { body = 1, vertex = {60, 0, 0} },
    { body = 1, vertex = {80, 0, 0} },
  },
}
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {40, 0, 0}, distance = 260 }
bearcad.ui.wait(8)

-- (60, -20) is one of the four spots: half a turn from where C sits now, and on no
-- geometry at all, so only the candidate path can pick it.
bearcad.ui.click_ground(60, -20)
bearcad.ui.wait(5)
assert(bearcad.status():find("end C point"),
  "the mid-air spot should land as end point C, got: " .. bearcad.status())

bearcad.ui.key("enter")
bearcad.ui.wait(8)
-- The move lands as a moved copy of the block, so a third body appears.
assert(bearcad.count("body") == 3,
  "the move should commit a moved body, got " .. bearcad.count("body") ..
  " (status: " .. bearcad.status() .. ")")

print("ok: end point C offers the four spots on its circle")
bearcad.quit()
