-- #1905: viewing a drawing marks that drawing's Elements-pane row (list and graph).
bearcad.new()
bearcad.cuboid{ width = 10, depth = 10, height = 10 }
bearcad.drawing{ name = "Sheet" }
bearcad.ui.tool("select")

local function drawing_active()
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == "drawing" and row.name == "Sheet" then
      return row.active
    end
  end
end

bearcad.ui.elements_view("graph")
bearcad.ui.wait(8)
assert(drawing_active() == true, "graph marks the open drawing")

bearcad.ui.elements_view("list")
bearcad.ui.wait(8)
assert(drawing_active() == true, "list view still reports the open drawing")
assert(type(bearcad.ui.elements_row_rect("Sheet")) == "table",
  "the drawing row is on screen")

bearcad.ui.workbench("model")
bearcad.ui.wait(4)
assert(drawing_active() == false, "leaving the page clears the mark")

print("ok: open drawing is marked in Elements")
bearcad.quit()
