-- #957: the Exploder decided what to fan from a hand-written `match tool`, so it knew nothing
-- about *which picker is armed*. It now prunes the crowd with the focused picker's own filter,
-- and it does not open at all when nothing is armed.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

local function kinds_at(x, y)
  bearcad.ui.move_ground(x, y)
  bearcad.ui.wait(3)
  bearcad.ui.key("space")
  bearcad.ui.wait(5)
  local seen = {}
  local n = 0
  for _, leaf in ipairs(bearcad.ui.exploder()) do
    seen[leaf.kind] = true
    n = n + 1
  end
  bearcad.ui.key("escape")
  bearcad.ui.wait(4)
  return seen, n
end

-- The Select tool takes everything, so a crowded corner fans the corner, its edges and the
-- whole body — the baseline the picker-driven filter has to preserve.
bearcad.ui.tool("select")
bearcad.ui.wait(5)
local seen, n = kinds_at(40, 30)
assert(n > 1, "Select should fan a crowd at the corner, got " .. n)
assert(seen["body_vertex"] and seen["body_edge"] and seen["body"],
  "Select's fan should still hold the corner, its edges and the body")

-- The Extrude tool's Profile picker takes analytic faces and nothing else, so the same corner
-- must not offer the corner or its edges — they are picks it could never use (#560).
bearcad.ui.tool("extrude")
bearcad.ui.wait(5)
seen, n = kinds_at(40, 30)
assert(n > 0, "the corner has profiles Extrude can take, so the fan should open")
assert(seen["face"], "and they should be faces, not something else")
assert(not seen["body_vertex"], "an Extrude fan should not offer a corner")
assert(not seen["body_edge"], "nor an edge")
assert(not seen["body"], "nor a whole body")

-- The Slice tool's two pickers take different things. Targets takes bodies; Cutters takes
-- planes and flat faces, so arming it must change what the fan offers at the same spot.
bearcad.ui.tool("slice")
bearcad.ui.wait(5)
local with_targets = kinds_at(20, 15)
assert(with_targets["body"], "Slice's Targets picker takes the body under the cursor")
bearcad.ui.picker_focus("Cutters")
bearcad.ui.wait(5)
local with_cutters = kinds_at(20, 15)
assert(not with_cutters["body"],
  "Cutters cannot take a body, so its fan must not offer one")

-- Inside a sketch a draw tool draws rather than picks, so nothing is armed: there is no pick
-- for the fan to disambiguate and Space leaves it closed. (Outside a sketch the same tool
-- *does* pick — the face to sketch on — and its fan opens.)
bearcad.ui.tool("sketch")
bearcad.ui.wait(5)
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(8)
bearcad.ui.tool("rectangle")
bearcad.ui.wait(5)
local _, closed = kinds_at(40, 30)
assert(closed == 0, "with no picker armed the fan should not open, got " .. closed)

print("ok: the fan offers what the armed picker can take")
bearcad.quit()
