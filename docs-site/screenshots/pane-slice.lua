-- Documentation screenshot: the Slice tool's Context pane with help mode on (#672).
--
-- A body and an offset construction plane as its cutter, tool active: the bodies and
-- cutters pickers and the infinite-cut toggle.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-slice.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ x = 0, y = 0, width = 16, height = 12 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ offset = 5 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("slice")
bearcad.ui.wait(2)
bearcad.ui.click_ground(8, 6)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
