-- Documentation screenshot: the Sketch tool's Context pane with help mode on (#672).
--
-- A body on screen so the face picker has something to point at; the pane shows the
-- selection picker and the default units.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-sketch.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

local sides = bearcad.rect{ x = 0, y = 0, width = 10, height = 8 }
bearcad.extrude{ profiles = sides, distance = 6 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("sketch")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
