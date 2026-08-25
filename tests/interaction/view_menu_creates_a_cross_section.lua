-- Interaction regression (#1671): the View menu offers Create Cross Section, and the
-- Elements pane's + menu offers the same thing. The native OS menu bar can't be driven by
-- pointer input, so its shape is asserted through `bearcad.ui.menu_structure`.
bearcad.new()
bearcad.ui.tool("select")

local view = bearcad.ui.menu_structure()["View"]
assert(view ~= nil, "the menu bar should have a View menu")
local labels = {}
for _, label in ipairs(view) do labels[label] = true end
assert(labels["Create Cross Section"],
  "View should offer Create Cross Section, it has: " .. table.concat(view, ", "))

-- The view element itself: created, listed, named, and shown in the Elements pane.
assert(bearcad.count("cross_section") == 0, "a new document has no views")
bearcad.cross_section{ name = "Front half" }
assert(bearcad.count("cross_section") == 1, "the view is created")
bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local rows = bearcad.ui.elements_graph().rows
local kinds = {}
for _, row in ipairs(rows) do kinds[row.kind] = row.name end
assert(kinds["views"], "the pane groups views under a Views section")
assert(kinds["cross_section"] == "Front half",
  "and shows the view by name, got " .. tostring(kinds["cross_section"]))

print("ok: a cross-section view can be created and shows in the Elements pane")
bearcad.quit()
