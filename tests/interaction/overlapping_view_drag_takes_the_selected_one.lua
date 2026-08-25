-- #1707: two projections overlap on the page. A press in the overlap went to whichever card
-- came first in the list — the one behind — so dragging the front card moved the wrong one.
-- The card on top wins now, and the card you have selected outranks whatever it overlaps.
bearcad.new()
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "right" }
-- Park them almost on top of each other, so a press lands inside both cards.
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.45, y = 0.5 }
bearcad.drawing_move_view{ drawing = d, view = 1, x = 0.52, y = 0.5 }
bearcad.ui.tool("select")
bearcad.ui.wait(8)

local card0 = bearcad.ui.drawing_view_rect(0)
local card1 = bearcad.ui.drawing_view_rect(1)
assert(card0 and card1, "both cards are on the page")
local lo = math.max(card0.x, card1.x)
local hi = math.min(card0.x + card0.w, card1.x + card1.w)
assert(hi - lo > 20, string.format("the cards should overlap, %.0f..%.0f", lo, hi))
local overlap_x = (lo + hi) / 2
local overlap_y = math.max(card0.y, card1.y) + 40
local vp = bearcad.ui.viewport()

local function drag_the_overlap()
  local before = bearcad.drawing_views(d)
  bearcad.ui.drag(overlap_x - vp.x, overlap_y - vp.y, overlap_x - vp.x, overlap_y - vp.y + 90)
  bearcad.ui.wait(10)
  local after = bearcad.drawing_views(d)
  return math.abs(after[1].pos_y - before[1].pos_y), math.abs(after[2].pos_y - before[2].pos_y)
end

-- With nothing selected, the card drawn on top — the later one — takes the press.
local moved0, moved1 = drag_the_overlap()
assert(moved1 > 0.02 and moved0 < 0.005,
  string.format("the card on top should move: front dy=%.3f, top dy=%.3f", moved0, moved1))

-- Now select the card *behind*, by clicking the strip of it the other one doesn't cover.
bearcad.ui.click(card0.x + 12 - vp.x, overlap_y - vp.y)
bearcad.ui.wait(8)
moved0, moved1 = drag_the_overlap()
assert(moved0 > 0.02 and moved1 < 0.005,
  string.format("the selected card should move even from under the other: behind dy=%.3f, top dy=%.3f",
    moved0, moved1))

print("ok: a drag over overlapping views takes the selected one, else the one on top")
bearcad.quit()
