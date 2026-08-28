-- #1640: the technical-drawing walkthrough runs on the drawing workbench — Bear's bubble
-- follows the user onto the sheet, and the assists leave a real page behind.
-- #1646/#1647/#1648/#1649/#1650: it also has to point at the right things while it does.
bearcad.ui.tool("select")
bearcad.ui.tutorial("drawing")
bearcad.ui.wait(8)
assert(bearcad.ui.tutorial_step() == 0, "the drawing tutorial starts on its intro")
assert(bearcad.count("body") >= 1, "it seeds a bracket to draw")

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

-- The steps that point into the Context pane (the view's orientation bear, its Style row)
-- only ring anything if the tutorial overlay is drawn on the drawing workbench too.
local guard = 0
local steps = {}
while bearcad.ui.tutorial_step() ~= nil do
  guard = guard + 1
  assert(guard < 60, "the walkthrough should finish")
  bearcad.ui.wait(5)
  local text = bearcad.ui.tutorial_narration()
  -- Settling the orb costs frames, so only do it for the steps under test.
  local pointed = text and
    (text:find("Elements pane") or text:find("Style") or text:find("on the bear"))
  steps[#steps + 1] = {
    text = text,
    orb = pointed and settled_orb() or bearcad.ui.tutorial_orb(),
  }
  -- An assist satisfies its step and auto-advances; Next is only for the steps without one,
  -- so pressing both would walk past every other step.
  local at = bearcad.ui.tutorial_step()
  bearcad.ui.tutorial_assist()
  bearcad.ui.wait(3)
  if bearcad.ui.tutorial_step() == at then
    bearcad.ui.tutorial_next()
    bearcad.ui.wait(3)
  end
end

local function steps_saying(pattern)
  local found = {}
  for _, s in ipairs(steps) do
    if s.text and s.text:find(pattern) then found[#found + 1] = s end
  end
  return found
end

assert(bearcad.count("drawing") == 1, "the walkthrough leaves one drawing")
local views = bearcad.drawing_views(0)
assert(#views == 4, "front + top + side + three-quarter, got " .. #views)
assert(views[1].orientation == "Front", "the base view is the front")
local aligned, shaded = 0, nil
for _, v in ipairs(views) do
  if v.aligned_to == 0 then
    aligned = aligned + 1
    assert(v.align_lines, v.orientation .. " should show its projection lines")
  end
  if v.style == "shaded" then shaded = v.orientation end
end
assert(aligned == 2, "two views aligned to the front, got " .. aligned)
assert(shaded and shaded:find("-"),
  "the at-an-angle view should be shaded, got " .. tostring(shaded))

-- #1647/#1649: a view is placed by clicking the body in the Elements pane, and the orb
-- rings that row. (The step list itself is checked in tutorial.rs; this is the live ring.)
local elements = bearcad.ui.pane_rect("elements")
assert(type(elements) == "table", "the Elements pane is shown")
local picks = steps_saying("Elements pane")
assert(#picks == 2, "both Add-view steps should send me to the Elements pane, got " .. #picks)
for _, s in ipairs(picks) do
  assert(s.orb, "the Elements-pane step should ring the body row: " .. s.text)
  assert(s.orb.x >= elements.x and s.orb.x <= elements.x + elements.w,
    string.format("the orb should sit in the Elements pane, x=%.0f pane %.0f..%.0f",
      s.orb.x, elements.x, elements.x + elements.w))
  -- Inside the pane, not left over on the toolbar button above it.
  assert(s.orb.y > elements.y and s.orb.y < elements.y + elements.h,
    string.format("and on a row of it, y=%.0f pane %.0f..%.0f",
      s.orb.y, elements.y, elements.y + elements.h))
end

-- #1648: the tutorial parks the base view in the lower left, so the aligned views it asks
-- for next have room above it and to its right.
assert(views[1].pos_y > 0.5 and views[1].pos_x < 0.5,
  string.format("the front view should sit lower-left, at %.2f, %.2f",
    views[1].pos_x, views[1].pos_y))

-- #1640/#1650: the Style step rings the dropdown on the Context pane, not the label beside it.
local context = bearcad.ui.pane_rect("context")
assert(type(context) == "table", "the Context pane is shown")
local bear = steps_saying("on the bear")[1]
assert(bear and bear.orb, "the orientation step should ring the bear on the Context pane")
assert(bear.orb.x > context.x, "the bear ring is on the Context pane")
local style = steps_saying("Style")[1]
assert(style and style.orb, "the Style step should ring something on the drawing workbench")
assert(style.orb.x > context.x + 78,
  string.format("the orb belongs on the Style dropdown, x=%.0f pane starts %.0f",
    style.orb.x, context.x))
assert(style.orb.y > bear.orb.y + 20,
  string.format("and on the Style row, below the bear: style y=%.0f bear y=%.0f",
    style.orb.y, bear.orb.y))

print("ok: the technical-drawing tutorial runs on the drawing workbench")
bearcad.quit()
