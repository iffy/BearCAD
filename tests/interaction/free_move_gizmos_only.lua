-- #1423: Free Move only lets you pick rotation and translation gizmo handles.
-- The background construction plane (and the body itself) must not hover-highlight
-- or join the move set once the gizmos are up.
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.camera{ target = {10, 10, 0}, distance = 320 }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
-- A filled reference point leaves focus on Bodies, which used to hover-highlight
-- the datum planes the Bodies picker also accepts.
bearcad.begin_move{
  bodies = {0},
  from = { body = 0, vertex = {0, 0, 0} },
}
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)

local bodies = bearcad.picker("Bodies")
assert(bodies and #bodies.items == 1 and bodies.items[1].kind == "body",
  "Free Move starts with the one body")

local gizmos = 0
for _, g in ipairs(bearcad.gizmos()) do
  if g.name:find("^move_") then gizmos = gizmos + 1 end
end
assert(gizmos >= 6, "Free Move should arm translation and rotation gizmos, got " .. gizmos)

-- (80, 80) is on the XY datum (gap 5..105), well clear of the 20×20 cuboid.
bearcad.ui.move_ground(80, 80)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(not h,
  "Free Move must not hover-highlight the construction plane, got "
    .. tostring(h and h.kind))

bearcad.ui.click_ground(80, 80)
bearcad.ui.wait(8)
bodies = bearcad.picker("Bodies")
assert(#bodies.items == 1 and bodies.items[1].kind == "body",
  "clicking the plane must not add it to the move set, got "
    .. #bodies.items .. " item(s), first=" .. tostring(bodies.items[1] and bodies.items[1].kind))

-- A point on the body's top face that is not a face-centre translation handle
-- (those sit at (10, 10)) and not a rotation ring.
bearcad.ui.move_ground(5, 5)
bearcad.ui.wait(8)
h = bearcad.hovered()
assert(not h,
  "Free Move must not hover-highlight the body, got " .. tostring(h and h.kind))

bearcad.ui.click_ground(5, 5)
bearcad.ui.wait(8)
assert(#bearcad.picker("Bodies").items == 1,
  "clicking the body must not drop it from the move set, got "
    .. #bearcad.picker("Bodies").items)

print("ok: free move hover and click stay on the gizmos")
bearcad.quit()
