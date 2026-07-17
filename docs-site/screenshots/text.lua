-- Documentation screenshot: the Text tool.
--
-- A wrapped "BearCAD text tool" label selected in a sketch: the glyph outlines, the
-- dashed wrap box with its width drag handles, and the nine anchor points.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/text.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.text{ text = "The Text tool bakes real glyph outlines", x = 0, y = 0, size = 8, wrap = 80 }
bearcad.select{ kind = "sketch_text", index = 0 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
-- Frame the text itself (zoom-to-fit would frame the whole ground quad).
bearcad.ui.camera{ target = {40, -8, 0}, distance = 140 }
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
