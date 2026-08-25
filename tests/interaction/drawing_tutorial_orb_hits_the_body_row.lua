-- #1702/#1705: the Elements-pane row anchors were written with `insert_temp` and only ever
-- filled when empty, so the rect captured while the Modeling pane held its longer tree stuck
-- around on the Drawing workbench — the orb sat rows below the body it named. Clicking where
-- the orb points has to place the view the step asks for.
bearcad.ui.tool("select")
bearcad.ui.tutorial("drawing")
bearcad.ui.wait(8)

local function settled_orb()
  local last = bearcad.ui.tutorial_orb()
  for _ = 1, 40 do
    bearcad.ui.wait(12)
    local now = bearcad.ui.tutorial_orb()
    if not now then return nil end
    if last and math.abs(now.x - last.x) < 1 and math.abs(now.y - last.y) < 1 then
      return now
    end
    last = now
  end
  return last
end

-- Walk to the first "click the bracket in the Elements pane" step, assisting the ones before
-- it so the page and the Add-view tool are really up.
local guard = 0
while true do
  guard = guard + 1
  assert(guard < 40, "the Elements-pane step should come up")
  local text = bearcad.ui.tutorial_narration()
  if text and text:find("Elements pane") then break end
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(4)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(4)
  end
  assert(bearcad.ui.tutorial_step() ~= nil, "the walkthrough ended before that step")
end

-- Next-ing past "Click the Projection tool" advances the step without arming it, so arm it
-- here: this test is about where the orb sits, not about the step before.
bearcad.ui.tool("drawing_add")
bearcad.ui.wait(6)

local before = #bearcad.drawing_views(0)
local orb = settled_orb()
assert(orb, "the step rings a row in the Elements pane")
-- `bearcad.ui.click` addresses the viewport, `tutorial_orb` reports a window point (#1692).
local vp = bearcad.ui.viewport()
bearcad.ui.click(orb.x - vp.x, orb.y - vp.y)
bearcad.ui.wait(12)
assert(#bearcad.drawing_views(0) > before,
  string.format("clicking the orb at %.0f,%.0f should land a view, still %d",
    orb.x, orb.y, before))

print("ok: the drawing tutorial's orb sits on the body row it names")
bearcad.quit()
