-- Documentation screenshot: the technical-drawing editor (#309).
--
-- Builds a plate with a hole, opens a drawing of it, and places front + top
-- views on the page (dimensions are on by default, #299). Captures the WHOLE
-- WINDOW (the `true` second arg) so the white sheet and its view cards are
-- visible as the user sees them.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/drawing.png"

bearcad.new()
bearcad.rect{ width = 60, height = 35, name = "Plate" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 12, name = "Block" }

local d = bearcad.drawing{ name = "Plate" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.3, y = 0.62 }
bearcad.drawing_move_view{ drawing = d, view = 1, x = 0.7, y = 0.35 }

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(3)
bearcad.ui.screenshot(out, true)

bearcad.quit()
