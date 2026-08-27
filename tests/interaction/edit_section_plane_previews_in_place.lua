-- #1783/#1787: editing a cutting plane (`bearcad.begin_edit_section_plane`) previews in
-- place of the plane being edited — the live drag must not compound with the plane's
-- committed state — and cancelling leaves the plane exactly as it was.
bearcad.new()
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.cross_section{ name = "Half" }
bearcad.section_plane{ plane = 1, offset = 5 }   -- the XZ datum, slid 5 mm
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)

local function offset_of()
  local cuts = bearcad.section_planes(0)
  return cuts[1].offset
end
assert(math.abs(offset_of() - 5) < 1e-4, "the plane starts at offset 5")

-- Open the live edit draft.
bearcad.begin_edit_section_plane{ cut = 0 }
bearcad.ui.wait(5)
assert(bearcad.status():find("Edit cutting plane"),
  "begin_edit_section_plane opens the edit draft, got: " .. bearcad.status())

-- Drag the offset arrow: the preview moves, the committed plane does not.
bearcad.ui.move_ground(60, 30)
bearcad.ui.wait(5)
bearcad.ui.click_ground(60, 30)
bearcad.ui.wait(6)
bearcad.ui.move_ground(85, 30)
bearcad.ui.wait(6)
assert(math.abs(offset_of() - 5) < 1e-4,
  "dragging the preview must not move the committed plane, got " .. offset_of())

-- Esc drops the edit: the plane keeps its committed pose.
bearcad.ui.key("escape")
bearcad.ui.wait(6)
assert(math.abs(offset_of() - 5) < 1e-4,
  "Esc leaves the committed plane alone, got " .. offset_of())

print("ok: editing a cutting plane previews in place and cancels cleanly")
bearcad.quit()
