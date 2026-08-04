-- Interaction regression (#1081): a mate is picked in **two** steps a side — first a face,
-- then a point on *that* face. Adjacent faces share their corners, so a corner click on its
-- own can't say which face was meant; naming the face first is what makes the point
-- unambiguous. The picker must not go straight to asking for a point.
bearcad.new()
-- A 30×30×10 base slab at the origin and a 20×20×10 block parked 60 mm away, so a
-- placement that fires is unmistakable.
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ origin = {60, 0, 0}, normal = {0, 0, 1} }
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {45, 15, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("joint")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
end

-- Both parts named, so the placement is what's left to pick.
bearcad.begin_joint{ a = 0, b = 1, kind = "slider" }
bearcad.ui.wait(8)
assert(picker("Moving face"), "the mate leads with a face on each part")
assert(picker("Fixed face"), "and the face it lands on")
assert(picker("Moving face").focused, "the moving face is picked first")

-- Looking straight down, a click on either part takes its top face. That is the FIRST of
-- two picks for this side: the row fills, but the side is not done and focus stays put.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
assert(#picker("Moving face").items == 1,
  "the first click takes the face, got " .. #picker("Moving face").items)
assert(picker("Moving face").focused,
  "the moving side still wants its point, so focus stays on it")
assert(#picker("Fixed face").items == 0, "the fixed side is untouched")

-- The second click on that same face takes the point on it, and only then does the side
-- finish and the ring move on.
bearcad.ui.click_ground(58, -2)
bearcad.ui.wait(8)
assert(#picker("Moving face").items == 2,
  "the second click takes a point on that face, got " .. #picker("Moving face").items)
assert(picker("Fixed face").focused, "now the ring moves on to the fixed face")

-- Same two steps on the fixed side.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Fixed face").items == 1, "the fixed side takes its face first")
assert(picker("Fixed face").focused, "and still wants its point")
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Fixed face").items == 2, "then a point on that face")

bearcad.ui.key("Enter")
bearcad.ui.wait(8)
assert(bearcad.count("joint") == 1, "Enter should commit, status: " .. bearcad.status())
local placed = bearcad.body_stats(1).bbox
assert(math.abs(placed.min[3] - 10) < 0.05,
  "the block should sit on the slab's top face (z = 10), got " .. placed.min[3])

print("ok: a mate takes a face, then a point on that face")
bearcad.quit()
