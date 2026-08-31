-- #953/#968: a picker's rules decide what it will hold, and `bearcad.ui.pickers()` is how a script
-- can tell an accepted pick from a rejected one. A body-set tool consumes the click either way,
-- so `selection()` alone can't tell them apart.
--
-- Combine turns its operands into shadow bodies, so after committing one, the `LiveBody` rule
-- refuses them: selecting a consumed body must leave the Move tool's picker empty.
bearcad.new()
bearcad.rect{ width = 40, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.rect{ width = 20, height = 20, x = 10, y = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 30 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction). The pickers are
-- derived whether or not the Context pane shows (#973), which this also checks.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")

-- Fuse them: both inputs become shadow bodies and the union is a new one.
bearcad.combine{ op = "union", a = {0, 1} }

bearcad.ui.tool("move")
bearcad.ui.wait(5)

local bodies = bearcad.ui.picker("Bodies")
assert(bodies, "the Move tool should show a Bodies picker")
assert(bodies.focused, "it should be the focused picker with nothing picked yet")
assert(#bodies.items == 0, "and start empty, got " .. #bodies.items)
local takes_bodies = false
for _, kind in ipairs(bodies.accepts) do
  if kind == "body" then takes_bodies = true end
end
assert(takes_bodies,
  "it takes bodies, among " .. table.concat(bodies.accepts, ","))

-- A body another operation has consumed is not a valid pick.
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(5)
assert(#bearcad.ui.picker("Bodies").items == 0,
  "a consumed body should not enter the picker, got " .. #bearcad.ui.picker("Bodies").items)

-- The live output body does enter it.
bearcad.select{ kind = "body", index = 2 }
bearcad.ui.wait(5)
local items = bearcad.ui.picker("Bodies").items
assert(#items == 1 and items[1].kind == "body" and items[1].index == 2,
  "the live body should enter the picker, got " .. #items)

print("ok: a picker refuses a body another operation has consumed")
bearcad.quit()
