-- #1703/#1709: a step that points at a spot on the drawing page anchored to a whole view
-- card, so its ring came out card-sized — hundreds of points across, swamping the page. And
-- the Dimension step rang the card's middle, where there is no line to click.
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

local guard = 0
local steps = {}
while bearcad.ui.tutorial_step() ~= nil do
  guard = guard + 1
  assert(guard < 60, "the walkthrough should finish")
  bearcad.ui.wait(5)
  local text = bearcad.ui.tutorial_narration()
  local pointed = text and (text:find("front view") or text:find("dimension it"))
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

-- Every ring on the page is a click target, so none of them is bigger than a toolbar button.
for _, pattern in ipairs({ "Click above the front view", "right of the front view", "dimension it" }) do
  local s = step_saying(pattern)
  assert(s and s.orb, "the step should ring something: " .. pattern)
  assert(s.orb.r and s.orb.r < 30,
    string.format("%s: ring radius %.0f is a card, not a click target", pattern, s.orb.r or -1))
end

-- #1709: and the Dimension step's ring sits on a line of the front view, not its middle.
local dim = step_saying("dimension it")
local card = bearcad.ui.drawing_view_rect(0)
assert(type(card) == "table", "the front view has a card on the page")
local cx, cy = card.x + card.w / 2, card.y + card.h / 2
assert(math.abs(dim.orb.x - cx) > 8 or math.abs(dim.orb.y - cy) > 8,
  string.format("the ring should be on a line, not the card centre (%.0f,%.0f)", cx, cy))

print("ok: the drawing walkthrough's page orbs are click-sized and land on lines")
bearcad.quit()
