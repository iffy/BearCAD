-- Documentation screenshot: the Sweep tool.
--
-- Sweeps a circular profile along an L-shaped path (up, then a curve over to the
-- side) into a bent tube, so the path-following is obvious.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/sweep.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Profile: a circle on the ground plane.
bearcad.circle{ x = 0, y = 0, r = 5, name = "Profile" }
bearcad.exit_sketch()

-- Path: on a vertical plane through the origin — straight up 20 mm, then a curve
-- bending over to the side.
bearcad.plane{ origin = { 0, 0, 0 }, normal = { 0, 1, 0 }, name = "Path plane" }
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.line{ x = 0, y = 0, x1 = 0, y1 = 20 }
bearcad.line{ x = 0, y = 20, x1 = 25, y1 = 38, bezier = { { 0, 30 }, { 14, 38 } } }
bearcad.exit_sketch()

bearcad.sweep{ circle = 0, path = { 0, 1 }, name = "Tube" }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 1 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
