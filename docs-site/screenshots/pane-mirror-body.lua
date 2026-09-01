-- Documentation screenshot: the Mirror tool's Context pane (3D) with help mode on (#672).
--
-- A body beside the YZ plane, the tool active: the mirror-plane and bodies pickers and
-- the output choice.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-mirror-body.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

local sides = bearcad.rect{ x = 8, y = 0, width = 14, height = 10 }
bearcad.extrude{ profiles = sides, distance = 6 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("mirror")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
