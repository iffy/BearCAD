-- Documentation screenshot: materials — the cube cluster, in Realistic shading.
--
-- The scene is `scenes/cube_cluster.lua`, which the view-styles page shoots too.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/materials.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

-- The scene itself is shared with the view-styles page (#1813).
dofile("docs-site/screenshots/scenes/cube_cluster.lua")

bearcad.clear_selection()
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
bearcad.ui.shading("realistic")
-- The OS cursor would hover-highlight whichever face it sits on; Dimension has no
-- pick hover, so the colours stay clean.
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
