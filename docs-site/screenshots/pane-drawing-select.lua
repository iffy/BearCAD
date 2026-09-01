-- Documentation screenshot: the drawing workbench's Select tool Context pane with help mode on (#672).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-drawing-select.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
local sides = bearcad.rect{ width = 60, height = 35, name = "Plate" }
bearcad.extrude{ profiles = sides, distance = 12, name = "Block" }
local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.4, y = 0.5 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)
bearcad.ui.wait(3)

bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
