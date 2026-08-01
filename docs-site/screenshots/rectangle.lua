-- Documentation screenshot: the Rectangle tool.
--
-- Builds a single locked 80 x 50 mm rectangle on the ground plane and captures
-- it from a fixed top-down view so the output is deterministic (SPEC §8).
--
-- The output directory comes from $BEARCAD_SCREENSHOT_OUT (set by
-- scripts/gen-doc-screenshots.sh); it falls back to "." so the script can be
-- run by hand for testing. The PNG is only written where a real GPU frame
-- renders (a display, or CI Linux with xvfb + software Vulkan); in a
-- display-less environment the capture never resolves and --timeout force-exits
-- without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/rectangle.png"

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ width = 80, height = 50, name = "Plate" }

-- A clean background (#667): the ground plane's quad and the grid both away. The
-- sketch drawn on that plane stays visible.
-- Hide the three datum planes a new document opens with.
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
