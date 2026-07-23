-- Documentation screenshot: the Extrude tool.
--
-- Extrudes an 80 x 50 mm rectangle 20 mm into a solid body and captures it from
-- a fixed front-top-right corner view so the 3D form is visible and the output
-- is deterministic (SPEC §8).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/extrude.png"

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ width = 80, height = 50, name = "Base" }
-- Extrude the rectangle's four lines as an explicit closed loop. (The `rect = 0`
-- shorthand builds the same body but currently wedges the screenshot render, so
-- the docs harness uses the explicit polygon form.)
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 20, name = "Block" }

bearcad.exit_sketch()
-- Hide the ground plane's display quad; it reads as a stray tan patch behind the body.
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
-- The OS cursor parks wherever the desktop left it (often mid-viewport) and would
-- hover-highlight whatever face it sits on; the Dimension tool has no pick hover,
-- keeping the capture deterministic.
bearcad.ui.tool("dimension")

bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
