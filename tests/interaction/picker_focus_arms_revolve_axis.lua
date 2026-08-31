-- #1485: picker_focus used to succeed and do nothing for Revolve's Axis (and most other
-- pickers). It now arms the named picker, and a name that isn't there is an error.
bearcad.new()
bearcad.rect{ width = 20, height = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)

bearcad.ui.tool("revolve")
bearcad.ui.wait(5)

assert(bearcad.ui.picker("Profile") and bearcad.ui.picker("Profile").focused, "Profile starts armed")
bearcad.ui.picker_focus("Axis")
bearcad.ui.wait(5)
assert(bearcad.ui.picker("Axis") and bearcad.ui.picker("Axis").focused, "picker_focus should arm Axis")
assert(not bearcad.ui.picker("Profile").focused, "and Profile should lose the ring")

local ok, err = pcall(bearcad.ui.picker_focus, "nope")
assert(not ok, "unknown picker must error")
assert(tostring(err):find("nope"), "unexpected error: " .. tostring(err))

print("ok: picker_focus arms Revolve Axis and errors on an unknown name")
bearcad.quit()
