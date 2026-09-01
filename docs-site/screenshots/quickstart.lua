-- Documentation screenshots: the Quickstart's angle bracket, step by step.
--
-- Builds the same part the Quickstart tutorial builds interactively — a 120-degree
-- bracket drawn *sloppily* and then squared up with geometric constraints and
-- dimensions (including a parameter-driven angle), bend rounded, extruded, screw
-- holes cut and countersunk, tip corners rounded — capturing one PNG per tutorial
-- step plus the final hero shot (quickstart.png, the one the harness verifies).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". PNGs are only written where a real GPU frame renders
-- (a GPU, or CI Linux with the software Vulkan driver); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local dir = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)
local function shot(name)
  bearcad.ui.zoom_fit()
  bearcad.ui.wait(1)
  bearcad.ui.screenshot(dir .. "/" .. name)
end

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- Step 1: parameters. Shown in the Parameters pane (full-window capture).
bearcad.add_parameter("leg", "50mm")
bearcad.add_parameter("width", "40mm")
bearcad.add_parameter("thick", "5mm")
bearcad.add_parameter("hole", "5mm")
bearcad.add_parameter("bend", "4mm")
bearcad.add_parameter("bend_angle", "120deg")
bearcad.ui.pane("parameters", "show")
bearcad.ui.wait(1)
bearcad.ui.screenshot(dir .. "/quickstart-params.png", true)
bearcad.ui.pane("parameters", "hide")

-- Step 2: the profile, drawn *sloppily* — roughly a 120-degree bracket, every
-- segment a little off. Corners still chain (the Line tool snaps each click to
-- the previous segment's end).
local loop = {
  bearcad.line{ x = 0,     y = 0,    x1 = 51,    y1 = 2.5 },  -- outer base
  bearcad.line{ x = 51,    y = 2.5,  x1 = 49.5,  y1 = 7.8 },  -- base end cap
  bearcad.line{ x = 49.5,  y = 7.8,  x1 = 4.5,   y1 = 5.5 },  -- inner base
  bearcad.line{ x = 4.5,   y = 5.5,  x1 = -17.5, y1 = 47 },   -- inner leg
  bearcad.line{ x = -17.5, y = 47,   x1 = -25.5, y1 = 43 },   -- leg end cap
  bearcad.line{ x = -25.5, y = 43,   x1 = 0,     y1 = 0 },    -- outer leg
}
for i = 1, #loop do
  local nxt = loop[i % #loop + 1]
  bearcad.constrain("coincident", loop[i]:endpoint("end"), nxt:start())
end
bearcad.ui.view("top")
shot("quickstart-sloppy.png")

-- Step 3: square it up: geometric constraints first, then exact dimensions on
-- the four lines whose sizes we care about, then the bend angle.
-- Anchor the whole profile: pin the bend corner (outer base start, at 0,0) to the sketch
-- origin so it's fully located, not free to drift.
bearcad.constrain("coincident", loop[1]:start(), { kind = "origin" })
bearcad.constrain("horizontal", loop[1])
bearcad.constrain("parallel", loop[1], loop[3])
bearcad.constrain("parallel", loop[4], loop[6])
bearcad.constrain("perpendicular", loop[2], loop[1])
bearcad.constrain("perpendicular", loop[5], loop[6])
bearcad.dimension{ kind = "line", index = loop[1], value = "leg" }
bearcad.dimension{ kind = "line", index = loop[6], value = "leg" }
bearcad.dimension{ kind = "line", index = loop[2], value = "thick" }
bearcad.dimension{ kind = "line", index = loop[5], value = "thick" }
bearcad.dimension{ kind = "angle", a = loop[1], b = loop[4], value = "bend_angle", sign = 1 }
shot("quickstart-squared.png")

-- Step 4: extrude the profile into the solid bracket.
bearcad.exit_sketch()
bearcad.extrude{ profiles = loop, distance = 40, name = "Bracket" }
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
shot("quickstart-extrude.png")

-- Step 5: the rounded bend — fillet the two swept bend edges (inner bend, outer
-- bend + thick), concentric like bent sheet metal. Vertical edge k is the junction
-- of side walls k and k+1, so the L2/L3 corner is edge 2 and the L5/L0 corner is 5.
bearcad.fillet{ body = 0, edge = { kind = "vertical", face = 0, edge = 2 }, radius = 4 }
bearcad.fillet{ body = 0, edge = { kind = "vertical", face = 0, edge = 5 }, radius = 9 }
shot("quickstart-bend.png")

-- Step 6: two screw holes cut through the base flange, drilled from the inner
-- face (edge 2 = the L2 side wall) — that's where the screw heads will sit.
-- The side face's frame normal points out of the solid, so cutting into the
-- flange is a negative distance (the GUI gesture "drag the handle into the
-- bracket" produces the same sign).
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 2 }
local hole_a = bearcad.circle{ x = 19, y = 10, r = 2.5 }
local hole_b = bearcad.circle{ x = 19, y = 30, r = 2.5 }
bearcad.exit_sketch()
bearcad.extrude{ profiles = {hole_a, hole_b}, distance = -6, body = "cut" }
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
shot("quickstart-holes.png")

-- Step 7: countersink the holes — chamfer each hole's outer rim. Frame the two
-- countersunk holes up close, looking at the inner base face from above (#421),
-- so the cone-shaped seats actually read in the capture.
bearcad.chamfer{
  extrusion = 1,
  edges = {
    { kind = "cap", face = 0, edge = 0, top = false },
    { kind = "cap", face = 1, edge = 0, top = false },
  },
  distance = 1.2,
}
bearcad.ui.view("corner", "back_right_top")
bearcad.ui.wait(1)
bearcad.ui.camera{ target = {28, 5, 20}, distance = 90 }
bearcad.ui.wait(1)
bearcad.ui.screenshot(dir .. "/quickstart-countersink.png")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(1)

-- Step 8: round the flange tip corners (the four remaining sharp junctions).
bearcad.fillet{
  body = 0,
  edges = {
    { kind = "vertical", face = 0, edge = 0 },
    { kind = "vertical", face = 0, edge = 1 },
    { kind = "vertical", face = 0, edge = 3 },
    { kind = "vertical", face = 0, edge = 4 },
  },
  radius = 2.0,
}
shot("quickstart-corners.png")

-- Step 9: engrave a "BearCAD" label on the outer face of the base flange (edge 0, the wall
-- opposite the countersinks), cut 1 mm deep, then turn the view around to read it.
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 0 }
local label = bearcad.text{ text = "BearCAD", x = 6, y = 17, size = 5 }
bearcad.exit_sketch()
bearcad.extrude{ profiles = label, distance = -1, body = "cut" }
bearcad.clear_selection()
bearcad.ui.view("corner", "front_right_bottom")
shot("quickstart-engrave.png")

-- Step 10: the parametric payoff — open the bend flatter by editing bend_angle.
bearcad.set_parameter("bend_angle", "150deg")
shot("quickstart-angle.png")
bearcad.set_parameter("bend_angle", "120deg")

-- Hero shot (the PNG the screenshot harness verifies).
bearcad.ui.view("corner", "front_left_top")
shot("quickstart.png")

bearcad.quit()
