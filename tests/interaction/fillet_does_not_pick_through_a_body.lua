-- #1462: from +X the near and far +Y verticals stack. A click must take the
-- visible near edge, not the far one through the solid.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 50, height = 22 }
bearcad.ui.tool("fillet")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("right")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 11}, distance = 200 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

assert(picker("Edges"), "the Fillet tool should show an Edges picker")
assert(#picker("Edges").items == 0, "starting empty")

-- The +Y verticals at x = ±20 share this screen position from +X.
bearcad.ui.click_ground(20, 25)
bearcad.ui.wait(5)
assert(#picker("Edges").items >= 1, "should pick the visible near edge")

bearcad.ui.key("enter")
bearcad.ui.wait(10)

local live
for i = 0, 5 do
  local s = bearcad.body_stats(i)
  if s then live = s end
end
assert(live, "fillet should produce a live body")
assert(live.bbox.min.x < -19.0,
  "the far (−X) vertical must stay sharp, min.x=" .. tostring(live.bbox.min.x))

print("ok: fillet tool does not pick an edge through a body")
bearcad.quit()
