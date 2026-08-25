-- #1741/#1742/#1743: the Repeat walkthrough's Gap-icon, lock and Distance-icon steps used to
-- ring the row's *value field*, because every Repeat step anchored to the whole row. Each of
-- those three steps points at a different widget on the row, so each has to ring that widget.
bearcad.ui.tool("select")
bearcad.ui.tutorial("repeat")
bearcad.ui.wait(8)

-- The orb glides to its target over many frames, so read it only once it has stopped moving.
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

local guard = 0
local steps = {}
while bearcad.ui.tutorial_step() ~= nil do
  guard = guard + 1
  assert(guard < 60, "the walkthrough should finish")
  bearcad.ui.wait(5)
  local text = bearcad.ui.tutorial_narration()
  -- Settling the orb costs frames, so only do it for the rows under test.
  local pointed = text and
    (text:find("Gap") or text:find("lock") or text:find("Distance"))
  steps[#steps + 1] = {
    text = text,
    orb = pointed and settled_orb() or bearcad.ui.tutorial_orb(),
  }
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(3)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(3)
  end
end

local function step_saying(pattern)
  for _, s in ipairs(steps) do
    if s.text and s.text:find(pattern) then return s end
  end
end

local gap_field = step_saying("in the Gap field")
assert(gap_field and gap_field.orb, "the Gap typing step rings the Gap value field")
local distance_field = step_saying("in Distance")
assert(distance_field and distance_field.orb, "the Distance typing step rings its value field")

-- #1741: the icon sits in the label column, well left of the field it labels.
local gap_icon = step_saying("Click the Gap icon")
assert(gap_icon and gap_icon.orb, "the Gap-icon step rings something")
assert(gap_icon.orb.x < gap_field.orb.x - 40,
  string.format("the Gap icon is left of the Gap field: icon x=%.0f field x=%.0f",
    gap_icon.orb.x, gap_field.orb.x))

-- #1743: same for the Distance measure icon.
local distance_icon = step_saying("Click the Distance icon")
assert(distance_icon and distance_icon.orb, "the Distance-icon step rings something")
assert(distance_icon.orb.x < distance_field.orb.x - 40,
  string.format("the Distance icon is left of the Distance field: icon x=%.0f field x=%.0f",
    distance_icon.orb.x, distance_field.orb.x))

-- #1742: the lock the step asks for is the Offset row's own, at the right end of that row --
-- not the Distance row, and not the value field.
local lock = step_saying("grey lock")
assert(lock and lock.orb, "the lock step rings something")
assert(lock.orb.x > gap_field.orb.x + 40,
  string.format("the lock sits right of the field: lock x=%.0f field x=%.0f",
    lock.orb.x, gap_field.orb.x))
assert(math.abs(lock.orb.y - gap_field.orb.y) < 12,
  string.format("and on the Offset row: lock y=%.0f offset row y=%.0f",
    lock.orb.y, gap_field.orb.y))

print("ok: the Repeat tutorial rings the icon, the field and the lock it names")
bearcad.quit()
