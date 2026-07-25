-- Documentation screenshot: the Loft tool's Context pane with help mode on (#672).
--
-- Two circle sections on stacked planes, with the tool active so its rows are on
-- screen: the Sections picker and the output choice.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-loft.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.circle{ x = 0, y = 0, r = 12, name = "Base" }
bearcad.exit_sketch()
bearcad.plane{ offset = 20 }
bearcad.begin_sketch("construction_plane", 1)
bearcad.circle{ x = 0, y = 0, r = 6, name = "Top" }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("loft")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
