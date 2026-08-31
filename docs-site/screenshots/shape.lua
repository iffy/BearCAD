-- Documentation screenshot: the Shape tool.
--
-- All three shapes it can place, side by side on the ground: a cuboid, a cylinder,
-- and a sphere, sized so they read at a glance and sit on a common baseline.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/shape.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Left to right, each resting on the ground plane. A sphere is placed by the point it
-- rests on, so its radius is what lifts it; the cuboid and cylinder grow from their base.
bearcad.cuboid{ at = {0, 0, 0}, width = 34, depth = 34, height = 34, name = "Cuboid" }
bearcad.cylinder{ at = {58, 0, 0}, radius = 17, height = 34, name = "Cylinder" }
bearcad.sphere{ at = {110, 0, 0}, radius = 17, name = "Sphere" }

bearcad.clear_selection()
-- A clean background (#667): the datum planes and the ground grid away, so the three
-- solids are the whole picture.
bearcad.set_visible({ kind = "plane" }, false)
bearcad.ui.ground("off")
bearcad.ui.shading("realistic")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
