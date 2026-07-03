-- Documentation screenshot: the Fillet tool.
--
-- Extrudes an 80 x 50 x 20 mm box and rounds its four vertical edges, then
-- captures the result from a fixed corner view. The rounded edges render as a
-- faceted mesh, so this works without the OCCT kernel (a --no-default-features
-- build) and is deterministic (SPEC §8).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/fillet.png"

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 80, height = 50, name = "Base" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 20, name = "Block" }

-- Round each of the four vertical edges of the box (edges 0-3 of face 0).
for edge = 0, 3 do
  bearcad.fillet_edge{
    extrusion = 0,
    edge = { kind = "vertical", face = 0, edge = edge },
    radius = 8,
  }
end

bearcad.exit_sketch()
-- Hide the ground plane's display quad; it reads as a stray tan patch behind the body.
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
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
