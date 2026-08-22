-- #1645: with the Dimension tool on a drawing, clicking two spots that aren't on an edge
-- measures between them. The dimension starts as the direct distance and can be re-measured
-- along one page axis.
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
-- Two spots inside the card but off its edges: the body's front face is a 40x15 rectangle,
-- so points a little way in from the middle land on blank paper inside the card.
bearcad.ui.move(cx - 30, cy - 40)
bearcad.ui.wait(3)
bearcad.ui.click(cx - 30, cy - 40)
bearcad.ui.wait(5)
assert(bearcad.status():find("second point"),
  "the first click should arm the second, got: " .. bearcad.status())

bearcad.ui.move(cx + 30, cy - 60)
bearcad.ui.wait(3)
bearcad.ui.click(cx + 30, cy - 60)
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
  for _, p in ipairs(bearcad.pickers()) do
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
local picked = false
for dx = -60, 60, 10 do
  for dy = -40, 40, 10 do
    bearcad.ui.move(cx + dx, cy - 50 + dy)
    bearcad.ui.wait(2)
    bearcad.ui.click(cx + dx, cy - 50 + dy)
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
bearcad.ui.move(cx - 30, cy - 40)
bearcad.ui.wait(3)
bearcad.ui.click(cx - 30, cy - 40)
bearcad.ui.wait(4)
assert(bearcad.status():find("second point"), "armed again")
bearcad.ui.key("escape")
bearcad.ui.wait(4)
bearcad.ui.move(cx + 30, cy - 60)
bearcad.ui.wait(3)
bearcad.ui.click(cx + 30, cy - 60)
bearcad.ui.wait(5)
assert(bearcad.status():find("second point"),
  "after Esc the next click starts a fresh dimension, got: " .. bearcad.status())

print("ok: two clicks on a drawing measure between them")
bearcad.quit()
