-- #1750/#1752/#1753/#1757: after a face pick the cutting plane tool shows one offset
-- gizmo and two in-plane tilts (not a spin about the normal). The Anchor picker
-- loses focus so a gizmo grab is not another pick.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 10}, distance = 260 }

bearcad.cross_section{ name = "Front half" }
bearcad.ui.wait(5)
assert(bearcad.ui.workbench() == "view")

bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

-- Click the top face of the block.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)

local names = {}
for _, g in ipairs(bearcad.gizmos()) do
  names[g.name] = g
end
assert(names.offset, "one offset gizmo after a face pick")
assert(names.tilt_u and names.tilt_v, "two in-plane tilts, not a roll about the normal")
assert(not names.roll, "rotation is not around the face normal")

local anchor
for _, p in ipairs(bearcad.pickers()) do
  if p.name == "Anchor" then anchor = p end
end
assert(anchor, "the tool still has an Anchor picker")
assert(not anchor.focused, "after the pick the offset field holds focus, not the Anchor")

print("ok: cutting plane face gizmos are offset + two tilts, picker unfocused")
bearcad.quit()
