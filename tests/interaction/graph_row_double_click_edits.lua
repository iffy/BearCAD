-- #1770: double-clicking an element row in the Elements pane's Graph view opens it for
-- editing, exactly like List-view rows do (#1767): dedicated editors for sketches, planes,
-- and extrusions, and the universal edit path (`node_editable_operation`) for every other
-- editable operation — not just right-click → Edit. A single click keeps selecting.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 40, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.cross_section{ name = "Front half" }
local cut = bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 0, 1}, offset = 4 }
assert(cut == 0, "the view starts with one cutting plane")

-- Creating a cutting plane opens the View workbench; its graph rows show there.
assert(bearcad.ui.workbench() == "view")
bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local function center(row)
  return row.x + row.w / 2, row.y + row.h / 2
end

-- egui folds rapid same-spot releases into single/double/triple counts over ~0.6 s.
-- Long waits before each probe let those counters reset, so every click below starts
-- from count 1 and each double-click lands on count 2 exactly.
local function fresh_click(row)
  local x, y = center(row)
  bearcad.ui.wait(250)
  bearcad.ui.click(x, y)
  bearcad.ui.wait(8)
end

local function fresh_double_click(row)
  local x, y = center(row)
  bearcad.ui.wait(250)
  bearcad.ui.click(x, y)
  bearcad.ui.wait(5)
  bearcad.ui.double_click(x, y)
  bearcad.ui.wait(8)
end

-- Universal path: one click on the cutting-plane row only selects; a double-click reopens
-- its live draft (#1755).
local cut_row = row_named("section_plane")
assert(cut_row and cut_row.x, "the cutting-plane row exists in the graph")
fresh_click(cut_row)
assert(#bearcad.selection() == 1 and bearcad.selection()[1].kind == "section_plane",
  "a plain click on the cutting-plane graph row selects it, got "
    .. #bearcad.selection() .. " item(s)")
assert(bearcad.ui.tool() == "select",
  "a single click must not start editing, tool is " .. tostring(bearcad.ui.tool()))
fresh_double_click(cut_row)
assert(bearcad.ui.tool() == "section_plane",
  "double-clicking the cutting-plane graph row enters its edit draft, tool is "
    .. tostring(bearcad.ui.tool()))

-- Enter commits the edit replace-in-place (#1755).
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
local cuts = bearcad.section_planes(0)
assert(#cuts == 1, "committing the edit replaces the plane, got " .. #cuts)

-- Dedicated editor: double-clicking the extrusion row reopens the Extrude tool's draft;
-- one click still just selects.
local ex = row_named("extrusion")
assert(ex and ex.x, "the extrusion row exists in the graph")
fresh_click(ex)
assert(#bearcad.selection() == 1 and bearcad.selection()[1].kind == "extrusion",
  "a plain click on the extrusion graph row selects it, got "
    .. #bearcad.selection() .. " item(s)")
fresh_double_click(ex)
assert(bearcad.ui.tool() == "extrude",
  "double-clicking the extrusion graph row reopens the Extrude tool, tool is "
    .. tostring(bearcad.ui.tool()))
bearcad.ui.key("Escape")

print("ok: double-clicking a graph row opens its element for editing")
bearcad.quit()
