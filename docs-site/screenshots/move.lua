-- Documentation screenshot: the Move tool.
--
-- A box moved 30 mm along X and rotated 30 degrees about Z — the original lives on as a
-- shadow body; the moved copy is a real body.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/move.png"

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 20, height = 14, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8, name = "Block" }

bearcad.move_bodies{ bodies = {0}, x = "30", axis = "z", angle = "30", name = "Shifted" }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
