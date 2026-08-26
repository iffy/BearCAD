-- #1728: every Parameters row overwrote the same tutorial anchor, so a step that says "click
-- the `width` value" rang whichever row happened to be drawn last. Clicking where the orb
-- points has to open the row the step names, whatever else is on the pane.
bearcad.ui.tool("select")
bearcad.ui.tutorial("parameters")
bearcad.ui.wait(8)

local function settled_orb()
  local last = bearcad.ui.tutorial_orb()
  for _ = 1, 20 do
    bearcad.ui.wait(8)
    local now = bearcad.ui.tutorial_orb()
    if not now then return nil end
    if last and math.abs(now.x - last.x) < 1 and math.abs(now.y - last.y) < 1 then
      return now
    end
    last = now
  end
  return last
end

-- Walk to the "click the `width` value" step by hand: it has no assist, so Next would skip it.
local guard = 0
while true do
  guard = guard + 1
  assert(guard < 40, "the width-value step should come up")
  local text = bearcad.ui.tutorial_narration()
  if text and text:find("value in the Parameters pane") then break end
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(4)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(4)
  end
  assert(bearcad.ui.tutorial_step() ~= nil, "the walkthrough ended before that step")
end

-- Put another parameter under it, so a wrong anchor has somewhere else to land: the buggy
-- one always rang the row the pane drew last.
bearcad.parameter("add", "zzz", "5mm")
bearcad.ui.wait(6)
assert(bearcad.count("parameter") == 2,
  "two parameters on the pane, got " .. bearcad.count("parameter"))

local step = bearcad.ui.tutorial_step()
local orb = settled_orb()
assert(orb, "the step rings a Parameters row")
-- `bearcad.ui.click` addresses the viewport, `tutorial_orb` reports a window point (#1692).
local vp = bearcad.ui.viewport()
bearcad.ui.click(orb.x - vp.x, orb.y - vp.y)
bearcad.ui.wait(10)
assert(bearcad.ui.tutorial_step() ~= step,
  string.format("clicking the orb at %.0f,%.0f should open the value it names", orb.x, orb.y))

print("ok: the tutorial orb sits on the parameter row its step names")
bearcad.quit()
