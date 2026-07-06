-- Documentation screenshot: the Repeat tool.
--
-- A block repeated four times along X with a parametric gap.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/repeat.png"

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 8, height = 14, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10, name = "Block" }

bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "count_gap", count = 4, spacing = 6, name = "Row" }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
