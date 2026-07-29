-- #894: the Joint tool commits its armed picks on Enter — the same keyboard path every
-- other tool ends with — and Esc drops an in-progress joint instead of committing it.
bearcad.new()
bearcad.rect{ width = 10, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

-- Arm the tool with both parts mated in place, then Enter commits it.
bearcad.begin_joint{
  a = 0, b = 1, kind = "slider",
  from = { body = 1, vertex = {40, 0, 0} },
  to   = { body = 0, vertex = {0, 0, 0} },
}
bearcad.ui.wait(3)
assert(bearcad.count("joint") == 0, "begin_joint must not commit")
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(bearcad.count("joint") == 1,
  "Enter should commit the armed joint, status: " .. bearcad.status())

-- Arm another and Esc it away: nothing further lands.
bearcad.begin_joint{ a = 0, b = 1, kind = "rigid" }
bearcad.ui.wait(3)
bearcad.ui.key("Escape")
bearcad.ui.wait(3)
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(bearcad.count("joint") == 1,
  "Esc should have dropped the second joint, status: " .. bearcad.status())

print("ok: Enter commits the armed joint; Esc drops it")
bearcad.quit()
