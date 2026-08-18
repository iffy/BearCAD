-- Documentation screenshot: the Extrude tool's Context pane with help mode on (#672).
--
-- A sketched rectangle with its face picked, so the pane is in the state you meet it in:
-- the face picker filled, and the distance and "up to" inputs it brings with it. The
-- Output modes swap what the extrusion does, not which fields are on offer, so one shot
-- covers the tool.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-extrude.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ width = 40, height = 25, name = "Plate" }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("extrude")
bearcad.ui.wait(3)
-- Click the profile to fill the face picker (the tool's first step).
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
