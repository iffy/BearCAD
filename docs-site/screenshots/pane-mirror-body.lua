-- Documentation screenshot: the Mirror tool's Context pane (3D) with help mode on (#672).
--
-- A body beside the YZ plane, the tool active: the mirror-plane and bodies pickers and
-- the output choice.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-mirror-body.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ x = 8, y = 0, width = 14, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 6 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("mirror")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
