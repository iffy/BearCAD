-- Documentation screenshot: materials — a 2x2x2 of cubes, each a different colour.
--
-- Eight cuboids on a small grid, assigned the eight contrasting palette colours
-- (Blue through Pink). A gap between them so each body reads as its own solid.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/materials.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Seeded palette ordinals 1–8: Blue, Green, Red, Yellow, Purple, Orange, Cyan, Pink.
-- Unobtainium (0) and Grey (9) stay unused so every cube is a distinct hue.
local palette = {
  { name = "Blue", material = 1 },
  { name = "Green", material = 2 },
  { name = "Red", material = 3 },
  { name = "Yellow", material = 4 },
  { name = "Purple", material = 5 },
  { name = "Orange", material = 6 },
  { name = "Cyan", material = 7 },
  { name = "Pink", material = 8 },
}

-- `at` is the cuboid's base centre. Offset so the world origin sits in the
-- gap between the four columns — otherwise the Z axis spears a cube.
local size = 20
local gap = 6
local step = size + gap
local half = step / 2
local i = 0
for z = 0, 1 do
  for y = 0, 1 do
    for x = 0, 1 do
      i = i + 1
      local spec = palette[i]
      bearcad.cuboid{
        at = { (x * 2 - 1) * half, (y * 2 - 1) * half, z * step },
        width = size,
        depth = size,
        height = size,
        name = spec.name,
      }
      bearcad.set_material{ body = i - 1, material = spec.material }
    end
  end
end

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
