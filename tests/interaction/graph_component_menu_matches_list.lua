-- #1927: a Graph-view component row offers the same right-click menu as the List view —
-- nested component, export, un-file, copy/paste, delete — not the generic element menu.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 20, depth = 20, height = 20 }
bearcad.component{ name = "Frame" }
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.elements_view("graph")
bearcad.ui.wait(8)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local comp = row_named("component")
assert(comp and comp.x, "the graph shows the component row")

bearcad.ui.right_click({ x = comp.x + comp.w * 0.6, y = comp.y + comp.h / 2 })
bearcad.ui.wait(4)
local items = bearcad.ui.menu_items()
assert(#items > 0, "right-clicking a graph component row should open its context menu")

local function has(label)
  for _, it in ipairs(items) do
    if it == label then return true end
  end
  return false
end

assert(has("New component inside"),
  "graph component menu should offer a nested component, got: " .. table.concat(items, ", "))
assert(has("Export STL…"),
  "graph component menu should offer STL export, got: " .. table.concat(items, ", "))
assert(has("Export 3MF…"),
  "graph component menu should offer 3MF export, got: " .. table.concat(items, ", "))
assert(has("Export STEP…"),
  "graph component menu should offer STEP export, got: " .. table.concat(items, ", "))
assert(has("Move to document root"),
  "graph component menu should offer un-filing, got: " .. table.concat(items, ", "))
assert(has("Copy"),
  "graph component menu should offer Copy, got: " .. table.concat(items, ", "))
assert(has("Delete"),
  "graph component menu should offer Delete, got: " .. table.concat(items, ", "))

print("ok: a Graph-view component row has the List-view component menu")
bearcad.quit()
