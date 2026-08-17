-- #1437: selecting Repeat in a sketch must show the Entities picker immediately,
-- hover-highlight sketch entities the picker can take, and let a click collect them.
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 4 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

bearcad.ui.tool("repeat")
bearcad.ui.wait(3)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

local entities = picker("Entities")
assert(entities, "selecting Repeat in a sketch should show the Entities picker immediately")
assert(entities.focused, "Entities should be armed so the next click fills it")
assert(#entities.items == 0, "the picker starts empty when nothing was pre-selected")

-- Hover the circle's rim: the focused picker takes circles, so it must light up.
bearcad.ui.move_ground(0, 4)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(h and h.kind == "circle",
  "the Repeat tool should highlight a hoverable circle, got " .. tostring(h and h.kind))

bearcad.ui.click_ground(0, 4)
bearcad.ui.wait(8)
entities = picker("Entities")
assert(entities and #entities.items == 1,
  "clicking the highlighted circle should collect it, got " .. #(entities and entities.items or {}))
assert(entities.items[1].kind == "circle",
  "the collected item should be the circle, got " .. tostring(entities.items[1] and entities.items[1].kind))

print("ok: in-sketch Repeat shows its Entities picker, highlights, and selects")
bearcad.quit()
