-- Documentation screenshot: the Loft tool.
--
-- Blends a wide circle on the ground plane into a small circle 15 mm up — the classic
-- horn/funnel loft.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/loft.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.circle{ x = 0, y = 0, r = 10 }
bearcad.plane{ offset = 15 }
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.circle{ x = 3, y = 0, r = 4 }
bearcad.exit_sketch()
bearcad.loft{ circles = {0, 1}, name = "Horn" }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.set_visible({ kind = "construction_plane", index = 1 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
