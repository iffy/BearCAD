-- Documentation screenshot: the Constraint tool's Context pane with help mode on (#672).
--
-- Two lines selected inside a sketch, so the constraint buttons are live.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-constraint.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 2 }
bearcad.line{ x = 0, y = 8, x1 = 20, y1 = 12 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("constraint")
bearcad.ui.wait(2)
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 1 }, true)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
