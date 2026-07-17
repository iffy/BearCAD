-- Documentation screenshot: the Chamfer tool in a sketch (2D).
--
-- A rectangle profile with one corner cut flat by a sketch chamfer, seen from the top
-- in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/chamfer-sketch.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 40, height = 30, name = "Profile" }
-- Cut the top-right corner (line 1's end = line 2's start) 10 mm flat.
bearcad.chamfer_vertex{ point = { kind = "line", index = 1, ["end"] = "end" }, distance = 10 }

bearcad.clear_selection()
bearcad.ui.tool("dimension")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
