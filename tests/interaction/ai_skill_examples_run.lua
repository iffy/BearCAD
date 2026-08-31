-- Interaction regression (#1602): the agent skill's worked examples actually run.
--
-- The skill is what an AI agent reads before driving BearCAD, so an example that no longer
-- works is worse than no example — the agent follows it and fails. These are the skill's
-- own snippets; change one there, change it here.
bearcad.new()
bearcad.ui.tool("select")

-- Sketching: the drawing verbs open a ground sketch on their own.
bearcad.rect{ x = 0, y = 0, width = 80, height = 50, name = "Box" }
bearcad.circle{ x = 10, y = 5, r = 12, name = "Hole" }
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0 }
bearcad.line{ length = 80, angle = 45 }
bearcad.text{ text = "Hello", x = 10, y = 10, size = 12 }
assert(bearcad.count("line") == 6, "rect is four lines plus two more, got " .. bearcad.count("line"))
assert(bearcad.count("circle") == 1)

-- `r` is a radius, which is the sort of thing an agent gets wrong.
local circle = bearcad.get{ kind = "circle", index = 0 }
assert(circle.r == 12 and circle.radius == 12 and circle.diameter == 24, "r is a radius, diameter is twice it")

-- The quickstart: sketch, extrude, check the volume.
bearcad.new()
local box = bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ profiles = box, distance = 20, name = "Block" }
assert(bearcad.count("body") == 1)
local stats = bearcad.body_stats(0)
assert(math.abs(stats.volume - 80 * 50 * 20) < 200,
  "a solid block's volume should match its box, got " .. stats.volume)

-- Named lookup, as the skill recommends over indices.
bearcad.select(bearcad.find("Block"))
assert(bearcad.selection()[1] ~= nil, "find + select should select the named body")

-- Cutting: sketch on a face, then extrude with body = "cut".
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon",
                      profile_lines = {0, 1, 2, 3}, top = true }
bearcad.circle{ x = 40, y = 25, r = 5 }
bearcad.extrude{ profiles = 0, distance = 20, body = "cut" }
local live = bearcad.count("body") - 1
local cylinders = bearcad.body_cylinders(live)
assert(#cylinders == 1, "the cut should leave one cylindrical hole, got " .. #cylinders)
assert(math.abs(cylinders[1].radius - 5) < 0.01, "hole radius, got " .. cylinders[1].radius)
assert(math.abs(cylinders[1].length - 20) < 0.1, "through-hole length, got " .. cylinders[1].length)
local expected = 80 * 50 * 20 - math.pi * 25 * 20
assert(math.abs(bearcad.body_stats(live).volume - expected) < 200,
  "through-hole volume should be the block minus the cylinder, got " .. bearcad.body_stats(live).volume)

-- Solids: the signatures the skill shows.
bearcad.new()
bearcad.rect{ width = 20, height = 40 }
bearcad.revolve{ profiles = {0, 1, 2, 3}, axis = "y", angle = 180 }
assert(bearcad.count("body") >= 1, "revolve needs an axis and should build a body")

bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ profiles = {0, 1, 2, 3}, distance = 10 }
bearcad.shell{ bodies = {0}, thickness = 2 }
-- An operation consumes its input body and produces a new one, so the index moves on —
-- the skill says so, and this is what proves it.
local shelled = bearcad.count("body") - 1
assert(shelled > 0, "shell should produce a new body")
assert(not pcall(function() bearcad.move_bodies{ bodies = {0}, x = 40 } end),
  "the consumed body cannot be moved")
bearcad.move_bodies{ bodies = {shelled}, x = 40 }

-- Parameters drive geometry, and changing one re-sizes it.
bearcad.new()
bearcad.add_parameter("w", "24")
bearcad.rect{ width = "w", height = "w / 3" }
bearcad.set_parameter("w", "30")
assert(math.abs(bearcad.get{ kind = "line", index = 0 }.length - 30) < 0.001,
  "changing the parameter should re-size the rectangle")

-- Constraints.
bearcad.constrain("parallel",
  { kind = "line", index = 0 }, { kind = "line", index = 2 })
assert(bearcad.count("constraint") > 0, "the constraint should land")

print("ok: the agent skill's examples still run")
bearcad.quit()
