-- Documentation screenshot: the Repeat tool.
--
-- A block repeated four times along X with a parametric gap.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/repeat.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 8, height = 14, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10, name = "Block" }

bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "count_gap", count = 4, spacing = 6, name = "Row" }

-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
