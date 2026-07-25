-- Documentation screenshot: a drawing text note's Context pane with help mode on (#672).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-drawing-text.png"

bearcad.new()
bearcad.rect{ width = 60, height = 35, name = "Plate" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 12, name = "Block" }
local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_text{ drawing = d, text = "All edges 0.5 mm", x = 0.55, y = 0.8 }

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
