-- Interaction regression (#1022): the McMaster-Carr catalog window builds a real webview
-- as a child of the app's window, and closing it takes the view down again. A webview that
-- fails to build closes the window and says why in the status, so a silent status is the
-- assertion: the platform view was actually created.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(5)

bearcad.ui.mcmaster("show", "91290A115")
bearcad.ui.wait(60)
local opened = bearcad.status()
assert(not opened:find("could not open"),
  "the catalog window should have built its webview, got: " .. opened)
assert(opened:find("opened"), "the catalog window should report itself open, got: " .. opened)

-- Closing drops the webview with the window; re-opening builds a fresh one, which is what
-- would break if the first were leaked.
bearcad.ui.mcmaster("hide")
bearcad.ui.wait(10)
assert(bearcad.status():find("closed"),
  "closing should report itself, got: " .. bearcad.status())

bearcad.ui.mcmaster("show")
bearcad.ui.wait(60)
assert(not bearcad.status():find("could not open"),
  "re-opening should build a fresh webview, got: " .. bearcad.status())
bearcad.ui.mcmaster("hide")
bearcad.ui.wait(10)

print("ok: the McMaster-Carr catalog window opens, closes, and re-opens")
bearcad.quit()
