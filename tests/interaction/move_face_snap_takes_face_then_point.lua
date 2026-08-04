-- Interaction regression (#1081): Face Snap picks each side in **two** steps — a face, then
-- a point on *that* face. Adjacent faces share their corners, so a corner click alone can't
-- say which face was meant. The tool must ask for the face first, and while it is asking,
-- no point picker may be live: hovering must offer faces, not corners.
bearcad.new()
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
bearcad.ui.tool("move")
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
end

bearcad.begin_move{ bodies = {1} }
bearcad.ui.wait(5)
bearcad.ui.tool_mode("face_snap")
bearcad.ui.wait(8)

-- Face Snap's own two rows, and *only* those: a live point picker beside them would have the
-- hover offering corners while the tool is asking for a face.
assert(picker("Moving face"), "Face Snap leads with a face on each side")
assert(picker("Fixed face"), "and the face it lands on")
assert(picker("Moving face").focused, "the moving face is picked first")
assert(not picker("Start point A"), "Point Snap's rows have no business being live here")
assert(not picker("End point A"), "nor its end point")

-- Click one: the face. The row fills, but the side is not done — focus stays put.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
assert(#picker("Moving face").items == 1,
  "the first click takes the face, got " .. #picker("Moving face").items)
assert(picker("Moving face").focused, "the side still wants its point, so focus stays")
assert(#picker("Fixed face").items == 0, "the other side is untouched")

-- Stage two offers nine points on a rectangular face: four corners, four edge midpoints and
-- the centre (#1083). Hovering an edge midpoint highlights it — the block spans x 60..80,
-- y 0..20, so (70, 0) is the middle of its near edge.
bearcad.ui.move_ground(70, 0)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h and h.kind == "body_vertex",
  "an edge midpoint should highlight, got " .. tostring(h and h.kind))

-- And a corner reached from **outside** the face still highlights and picks: a corner sits on
-- the outline, so requiring the cursor be inside made the very points you aim at unpickable.
bearcad.ui.move_ground(58, -2)
bearcad.ui.wait(8)
local h = bearcad.hovered()
assert(h and h.kind == "body_vertex",
  "a corner approached from off the face should highlight, got " .. tostring(h and h.kind))

-- Click two: one of those nine points — the corner at (60, 0), taken from off the face. Only
-- now does the side finish and the ring move on. A spot that is *not* one of the nine is not
-- a pick: a mate lands on a feature of the face, not wherever the cursor happened to be.
bearcad.ui.click_ground(58, -2)
bearcad.ui.wait(8)
assert(#picker("Moving face").items == 2,
  "the second click takes a point on that face, got " .. #picker("Moving face").items)
assert(picker("Fixed face").focused, "now the ring moves on to the fixed side")

-- The same two steps on the fixed side.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Fixed face").items == 1, "the fixed side takes its face first")
assert(picker("Fixed face").focused, "and still wants its point")
-- The slab's top face spans 0..30 both ways, so (15, 15) is its centre — one of its nine.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Fixed face").items == 2, "then a point on that face")

-- Committing lands the moving block's picked spot on the fixed slab's, surfaces together.
bearcad.ui.key("Enter")
bearcad.ui.wait(8)
-- A Move makes an output body; the input becomes shadow, so read the last one. The block's
-- picked corner lands on the slab's picked centre, surfaces together.
local placed = bearcad.body_stats(bearcad.count("body") - 1).bbox
assert(math.abs(placed.min[3] - 10) < 0.05,
  "the block should sit on the slab's top face (z = 10), got " .. placed.min[3])
-- Landing flips the part (the two normals end up opposed), so which corner of the bounding
-- box the picked one becomes depends on the turn; what must hold is that a corner is there.
local function touches(lo, hi, v)
  return math.abs(lo - v) < 0.05 or math.abs(hi - v) < 0.05
end
assert(touches(placed.min[1], placed.max[1], 15) and touches(placed.min[2], placed.max[2], 15),
  "its picked corner should land on the slab's centre (15, 15), spans x "
  .. placed.min[1] .. ".." .. placed.max[1] .. ", y " .. placed.min[2] .. ".." .. placed.max[2])

print("ok: Face Snap takes a face, then a point on that face")
bearcad.quit()
