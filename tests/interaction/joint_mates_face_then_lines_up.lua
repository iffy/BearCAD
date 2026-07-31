-- Interaction regression (#1021): a mate reads *put this face on that face, then line this
-- up with that*. Clicking a face on each part places the moving one flush; the line-up rows
-- that follow take away what the face pair leaves free, and stop appearing once nothing is
-- left to pin.
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

-- Both parts named, so the mate is what's left to pick.
bearcad.begin_joint{ a = 0, b = 1, kind = "slider" }
bearcad.ui.wait(8)
assert(picker("Moving face"), "the mate leads with a face on each part")
assert(picker("Fixed face"), "and the face it lands on")
assert(picker("Moving face").focused, "the moving face is picked first")
assert(not picker("Line up 1 moving"),
  "no line-up row appears until the face pair places something")

-- Looking straight down, a click on either part takes its top face.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
assert(#picker("Moving face").items == 1,
  "the first click should fill the moving face, got " .. #picker("Moving face").items)
assert(picker("Fixed face").focused, "the ring moves on to the fixed face")

bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
assert(#picker("Fixed face").items == 1, "the second click should fill the fixed face")

-- With the pair complete a line-up row opens, because two slides and the spin are still
-- free. Corners of each part, picked in the top view.
assert(picker("Line up 1 moving"), "a line-up row opens once the faces are paired")
assert(picker("Line up 1 moving").focused, "and takes the next click")
bearcad.ui.click_ground(60, 0)
bearcad.ui.wait(8)
assert(#picker("Line up 1 moving").items == 1, "the row takes a corner on the moving part")
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
assert(#picker("Line up 1 fixed").items == 1, "and the corner it lines up with")

-- One freedom is left — the spin — so exactly one more row appears.
assert(picker("Line up 2 moving"), "the spin is still free, so another row opens")
assert(not picker("Line up 3 moving"), "one row at a time")

-- Committing applies the placement: the block sits flush on the slab's top face, and the
-- two picked corners' projections coincide — so one end of the block's span lands on the
-- origin corner in both directions, whichever way round the face pair turned it.
bearcad.ui.key("Enter")
bearcad.ui.wait(8)
assert(bearcad.count("joint") == 1, "Enter should commit, status: " .. bearcad.status())
local placed = bearcad.body_stats(1).bbox
assert(math.abs(placed.min[3] - 10) < 0.05,
  "the block should sit on the slab's top face (z = 10), got " .. placed.min[3])
local function touches_zero(lo, hi)
  return math.abs(lo) < 0.05 or math.abs(hi) < 0.05
end
assert(touches_zero(placed.min[1], placed.max[1])
   and touches_zero(placed.min[2], placed.max[2]),
  "the corners should now coincide, block spans x " .. placed.min[1] .. ".." .. placed.max[1]
  .. ", y " .. placed.min[2] .. ".." .. placed.max[2])

print("ok: a mate puts a face on a face, then lines the part up")
bearcad.quit()
