-- Documentation screenshot: a tracing image's Context pane with help mode on (#672).
--
-- The repo's own preview PNG imported onto the ground plane, calibrated, and selected —
-- so the calibrate rows show.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Runs from the repo root (gen-doc-screenshots.sh cd's there).

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-tracing.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.import_image("rectangle_preview.png")
bearcad.calibrate_image{ image = 0, from = { -100, 0 }, to = { 100, 0 }, length = 50 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Import already selected the image (#1582).
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
