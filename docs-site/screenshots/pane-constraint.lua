-- Documentation screenshot: the Constraint tool's Context pane with help mode on (#672).
--
-- Two lines selected inside a sketch, so the constraint buttons are live.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-constraint.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

local a = bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 2 }
local b = bearcad.line{ x = 0, y = 8, x1 = 20, y1 = 12 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("constraint")
bearcad.ui.wait(2)
bearcad.select(a)
bearcad.select(b, true)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
