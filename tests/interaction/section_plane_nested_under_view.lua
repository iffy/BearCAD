-- #1754/#1761: cutting planes nest under their view. In the modeling workbench they
-- do not show in the Elements pane.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cross_section{ name = "Front half" }
bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 0, 1} }

bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local g = bearcad.ui.elements_graph()
local plane_row, view_row
for i, row in ipairs(g.rows) do
  if row.kind == "section_plane" then plane_row = i end
  if row.kind == "cross_section" then view_row = i end
end
assert(plane_row and view_row, "both the view and its plane are rows")
local hung = false
for _, e in ipairs(g.edges) do
  if e.to == plane_row and e.from == view_row and e.kind == "parent" then
    hung = true
  end
end
assert(hung, "the cutting plane hangs off its view as a parent edge")

-- Leave the View workbench: the plane must disappear from the pane.
bearcad.ui.workbench("model")
bearcad.ui.wait(5)
local kinds = {}
for _, row in ipairs(bearcad.ui.elements_graph().rows) do
  kinds[row.kind] = (kinds[row.kind] or 0) + 1
end
assert(kinds.cross_section == 1, "the view remains")
assert(kinds.section_plane == nil, "its cutting planes hide in the modeling workbench")

print("ok: cutting planes nest under their view and hide while modeling")
bearcad.quit()
