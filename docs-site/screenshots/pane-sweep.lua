-- Documentation screenshot: the Sweep tool's Context pane with help mode on (#672).
--
-- A circle profile and a path drawn, with the tool active so its rows are on screen:
-- the profile and path pickers and the output choice.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-sweep.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.circle{ x = 0, y = 0, r = 4, name = "Profile" }
bearcad.line{ x = 0, y = 0, x1 = 0, y1 = 30 }
bearcad.line{ x = 0, y = 30, x1 = 25, y1 = 30 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("sweep")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
