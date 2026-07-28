-- Documentation screenshot: the Chamfer tool on a solid (3D).
--
-- An 80 x 50 x 20 mm box with its four top edges cut flat, from a fixed corner view
-- (the angular sibling of fillet.lua).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/chamfer.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 80, height = 50, name = "Base" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 20, name = "Block" }

-- Cut the two long top edges flat in one operation (opposite edges — bevels meeting at
-- a shared corner aren't supported). One call per edge would give two operations each
-- cutting the *same* sharp box, and the overlapping outputs would hide both (#672).
bearcad.chamfer_edge{
  extrusion = 0,
  edges = {
    { kind = "cap", face = 0, edge = 0, top = true },
    { kind = "cap", face = 0, edge = 2, top = true },
  },
  distance = 6,
}

bearcad.exit_sketch()
-- Hide the three datum planes a new document opens with.
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
-- The source sketch too: its rectangle sits outside the beveled body and reads as a stray outline.
bearcad.set_visible({ kind = "sketch", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
