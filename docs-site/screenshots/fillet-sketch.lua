-- Documentation screenshot: the Fillet tool in a sketch (2D).
--
-- A rectangle profile with one corner rounded by a sketch fillet, seen from the top
-- in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/fillet-sketch.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 40, height = 30, name = "Profile" }
-- Round the top-right corner (line 1's end = line 2's start) at 10 mm.
bearcad.fillet_vertex{ point = { kind = "line", index = 1, endpoint = "end" }, radius = 10 }

bearcad.clear_selection()
bearcad.ui.tool("dimension")
-- A clean background (#667): the ground plane's quad and the grid both away. The
-- sketch drawn on that plane stays visible.
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
