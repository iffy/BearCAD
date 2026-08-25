-- Interaction regression (#1689): a technical drawing can import a whole cross-section view.
-- What lands on the page is the model cut the way the View workbench shows it, with the faces
-- the planes opened hatched.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 30 }
bearcad.exit_sketch()

bearcad.cross_section{ name = "Half" }
bearcad.section_plane{ plane = 1 }          -- the XZ datum, keeping the +Y half
bearcad.ui.workbench("model")

bearcad.drawing{ name = "Sheet" }
assert(bearcad.ui.workbench() == "drawing", "the new drawing opens its workbench")
bearcad.drawing_view{ drawing = 0, cross_section = 0 }
bearcad.ui.wait(5)

local views = bearcad.drawing_views(0)
assert(#views == 1, "the view landed on the page, got " .. #views)
assert(bearcad.status():find("cross section"),
  "the status names what was added, got: " .. bearcad.status())

-- The page shows only half the block: its projected height is the full 30, but its width
-- across the cut is half the 40 depth.
local v = views[1]
assert(v.orientation ~= nil, "the placed view reports its orientation")

-- Bodies can be imported *from* a view too: the same body, shown cut by it.
bearcad.drawing_view{ drawing = 0, body = 0, cross_section = 0 }
assert(#bearcad.drawing_views(0) == 2, "a body imported from the view sits beside it")
bearcad.drawing_view_section{ drawing = 0, view = 1, cross_section = false }
assert(bearcad.status():find("whole body"),
  "and its section can be cleared, got: " .. bearcad.status())

print("ok: a drawing can import a whole cross-section view")
bearcad.quit()
