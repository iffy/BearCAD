-- A hanging cutting plane is its own element under the Views section, not just a
-- context-pane row on the open view.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cross_section{ name = "Front half" }
bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 0, 1} }

bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local rows = bearcad.ui.elements_graph().rows
local kinds = {}
local names = {}
for _, row in ipairs(rows) do
  kinds[row.kind] = row.name
  names[#names + 1] = row.kind .. "=" .. row.name
end
assert(kinds["views"], "the pane groups under a Views section")
assert(kinds["cross_section"] == "Front half",
  "the view is still a row, got " .. tostring(kinds["cross_section"]))
assert(kinds["section_plane"] ~= nil,
  "the cutting plane is its own row in Views, got: " .. table.concat(names, ", "))

print("ok: a cutting plane shows as an element in the Views section")
bearcad.quit()
