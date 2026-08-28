-- Interaction regression (#1805/#1811/#1812): the pencil shading modes survive a real frame.
--
-- The pencil view is scene-building code — wobbled strokes, silhouettes, a hatched contact
-- shadow — that unit tests exercise only in pieces. Driving a real frame is what proves the
-- whole path runs: a panic or a degenerate mesh in any of it fails here.
bearcad.new()
bearcad.ui.tool("select")

bearcad.cuboid{ width = 40, depth = 30, height = 20, at = { 0, 0, 10 } }
bearcad.cylinder{ radius = 8, height = 30, at = { 0, 0, 25 } }
bearcad.sphere{ radius = 6, at = { 40, 0, 6 } }
assert(bearcad.count("body") == 3, "three bodies, got " .. bearcad.count("body"))

bearcad.ui.shading("loose_pencil")
bearcad.ui.view("iso")
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Orbiting re-derives every silhouette from the new eye position, which is the part that
-- would blow up on a degenerate face.
bearcad.ui.orbit(120, 30)
bearcad.ui.wait(2)
-- From below, the contact hatch is suppressed rather than drawn floating in space.
bearcad.ui.camera{ pitch = -40 }
bearcad.ui.wait(2)

-- The mode is a camera setting, so it outlives the geometry it was turned on for.
bearcad.select({ kind = "body", index = 0 })
bearcad.delete_selection()
bearcad.ui.wait(2)
assert(bearcad.ui.camera{}.shading == "loose_pencil", "the mode stuck")

-- Coloured pencil takes each body's material colour (#1812), including bodies whose
-- material was assigned after the mode was already on.
bearcad.ui.shading("colour_pencil")
bearcad.ui.wait(2)
bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
bearcad.ui.wait(2)
assert(bearcad.ui.camera{}.shading == "colour_pencil", "still in coloured pencil")

-- Coloured pencil shades each flat by how it faces the light and drops the solids' shadows
-- onto each other (#1818): more scene-building code — per-flat hatching, a projection onto
-- every lit face, clipped to it — that a real frame has to survive. A post standing on a
-- plate is the case that exercises both.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 90, depth = 90, height = 12 }
bearcad.cuboid{ at = { 30, 30, 12 }, width = 14, depth = 14, height = 45 }
bearcad.cylinder{ radius = 7, height = 20, at = { 70, 20, 12 } }
bearcad.ui.shading("colour_pencil")
bearcad.ui.view("iso")
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
-- Orbiting re-derives the shading and the shadows from the new eye position.
bearcad.ui.orbit(140, 25)
bearcad.ui.wait(2)
-- From below: every face is turned away from the light, so nothing receives a shadow.
bearcad.ui.camera{ pitch = -40 }
bearcad.ui.wait(2)
assert(bearcad.ui.camera{}.shading == "colour_pencil", "still in coloured pencil")

-- And every other mode still renders after them.
for _, mode in ipairs({ "wireframe", "transparent", "solid", "solid_wireframe", "realistic" }) do
  bearcad.ui.shading(mode)
  bearcad.ui.wait()
end
