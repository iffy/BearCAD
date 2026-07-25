-- Documentation screenshot: the Offset tool's Context pane with help mode on (#672).
--
-- A sketch open with a shape to offset, the tool active: the entities picker, the
-- distance, and the construction toggle.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-offset.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.circle{ x = 0, y = 0, r = 10 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("offset")
bearcad.ui.wait(2)
bearcad.ui.click_ground(0, 10)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
