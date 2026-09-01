-- #1912: Space over a drawing zoom loupe must fan the projected corner under the
-- cursor, including when two bodies share that vertex. The Dimension tool's
-- ordinary hover takes an edge at a corner; the exploder is how you pick the
-- vertex instead. Candidates used to be collected in the card's un-magnified
-- pixels, so Space inside the magnified circle flickered open over nothing.
bearcad.new()
bearcad.ui.tool("select")
-- Two 20 mm cubes sharing the face at x = 10: the top of that face is one
-- world point belonging to both bodies.
bearcad.cuboid{ width = 20, depth = 20, height = 20 }
bearcad.cuboid{ width = 20, depth = 20, height = 20, at = {20, 0, 0} }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

-- Front view: X right, Z up. Shared top vertex sits at (10, 20) mm.
local ux, uz = 10, 20
bearcad.drawing_loupe{ drawing = d, view = 0, at = {ux, uz}, radius = 5,
                       to = {ux + 50, uz - 50}, to_radius = 28 }
bearcad.clear_selection()
bearcad.ui.wait(10)

local vp = bearcad.ui.viewport()
local rect = assert(bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true },
  "the page reports the magnified circle")
local cx, cy = rect.x + rect.w / 2 - vp.x, rect.y + rect.h / 2 - vp.y

bearcad.ui.tool("dimension")
bearcad.ui.wait(5)
bearcad.ui.move(cx, cy)
bearcad.ui.wait(5)
assert(#bearcad.ui.exploder() == 0, "no fan before Space")

bearcad.ui.key("space")
bearcad.ui.wait(6)
local leaves = bearcad.ui.exploder()
assert(#leaves > 0,
  "Space inside the magnified circle should fan what is under the cursor, got 0 leaves")
local corners, corner_hit = 0, nil
for _, l in ipairs(leaves) do
  if l.kind == "projected_corner" then
    corners = corners + 1
    if l.x then corner_hit = l end
  end
end
assert(corners >= 1,
  "the fan should offer the projected vertex the loupe is magnifying, got "
    .. #leaves .. " leaves and " .. corners .. " corners")
assert(corners >= 2,
  "a vertex shared by both bodies should fan as two corners, got " .. corners)

-- Picking a corner loupe arms a point-to-point dimension, which is why the
-- fan is there: the Dimension tool's click on the circle would have taken an edge.
assert(corner_hit, "a corner loupe should be clickable")
bearcad.ui.click(corner_hit.x, corner_hit.y)
bearcad.ui.wait(8)
assert(bearcad.status():find("second point"),
  "picking the vertex loupe should arm a point dimension, got: " .. bearcad.status())

print("ok: Space in a drawing loupe fans a coincident projected corner")
bearcad.quit()
