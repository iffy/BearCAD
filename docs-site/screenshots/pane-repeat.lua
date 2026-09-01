-- Documentation screenshot: the Repeat tool's Context pane with help mode on (#672).
--
-- A body picked and an axis set, so every row shows: the bodies picker, the axis, the
-- count/gap/distance trio with its locks, and the distance-to picker.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-repeat.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

local sides = bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
bearcad.extrude{ profiles = sides, distance = 6 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("repeat")
bearcad.ui.wait(2)
-- Pick the body into the tool's set (its face is at the origin).
bearcad.ui.click_ground(5, 5)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
