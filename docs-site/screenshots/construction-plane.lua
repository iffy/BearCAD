-- Documentation screenshot: the Construction Plane tool.
--
-- A block on the ground with a construction plane hovering 35 mm above it, holding a
-- circle sketch — the plane's translucent quad makes the offset visible.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/construction-plane.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 60, height = 40, name = "Base" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 15, name = "Block" }
bearcad.exit_sketch()

-- A construction plane 35 mm above the ground, with a circle sketched on it.
bearcad.plane{ offset = 35, name = "Section plane" }
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.circle{ x = 30, y = 20, r = 12 }
bearcad.exit_sketch()

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
