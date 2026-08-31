-- #1541: hovering a closed sketch face during Mirror lights its four edges as a Curve
-- highlight, which carried no element identity — so `bearcad.ui.hovered()` reported nil and a
-- script could not assert the very hover #1539 added.
bearcad.new()
bearcad.rect{ x = -20, y = -15, width = 40, height = 30 }
-- Mirror axis well away from the rectangle so a hover inside the face cannot land on it.
bearcad.line{ x = 50, y = -40, x1 = 50, y1 = 40 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(8)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("mirror")
bearcad.ui.wait(5)
-- Feed the axis through the pane so this test is only about the face hover.
bearcad.select{ kind = "line", index = 4 }
bearcad.ui.wait(5)
assert(picker("Shapes") and picker("Shapes").focused,
  "with an axis set, Shapes takes the next hover")

-- Hover well inside the rectangle, away from every edge.
bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(8)
local h = bearcad.ui.hovered()
assert(h, "the face's lit-up boundary should be visible to scripts, got nil")
assert(h.kind == "line", "the boundary is made of sketch lines, got " .. h.kind)
assert(h.count == 4,
  "all four edges light up, so hovered() should count 4, got " .. tostring(h.count))

-- A plain single-element hover still reports one.
bearcad.ui.tool("select")
bearcad.ui.wait(5)
bearcad.ui.move_ground(0, -15)
bearcad.ui.wait(8)
h = bearcad.ui.hovered()
assert(h and h.count == 1,
  "hovering one edge should report a count of 1, got " .. tostring(h and h.count))

print("ok: a multi-edge hover highlight reports its elements to scripts")
bearcad.quit()
