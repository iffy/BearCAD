-- #1369: a construction plane can be deleted (the context-menu/Delete path).
-- Create a couple of offset construction planes, select one, and delete it.
bearcad.new()
bearcad.rect{ width = 80, height = 40 }
bearcad.exit_sketch()
bearcad.ui.tool("select")
bearcad.ui.wait(3)
bearcad.plane{ from = 0, offset = 20 }
bearcad.plane{ from = 0, offset = 40 }
bearcad.ui.wait(3)

local n0 = bearcad.count("construction_plane")
-- Select the most recently created plane (the count is its ordinal).
-- A warm-up select (then clear) keeps the target selection applying reliably.
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(2)
bearcad.clear_selection()
bearcad.ui.wait(1)
bearcad.select{ kind = "construction_plane", index = n0 - 1 }
bearcad.ui.wait(3)
assert(#bearcad.selection() == 1, "expected the plane to be selected")

bearcad.delete_selection()
bearcad.ui.wait(5)
local n1 = bearcad.count("construction_plane")
assert(n1 == n0 - 1,
  "expected the selected construction plane to be deleted, got " .. n0 .. " -> " .. n1)
print("ok: a construction plane can be deleted")
bearcad.quit()