-- Documentation screenshot: a tracing image imported onto the ground plane.
--
-- Imports the committed sample drawing (docs-site/screenshots/assets/
-- tracing-sample.png), calibrates the red reference bar (its drawn span is
-- 200 px = 200 mm at the 1 px = 1 mm import seed) to a real 50 mm, and
-- captures the result with sketch geometry drawn over it, showing the
-- calibrated image scaled down and the traced lines on top.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/tracing.png"

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Import the sample drawing onto the ground plane (1 px = 1 mm seed).
bearcad.import_image{ path = "docs-site/screenshots/assets/tracing-sample.png" }

-- Calibrate: the red bar spans x -100..100 at plane y -120 at import scale (the
-- image is centered on the plane origin); declare its real length to be 50 mm.
bearcad.calibrate_image{ image = 0, from = { -100, -120 }, to = { 100, -120 }, length = 50 }

-- Trace part of the plate outline over the (now rescaled) image: after the x0.25
-- calibration about the bar midpoint, the plate spans x -45..45, y -105..-57.5.
bearcad.line{ x = -45, y = -105, x1 = 45, y1 = -105 }
bearcad.line{ x = 45, y = -105, x1 = 45, y1 = -57.5 }
bearcad.exit_sketch()

-- Hide the ground plane's display quad; the image itself reads as the surface.
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Park the cursor-independent Dimension tool for a deterministic capture.
bearcad.ui.tool("dimension")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
