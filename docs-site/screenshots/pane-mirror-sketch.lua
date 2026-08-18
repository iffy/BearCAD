-- Documentation screenshot: the Mirror tool's Context pane inside a sketch (#672).
--
-- A sketch open with a line to mirror across, the tool active: the mirror-line and
-- shapes pickers.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-mirror-sketch.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.line{ x = 0, y = -15, x1 = 0, y1 = 15, name = "Axis" }
bearcad.circle{ x = 10, y = 5, r = 4 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("mirror")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
