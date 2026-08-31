-- #1645: with the Dimension tool on a drawing, clicking two spots that aren't on an edge
-- measures between them. The dimension starts as the direct distance and can be re-measured
-- along one page axis. The two spots are found by a fine vertical scan out from the card's
-- centre — the blank bands just above and below the projected body are where a click arms a
-- free measurement — so the test holds at any window size.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
local cx, cy = vp.width / 2, vp.height / 2

bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

-- Click up (or down) the page from the centre until a click arms a free measurement, then
-- cancel. Runs only while nothing is armed, so the status readout is meaningful: a spot on
-- an edge toggles that edge's dimension instead, and every miss re-arms the Dimension tool
-- (Esc cancels back to Select).
local function find_arming_spot(sign, dx0)
  for k = 2, 40 do
    local x, y = math.floor(cx + dx0), math.floor(cy + sign * k * 8)
    bearcad.ui.move(x, y)
    bearcad.ui.wait(2)
    bearcad.ui.click(x, y)
    bearcad.ui.wait(4)
    if bearcad.status():find("second point") then
      bearcad.ui.key("escape")
      bearcad.ui.wait(3)
      bearcad.ui.tool("dimension")
      bearcad.ui.wait(3)
      return { x, y }
    end
    bearcad.ui.key("escape")
    bearcad.ui.wait(2)
    bearcad.ui.tool("dimension")
    bearcad.ui.wait(2)
  end
  return nil
end

-- Offset in x too, so the pair measures a real horizontal separation for the axis switch.
local first = find_arming_spot(-1, -30)
assert(first, "expected a spot above the body that arms a free measurement")
local second = find_arming_spot(1, 0)
assert(second, "expected a spot below the body that arms a free measurement")

bearcad.ui.move(first[1], first[2])
bearcad.ui.wait(3)
bearcad.ui.click(first[1], first[2])
bearcad.ui.wait(5)
assert(bearcad.status():find("second point"),
  "the first click should arm the second, got: " .. bearcad.status())

bearcad.ui.move(second[1], second[2])
bearcad.ui.wait(3)
bearcad.ui.click(second[1], second[2])
bearcad.ui.wait(6)
assert(bearcad.status():find("dimension"),
  "the second click should add the dimension, got: " .. bearcad.status())

-- Re-measure it along one axis: the value must change to the horizontal separation only.
bearcad.drawing_point_dimension_axis{ drawing = d, view = 0, index = 0, axis = "horizontal" }
bearcad.ui.wait(4)
assert(bearcad.status():find("horizontal"),
  "changing the axis should say so, got: " .. bearcad.status())

-- Its label is a click target for the Select tool, so it can be re-measured or deleted later.
local function selection_picker()
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == "Selection" then return p end
  end
  return nil
end
bearcad.ui.tool("select")
bearcad.ui.wait(5)
bearcad.ui.click(cx + 90, cy + 90)   -- blank page: clears the selection
bearcad.ui.wait(5)
local sel = selection_picker()
assert(sel and #sel.items == 0, "clicking blank paper clears the selection")
-- The dimension's label sits between the two picked points, pushed clear of them.
local mx = (first[1] + second[1]) / 2
local my = (first[2] + second[2]) / 2
local picked = false
for dx = -80, 80, 10 do
  for dy = -60, 60, 10 do
    bearcad.ui.move(mx + dx, my + dy)
    bearcad.ui.wait(2)
    bearcad.ui.click(mx + dx, my + dy)
    bearcad.ui.wait(4)
    local s = selection_picker()
    if s and #s.items == 1 and s.items[1].kind == "drawing_dimension" then
      picked = true
      break
    end
  end
  if picked then break end
end
assert(picked, "clicking a free dimension's label should select it")

bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

-- Esc must drop a half-made one rather than leaving it armed forever.
bearcad.ui.move(first[1], first[2])
bearcad.ui.wait(3)
bearcad.ui.click(first[1], first[2])
bearcad.ui.wait(4)
assert(bearcad.status():find("second point"), "armed again")
bearcad.ui.key("escape")
bearcad.ui.wait(4)
bearcad.ui.tool("dimension")   -- Esc cancelled back to Select; keep dimensioning
bearcad.ui.wait(4)
-- The second spot arms again: Esc dropped the half-made pick entirely.
bearcad.ui.move(second[1], second[2])
bearcad.ui.wait(3)
bearcad.ui.click(second[1], second[2])
bearcad.ui.wait(5)
assert(bearcad.status():find("second point"),
  "after Esc the next click starts a fresh dimension, got: " .. bearcad.status())

print("ok: two clicks on a drawing measure between them")
bearcad.quit()
