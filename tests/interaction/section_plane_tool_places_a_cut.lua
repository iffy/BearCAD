-- Interaction regression (#1687): in the View workbench, the Cutting plane tool places a
-- plane on whatever face the click lands on, and each further click adds another.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
-- Hide the side panes and pin the camera so ground coordinates land where we expect.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }

bearcad.cross_section{ name = "Front half" }
bearcad.ui.wait(5)
assert(bearcad.ui.workbench() == "view")
assert(#bearcad.section_planes() == 0, "a new view cuts with nothing")

bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)
-- Click the top face of the block.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)
local cuts = bearcad.section_planes()
assert(#cuts == 1, "the click placed a cutting plane, got " .. #cuts)

-- The context pane's numbers drive it: slide, turn, flip.
bearcad.edit_section_plane{ cut = 0, offset = 4, roll = 30, flip = true }
cuts = bearcad.section_planes()
assert(math.abs(cuts[1].offset - 4) < 1e-4, "offset " .. cuts[1].offset)
assert(math.abs(cuts[1].roll - 30) < 1e-3, "roll " .. cuts[1].roll)
assert(cuts[1].flip, "flip")

-- A second click adds a second plane rather than replacing the first.
bearcad.ui.click_ground(20, 20)
bearcad.ui.wait(5)
assert(#bearcad.section_planes() == 2, "a second plane joins the first")

bearcad.delete_section_plane{ cut = 1 }
assert(#bearcad.section_planes() == 1, "and can be dropped again")

print("ok: the cutting plane tool places, adjusts, and stacks planes")
bearcad.quit()
