-- #1967: a component copied from its Elements row pastes as a new component holding a copy
-- of every body in it — by keyboard (⌘C/⌘V) and from the row's context menu.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 20, depth = 20, height = 10 }
bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {40, 0, 0} }
local c = bearcad.component{ name = "Pair" }
bearcad.move_to_component{ kind = "body", index = 0, component = c }
bearcad.move_to_component{ kind = "body", index = 1, component = c }
bearcad.clear_selection()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local function row(label)
  local r = bearcad.ui.elements_row_rect(label)
  assert(r, "no Elements row labelled " .. label)
  return r
end

-- Keyboard: click the row, ⌘C, ⌘V, Enter.
bearcad.ui.click(row("Pair"))
bearcad.ui.wait(3)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "component",
  "clicking the row should select the component, got " .. #sel .. " items")
bearcad.ui.key("c", { cmd = true })
bearcad.ui.wait(3)
assert(bearcad.status():find("Copied"), "⌘C should copy the component: " .. bearcad.status())
bearcad.ui.key("v", { cmd = true })
bearcad.ui.wait(3)
bearcad.ui.key("enter")
bearcad.ui.wait(8)
assert(bearcad.count("body") == 4, "paste should add both bodies, got " .. bearcad.count("body"))
assert(bearcad.count("component") == 2,
  "paste should add a component, got " .. bearcad.count("component"))

-- Context menu: right-click the (unselected) row and choose Copy, then Paste.
bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.wait(3)
local function choose(label)
  local r = bearcad.ui.menu_item_rect(label)
  assert(r, "context menu has no " .. label .. " (menu: "
    .. table.concat(bearcad.ui.menu_items(), ", ") .. ")")
  bearcad.ui.click(r)
  bearcad.ui.wait(3)
end
bearcad.ui.right_click(row("Pair"))
bearcad.ui.wait(3)
choose("Copy")
assert(bearcad.status():find("Copied"),
  "context-menu Copy should copy the right-clicked component: " .. bearcad.status())
bearcad.ui.right_click(row("Pair"))
bearcad.ui.wait(3)
choose("Paste")
bearcad.ui.key("enter")
bearcad.ui.wait(8)
assert(bearcad.count("body") == 6, "second paste should add two more bodies, got "
  .. bearcad.count("body"))

print("ok: a component copies and pastes from its row")
bearcad.quit()
