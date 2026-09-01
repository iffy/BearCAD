-- #1776: dragging a cross-section view from the Elements pane onto a projection sections
-- that view — and its aligned children with it. The right-click menu then offers
-- "Remove cross section N" to take it off again.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
-- A cross-section view with one cutting plane through the middle.
local cs = bearcad.cross_section{ name = "Cut" }
local mid = bearcad.body_stats(0).bbox.max.z / 2
bearcad.section_plane{ origin = {0, 0, mid}, normal = {0, 0, -1} }
-- A drawing with a base projection and an aligned child.
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "right", pos = 0.72 }
-- Hide the side panes the drag doesn't need (CI's WM-less Xvfb can't maximize).
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.tool("select")
bearcad.ui.wait(8)

local function sections()
  local views = bearcad.drawing_views(d)
  return views[1].cross_section, views[2].cross_section
end
local base0, child0 = sections()
assert(base0 == nil and child0 == nil, "nothing applied to start")

-- Drag the cross-section view's row ("Cut") onto the base projection's card.
local row = bearcad.ui.elements_row_rect("Cut")
assert(type(row) == "table", "the cross-section row is in the Elements pane")
local card = bearcad.ui.drawing_view_rect(0)
assert(card, "the base projection has a card on the page")
bearcad.ui.drag(row, card)

local base, child = sections()
assert(base == cs, "the dropped cross section applies to the projection, got " .. tostring(base))
assert(child == cs, "the aligned child sections with its base, got " .. tostring(child))

print("ok: dropping a cross section on a projection sections it and its aligned children")
bearcad.quit()
