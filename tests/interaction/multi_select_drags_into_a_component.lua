-- #1966: with several Elements rows selected, dragging one of them onto a component files
-- the whole selection there.
bearcad.new()
bearcad.ui.tool("select")
for i = 0, 2 do
  bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {i * 20, 0, 0} }
end
local c = bearcad.component{ name = "Group" }
bearcad.clear_selection()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local function row(label)
  local r = bearcad.ui.elements_row_rect(label)
  assert(r, "no Elements row labelled " .. label)
  return r
end

bearcad.ui.click(row("Cuboid 0"))
bearcad.ui.click(row("Cuboid 1"), { shift = true })
assert(#bearcad.selection() == 2, "two rows selected, got " .. #bearcad.selection())

bearcad.ui.drag(row("Cuboid 1"), row("Group"))
bearcad.ui.wait(4)

bearcad.set_visible(c, false)
assert(not bearcad.visible({ kind = "body", index = 0 }), "Cuboid 0 should be filed in Group")
assert(not bearcad.visible({ kind = "body", index = 1 }), "Cuboid 1 should be filed in Group")
assert(bearcad.visible({ kind = "body", index = 2 }), "Cuboid 2 wasn't selected and stays out")

print("ok: a multi-selection drags into a component")
bearcad.quit()
