-- Documentation screenshot: the Fillet tool.
--
-- Extrudes an 80 x 50 x 20 mm box and rounds its four vertical edges, then
-- captures the result from a fixed corner view. The rounded edges render as a
-- faceted mesh, so this works without the OCCT kernel (a --no-default-features
-- build) and is deterministic (SPEC §8).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a GPU, or CI Linux with the software Vulkan driver); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/fillet.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

local sides = bearcad.rect{ x = 0, y = 0, width = 80, height = 50, name = "Base" }
bearcad.extrude{ profiles = sides, distance = 20, name = "Block" }

-- Round all four vertical edges of the box in one operation (edges 0-3 of face 0).
-- One call per edge would give four operations each rounding the *same* sharp box, and
-- the overlapping outputs would render as an unfilleted block (#672).
bearcad.fillet{
  body = 0,
  edges = {
    { kind = "vertical", face = 0, edge = 0 },
    { kind = "vertical", face = 0, edge = 1 },
    { kind = "vertical", face = 0, edge = 2 },
    { kind = "vertical", face = 0, edge = 3 },
  },
  radius = 8,
}

bearcad.exit_sketch()
-- Hide the ground plane's display quad; it reads as a stray tan patch behind the body.
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
-- The source sketch too: its rectangle sits outside the rounded body and reads as a stray outline.
bearcad.set_visible({ kind = "sketch", index = 0 }, false)
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
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
