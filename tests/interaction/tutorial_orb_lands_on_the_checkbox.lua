-- #1732: the Curves walkthrough's "tick Curve" step anchored to the whole Context row, so the
-- orb sat in the gap between the label and the tick box. Clicking where the orb points has to
-- do the step's work.
bearcad.ui.tool("select")
bearcad.ui.tutorial("curves")
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

-- Walk to the tick step by hand, so the assist never does it for us.
local guard = 0
while true do
  guard = guard + 1
  assert(guard < 40, "the tick step should come up")
  local text = bearcad.ui.tutorial_narration()
  if text and text:find("Now tick") then break end
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(4)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(4)
  end
  assert(bearcad.ui.tutorial_step() ~= nil, "the walkthrough ended before the tick step")
end

local step = bearcad.ui.tutorial_step()
local orb = settled_orb()
assert(orb, "the tick step rings something")
-- `bearcad.ui.click` addresses the viewport, `tutorial_orb` reports a window point (#1692).
local vp = bearcad.ui.viewport()
bearcad.ui.click(orb.x - vp.x, orb.y - vp.y)
bearcad.ui.wait(10)
assert(bearcad.ui.tutorial_step() ~= step,
  string.format("clicking the orb at %.0f,%.0f should tick Curve and advance the step",
    orb.x, orb.y))

print("ok: the tutorial orb sits on the checkbox it asks you to tick")
bearcad.quit()
