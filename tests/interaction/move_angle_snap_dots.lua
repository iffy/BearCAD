-- #917/#918: the Move pane's Angle snap sets how far apart the rotation's candidate dots
-- sit. At 90° only the axis directions are offered; at 45° the diagonals are too.
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
-- The A pair and start B are set; end B is what the next click is for. The sphere is
-- centred on (60, 0, 0) with a radius of 20.
local function arm()
  bearcad.begin_move{
    bodies = {0},
    from   = { body = 0, vertex = {0, 0, 0} },
    to     = { body = 1, vertex = {60, 0, 0} },
    from_b = { body = 0, vertex = {20, 0, 0} },
  }
end
arm()
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {50, 0, 0}, distance = 300 }
bearcad.ui.wait(8)

-- (74.14, -14.14) is 45° round the sphere, in mid-air off every body.
bearcad.ui.angle_snap(90)
bearcad.ui.wait(3)
bearcad.ui.click_ground(74.14, -14.14)
bearcad.ui.wait(5)
assert(not bearcad.status():find("end B point"),
  "at 90 degrees the diagonal isn't offered, got: " .. bearcad.status())

bearcad.ui.angle_snap(45)
bearcad.ui.wait(3)
bearcad.ui.click_ground(74.14, -14.14)
bearcad.ui.wait(5)
assert(bearcad.status():find("end B point"),
  "at 45 degrees the diagonal is a candidate, got: " .. bearcad.status())

print("ok: the angle snap sets which rotation dots are offered")
bearcad.quit()
