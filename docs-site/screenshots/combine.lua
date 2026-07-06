-- Documentation screenshot: the Combine tool.
--
-- Cuts one overlapping box out of another, leaving a notched result body — the two
-- inputs live on as shadow bodies in the Elements pane.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/combine.png"

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 30, height = 20, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 12, name = "Block" }

bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 18, y = 6, width = 24, height = 8, name = "Bite" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 20, name = "Bite" }

bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Notched block" }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
