-- #1630: right-clicking a construction plane in the viewport opens that plane's context
-- menu — the same menu its Elements-pane row shows, so "Import image on this plane…" is
-- one right-click away without selecting the plane first.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.tool("select")
bearcad.ui.view("top")
-- The XY datum plane spans 5..105 mm on both axes; frame its middle.
bearcad.ui.camera{ target = {55, 55, 0}, distance = 400, projection = "orthographic" }
bearcad.ui.wait(8)

assert(bearcad.ui.context_menu() == nil, "no context menu should be open to start with")
assert(#bearcad.selection() == 0, "nothing should be selected to start with")

-- (55, 55) is inside the XY plane's quad and clear of the world axes, which win the pick.
bearcad.ui.right_click_ground(55, 55)
bearcad.ui.wait(8)

local menu = bearcad.ui.context_menu()
assert(menu, "right-clicking the XY plane should open a context menu")
assert(menu.kind == "construction_plane",
  "the menu should act on the construction plane, got " .. tostring(menu.kind))
local plane = bearcad.get{ kind = "construction_plane", index = menu.index }
assert(plane, "the menu should name a real plane, index " .. tostring(menu.index))
assert(math.abs(plane.normal[3] - 1) < 1e-3,
  "the right-click should land on the XY plane, got normal z " .. tostring(plane.normal[3]))

print("ok: right-clicking a construction plane opens its context menu")
bearcad.quit()
