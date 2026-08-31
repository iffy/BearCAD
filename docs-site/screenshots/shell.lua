-- Documentation screenshot: the Shell tool (#1824).
--
-- A block hollowed to a 3 mm wall with its top face left open, seen from above-front so
-- the wall thickness and the floor inside both read. The input lives on as a shadow body,
-- which is why only the hollowed result is in the picture.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/shell.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.cuboid{ width = 44, depth = 32, height = 22, name = "Tray" }
bearcad.shell{
  bodies = {0},
  faces = {{ kind = "primitive_face", primitive = 0, face = "top" }},
  thickness = "3",
  name = "Tray",
}

-- Hide the three datum planes a new document opens with, and the ground grid, for a clean
-- background (#579/#667).
bearcad.set_visible({ kind = "plane" }, false)
bearcad.ui.ground("off")
bearcad.ui.tool("shell")
-- High enough to look down into the open top; near enough to the front that the outside
-- walls still show their thickness at the rim.
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into the web
-- app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
