-- Documentation screenshot: the command palette.
--
-- Opens the palette over a simple scene and captures the whole window, so the
-- searchable command list is visible.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/command-palette.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("parameters", "hide")

local sides = bearcad.rect{ x = 0, y = 0, width = 60, height = 40 }
bearcad.extrude{ profiles = sides, distance = 15 }
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)

bearcad.ui.palette("show")
bearcad.ui.wait(2)
bearcad.ui.screenshot(out, true)

bearcad.quit()
