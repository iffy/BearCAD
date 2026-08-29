-- Documentation screenshot: materials — the cube cluster, in Realistic shading, and the
-- same frame again in Dark pencil (#1843/#1844). The front page lays the second over the first
-- and fades between them along a diagonal, so the hero shows the app drawing the same model
-- two ways; the two shots must therefore be framed identically, which is why one script
-- takes both without touching the camera in between.
--
-- The scene is `scenes/cube_cluster.lua`, which the view-styles page shoots too.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local dir = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."
local out = dir .. "/materials.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

-- The scene itself is shared with the view-styles page (#1813).
dofile("docs-site/screenshots/scenes/cube_cluster.lua")

bearcad.clear_selection()
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
bearcad.ui.shading("realistic")
-- The OS cursor would hover-highlight whichever face it sits on; Dimension has no
-- pick hover, so the colors stay clean.
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern). Saved before the style
-- changes, so the file opens the way the picture beside it looks.
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

-- The same frame, drawn by hand (#1843) — the camera is untouched, so the two pictures
-- register pixel for pixel and the front page can cross-fade between them. The dark-mode
-- pencil (#1844), so both halves of the hero sit on the same ground and what changes across
-- the fade is the drawing, not the paper.
bearcad.ui.shading("dark_pencil")
bearcad.ui.wait(3)
bearcad.ui.screenshot(dir .. "/materials-pencil.png")

bearcad.quit()
