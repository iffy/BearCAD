-- Documentation screenshot: the Text tool's Context pane with help mode on (#672).
--
-- A text element selected so the editor rows show.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-text.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.text{ text = "BearCAD", x = 0, y = 0, size = 10 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("text")
bearcad.ui.wait(2)
bearcad.select{ kind = "sketch_text", index = 0 }
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
