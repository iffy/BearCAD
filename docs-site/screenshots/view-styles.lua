-- Documentation screenshot: one small picture of the cube cluster per shading mode (#1813).
--
-- The same scene the front page shows, framed identically each time, so the pictures differ
-- only in how the scene is drawn — which is the whole point of the page they illustrate.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling back
-- to ".". The PNGs are only written where a real GPU frame renders.

local out = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

bearcad.ui.tool_hints(false)
dofile("docs-site/screenshots/scenes/cube_cluster.lua")

bearcad.clear_selection()
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
bearcad.ui.ground("off")
-- Dimension has no pick hover, so the OS cursor can't highlight a face into the shot.
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Every mode the gear popup offers. The docs page shows one picture per name here, and a
-- test fails if the two lists ever disagree.
local styles = {
  "wireframe", "transparent", "solid", "solid_wireframe",
  "realistic", "loose_pencil", "dark_pencil", "color_pencil", "watercolor",
}
for _, style in ipairs(styles) do
  bearcad.ui.shading(style)
  bearcad.ui.wait(3)
  bearcad.ui.screenshot(out .. "/view-styles-" .. style .. ".png")
end

bearcad.quit()
