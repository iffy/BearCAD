-- Documentation screenshot: the circle tool's Context pane with help mode on (#672).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-circle.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.begin_sketch("construction_plane", 0)

bearcad.ui.view("top")
bearcad.ui.wait(2)

bearcad.ui.tool("circle")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
