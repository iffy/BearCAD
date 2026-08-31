-- #1504: Chamfer in a sketch toggles vertices. Re-click drops one; Esc is not required.
bearcad.new()
bearcad.rect{ x = -20, y = -15, width = 40, height = 30 }
bearcad.ui.tool("chamfer")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local sel = picker("Selection")
assert(sel, "in-sketch Chamfer should show a Selection picker")
assert(#sel.items == 0, "starting empty, got " .. #sel.items)

-- Two opposite corners, far from each other's gizmo.
bearcad.ui.click_ground(-20, -15)
bearcad.ui.wait(5)
assert(#picker("Selection").items == 1,
  "first corner should enter the set, got " .. #picker("Selection").items)

bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(5)
assert(#picker("Selection").items == 2,
  "second corner should add, not replace, got " .. #picker("Selection").items)

-- Re-click the second corner (the gizmo sits on the first).
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(5)
assert(#picker("Selection").items == 1,
  "re-click should drop that corner, got " .. #picker("Selection").items)

print("ok: in-sketch Chamfer clicks toggle vertices")
bearcad.quit()
