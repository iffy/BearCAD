-- #1767: double-clicking a cutting plane row in the Elements pane reopens it for editing —
-- the cutting-plane tool takes over with its live offset/tilt draft (#1755) — rather than
-- only reopening the View workbench, which leaves the plane itself uneditable.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cross_section{ name = "Front half" }
local cut = bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 0, 1}, offset = 4 }
assert(cut == 0, "the view starts with one cutting plane")

-- Creating the view opens the View workbench; its Elements rows show there (default List view).
assert(bearcad.ui.workbench() == "view")
bearcad.ui.wait(5)

local row = bearcad.ui.elements_row_rect("Cutting plane")
assert(row, "the cutting plane row exists in Elements")

-- Row rects are in window coordinates; pointer calls are viewport-relative.
local vp = bearcad.ui.viewport()
assert(vp.x and vp.y, "the viewport reports where it sits in the window")
bearcad.ui.double_click(row.x + row.w / 2 - vp.x, row.y + row.h / 2 - vp.y)
bearcad.ui.wait(8)

assert(bearcad.ui.tool() == "section_plane",
  "double-clicking the cutting plane enters its edit draft, tool is "
    .. tostring(bearcad.ui.tool()))

-- Enter commits the edit: replace-in-place, so the plane does not stack a twin (#1755).
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
local cuts = bearcad.section_planes(0)
assert(#cuts == 1, "committing an edit replaces the plane, got " .. #cuts)
assert(math.abs(cuts[1].offset - 4) < 1e-4, "the committed edit keeps the plane's offset")

print("ok: double-clicking a cutting-plane row reopens it for editing")
bearcad.quit()
