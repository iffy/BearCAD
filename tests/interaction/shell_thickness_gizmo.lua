-- #1164: the Shell tool's thickness push/pull gizmo is scriptable (`set_gizmo`) and
-- drives the live wall thickness that commit uses.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 40, height = 30 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 15}, distance = 200 }
bearcad.ui.wait(5)

bearcad.ui.tool("shell")
bearcad.ui.wait(3)
-- Pick the cuboid body into the shell targets (click its footprint).
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)

local gizmos = bearcad.ui.gizmos()
local found = false
for _, g in ipairs(gizmos) do
  if g.name == "shell" and g.kind == "push_pull" then
    found = true
    break
  end
end
assert(found, "shell thickness gizmo should be available after a body pick")

bearcad.ui.set_gizmo{ name = "shell", value = 4 }
bearcad.ui.wait(3)
local after = nil
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name == "shell" then after = g.value break end
end
assert(after and math.abs(after - 4) < 0.01,
  "set_gizmo should set shell thickness to 4 mm, got " .. tostring(after))
bearcad.ui.key("enter")
bearcad.ui.wait(10)

-- Input is shadowed; one hollowed output body.
assert(bearcad.count("body") == 2,
  "input + hollowed output, got " .. bearcad.count("body"))
local status = bearcad.status()
assert(status:find("[Ss]hell") or status:find("hollow") or status:find("4"),
  "expected a shell commit status, got: " .. status)

print("ok: shell thickness gizmo is exposed and set_gizmo drives wall thickness")
bearcad.quit()
