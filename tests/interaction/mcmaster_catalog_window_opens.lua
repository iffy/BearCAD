-- Interaction regression (#1022): the McMaster-Carr catalog window is a second process —
-- this same binary under `bearcad mcmaster` — so opening it spawns that process and closing
-- it kills the process again. A spawn that fails closes the window and says why in the
-- status, so a status free of that is the assertion: the process really started.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(5)

bearcad.ui.mcmaster("show", "91290A115")
bearcad.ui.wait(60)
local opened = bearcad.status()
assert(not opened:find("could not"),
  "the catalog window's process should have started, got: " .. opened)
assert(opened:find("opened"), "the catalog window should report itself open, got: " .. opened)

-- Closing kills the process; re-opening starts a fresh one, which is what would break if
-- the first were leaked or left running.
bearcad.ui.mcmaster("hide")
bearcad.ui.wait(10)
assert(bearcad.status():find("closed"),
  "closing should report itself, got: " .. bearcad.status())

bearcad.ui.mcmaster("show")
bearcad.ui.wait(60)
assert(not bearcad.status():find("could not"),
  "re-opening should start a fresh process, got: " .. bearcad.status())
bearcad.ui.mcmaster("hide")
bearcad.ui.wait(10)

print("ok: the McMaster-Carr catalog window opens, closes, and re-opens")
bearcad.quit()
