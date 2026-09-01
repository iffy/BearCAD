-- Documentation screenshot: one small picture of a projection per drawing style (#1838).
--
-- The same part, the same card, the same camera each time — so the pictures differ only in
-- how the projection is drawn, which is what the page they illustrate is about. A plate with
-- a hole and a coloured block on top: the hole reads as hidden lines in Wireframe and
-- disappears in Visible edges, and the two bodies keep the styles that use material colour
-- (Colorful, Coloured pencil, Watercolour) from coming out one flat tone.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling back
-- to ".". The PNGs are only written where a real GPU frame renders.

local out = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

bearcad.ui.tool_hints(false)
bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Plate (blue) with a hole through it. A cut makes a new body; hold the handle it returns.
local plate = bearcad.cuboid{ width = 60, depth = 40, height = 10, name = "Plate" }
bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
local hole = bearcad.circle{ x = 18, y = 20, r = 7, name = "Hole" }
plate = bearcad.extrude{ profiles = hole, distance = -14, body = "cut" }
bearcad.exit_sketch()
bearcad.set_visible({ kind = "sketch", index = 0 }, false)
bearcad.set_material{ body = plate, material = 1 }

-- A smaller block (orange) standing on it, so the styles that keep material color have two
-- to hold rather than one flat tone.
local boss = bearcad.cuboid{ at = { 14, 0, 10 }, width = 20, depth = 20, height = 18, name = "Boss" }
bearcad.set_material{ body = boss, material = 6 }

local d = bearcad.drawing{}
-- A page the shape of the viewport, so the sheet fills the shot instead of floating in it.
bearcad.drawing_page{ drawing = d, width = 240, height = 135, margin = 6 }
-- White paper for every sample (#1831): it is what the print looks like, and the pencil
-- styles draw on it regardless — so the seven pictures compare like for like.
bearcad.drawing_paper{ drawing = d, paper = "white" }
bearcad.drawing_view{ drawing = d, bodies = { plate, boss }, orientation = "front-right-top" }
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.5, y = 0.5 }
bearcad.drawing_view_size{ drawing = d, view = 0, width = 0.92, height = 0.92 }
bearcad.drawing_view_label{ drawing = d, view = 0, hidden = true }
bearcad.ui.wait(6)
local vp = bearcad.ui.viewport()

-- Every style the projection's Style dropdown offers. The docs page shows one picture per
-- name here, and a test fails if the two lists ever disagree.
local styles = {
  "visible", "wireframe", "shaded", "colorful",
  "loose_pencil", "color_pencil", "watercolor",
}
for _, style in ipairs(styles) do
  bearcad.drawing_view_style{ drawing = d, view = 0, style = style }
  -- No selection chrome in the picture: the card's handles and ✕ are not the subject.
  -- Setting a style selects the view it was set on, so deselect after, not before — by
  -- clicking the empty margin beside the sheet, which is what a person would do.
  bearcad.ui.click(vp.width * 0.02, vp.height * 0.5)
  bearcad.ui.wait(4)
  bearcad.ui.screenshot(out .. "/view-styles-drawing-" .. style .. ".png")
end

bearcad.quit()
