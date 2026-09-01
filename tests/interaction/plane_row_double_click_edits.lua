-- #1691/#1692: double-clicking a construction plane row in the Elements pane reopens it in
-- the Plane tool, the way a sketch or an extrusion row does. Driven through real pointer
-- input, aimed by the Angled-plane walkthrough's own orb — so this also checks the orb
-- points at the row the step tells you to double-click.
bearcad.ui.tool("select")
bearcad.ui.tutorial("tilted_plane")
bearcad.ui.wait(8)

-- Walk to the step that says to reopen the plane, doing each step's work with its assist.
local guard = 0
while bearcad.ui.tutorial_step() ~= nil do
  guard = guard + 1
  assert(guard < 60, "the walkthrough should reach the reopen step")
  local text = bearcad.ui.tutorial_narration()
  if text and text:find("Double%-click it in the Elements pane") then break end
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(3)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(3)
  end
end
assert(bearcad.ui.tutorial_step() ~= nil, "the reopen step exists")

-- The orb glides to its target, so read it once it has stopped moving.
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

local orb = settled_orb()
assert(orb, "the reopen step rings the plane's row in Elements")

bearcad.ui.double_click(orb)

assert(bearcad.ui.tool() == "construction_plane",
  "double-clicking the plane row reopens it in the Plane tool, tool is "
    .. tostring(bearcad.ui.tool()))

print("ok: double-clicking a plane row opens it for editing")
bearcad.quit()
