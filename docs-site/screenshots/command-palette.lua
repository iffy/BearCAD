-- Documentation screenshot: the command palette.
--
-- Opens the palette over a simple scene and captures the whole window, so the
-- searchable command list is visible.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/command-palette.png"

bearcad.new()
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 60, height = 40 }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 15 }
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
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
