-- Documentation screenshot: the Extrude tool.
--
-- Extrudes an 80 x 50 mm rectangle 20 mm into a solid body and captures it from
-- a fixed front-top-right corner view so the 3D form is visible and the output
-- is deterministic (SPEC §8).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a GPU, or CI Linux with the software Vulkan driver); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/extrude.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

local sides = bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ profiles = sides, distance = 20, name = "Block" }

bearcad.exit_sketch()
-- Hide the ground plane's display quad; it reads as a stray tan patch behind the body.
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
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
