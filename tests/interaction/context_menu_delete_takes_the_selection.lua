-- Interaction regression (#1853): Delete in a context menu opened on one of several selected
-- rows takes the whole selection, and says so before it does. Reachable from a script at all
-- only because menu items publish their labels and rects (#1856) — an egui popup is otherwise
-- invisible from outside.
bearcad.new()
bearcad.ui.tool("select")
for i = 0, 2 do
  bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {i * 20, 0, 0} }
end
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)
assert(bearcad.count("primitive") == 3, "three cuboids, got " .. bearcad.count("primitive"))

local function row(label)
  local r = bearcad.ui.elements_row_rect(label)
  assert(r, "no Elements row labelled " .. label)
  return r
end

-- Select all three rows: plain click, then shift-click the other two.
bearcad.ui.click(row("Cuboid 0"))
for _, label in ipairs({ "Cuboid 1", "Cuboid 2" }) do
  bearcad.ui.click(row(label), { shift = true })
end
assert(#bearcad.selection() == 3, "three rows selected, got " .. #bearcad.selection())

-- Right-click one of them: the menu must offer to take all three.
bearcad.ui.right_click(row("Cuboid 1"))
local items = bearcad.ui.menu_items()
assert(#items > 0, "right-clicking a row should open its context menu")
local delete
for _, label in ipairs(items) do
  if label:find("^Delete") then delete = label end
end
assert(delete == "Delete 3 elements",
  "the item should say how many it takes, got " .. tostring(delete)
  .. " (menu: " .. table.concat(items, ", ") .. ")")

local r = bearcad.ui.menu_item_rect(delete)
assert(r, "the Delete item should report where it drew")
bearcad.ui.click(r)
assert(bearcad.count("primitive") == 0,
  "Delete should take every selected cuboid, got " .. bearcad.count("primitive") .. " left")

print("ok: context-menu Delete takes the whole selection")
bearcad.quit()
