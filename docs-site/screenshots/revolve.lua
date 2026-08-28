-- Documentation screenshot: the Revolve tool.
--
-- Revolves a rectangular profile 270 degrees around the global Y axis into a
-- partial ring, so both the swept solid and the flat sweep-end caps are visible.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a GPU, or CI Linux with the software Vulkan driver); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/revolve.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Profile: a 10 x 12 rectangle standing off the axis by 12 mm.
bearcad.rect{ x = 12, y = 0, width = 10, height = 12, name = "Profile" }
bearcad.exit_sketch()
bearcad.revolve{ polygon = {0, 1, 2, 3}, axis = "y", angle = 270, name = "Ring" }

-- Hide the three datum planes a new document opens with.
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
