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
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local dir = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."
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
bearcad.parameter("add", "leg", "50mm")
bearcad.parameter("add", "width", "40mm")
bearcad.parameter("add", "thick", "5mm")
bearcad.parameter("add", "hole", "5mm")
bearcad.parameter("add", "bend", "4mm")
bearcad.parameter("add", "bend_angle", "120deg")
bearcad.ui.pane("parameters", "show")
bearcad.ui.wait(1)
bearcad.ui.screenshot(dir .. "/quickstart-params.png", true)
bearcad.ui.pane("parameters", "hide")

-- Step 2: the profile, drawn *sloppily* — roughly a 120-degree bracket, every
-- segment a little off. Corners still chain (the Line tool snaps each click to
-- the previous segment's end).
bearcad.line{ x = 0,     y = 0,    x1 = 51,    y1 = 2.5 }  -- 0 outer base
bearcad.line{ x = 51,    y = 2.5,  x1 = 49.5,  y1 = 7.8 }  -- 1 base end cap
bearcad.line{ x = 49.5,  y = 7.8,  x1 = 4.5,   y1 = 5.5 }  -- 2 inner base
bearcad.line{ x = 4.5,   y = 5.5,  x1 = -17.5, y1 = 47 }   -- 3 inner leg
bearcad.line{ x = -17.5, y = 47,   x1 = -25.5, y1 = 43 }   -- 4 leg end cap
bearcad.line{ x = -25.5, y = 43,   x1 = 0,     y1 = 0 }    -- 5 outer leg
for i = 0, 5 do
  local j = (i + 1) % 6
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = j, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end
bearcad.clear_selection()
bearcad.ui.view("top")
shot("quickstart-sloppy.png")

-- Step 3: square it up: geometric constraints first, then exact dimensions on
-- the four lines whose sizes we care about, then the bend angle.
local function geo(kind, a, b)
  bearcad.select{ kind = "line", index = a }
  if b then bearcad.select({ kind = "line", index = b }, true) end
  bearcad.add_geometric_constraint(kind)
  bearcad.clear_selection()
end
-- Anchor the whole profile: pin the bend corner (line 0's start, at 0,0) to the sketch
-- origin so it's fully located, not free to drift.
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
bearcad.select({ kind = "origin" }, true)
bearcad.add_geometric_constraint("coincident")
bearcad.clear_selection()
geo("horizontal", 0)
geo("parallel", 0, 2)
geo("parallel", 3, 5)
geo("perpendicular", 1, 0)
geo("perpendicular", 4, 5)
bearcad.add_constraint({ kind = "line", index = 0 }, "leg")
bearcad.add_constraint({ kind = "line", index = 5 }, "leg")
bearcad.add_constraint({ kind = "line", index = 1 }, "thick")
bearcad.add_constraint({ kind = "line", index = 4 }, "thick")
bearcad.add_angle_constraint{ a = 0, b = 3, value = "bend_angle", sign = 1 }
shot("quickstart-squared.png")

-- Step 4: extrude the profile into the solid bracket.
bearcad.exit_sketch()
local loop = {0, 1, 2, 3, 4, 5}
bearcad.extrude{ polygon = loop, distance = 40, name = "Bracket" }
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
shot("quickstart-extrude.png")

-- Step 5: the rounded bend — fillet the two swept bend edges (inner bend, outer
-- bend + thick), concentric like bent sheet metal. Vertical edge k is the junction
-- of side walls k and k+1, so the L2/L3 corner is edge 2 and the L5/L0 corner is 5.
bearcad.fillet_edge{ extrusion = 0, edge = { kind = "vertical", face = 0, edge = 2 }, radius = 4 }
bearcad.fillet_edge{ extrusion = 0, edge = { kind = "vertical", face = 0, edge = 5 }, radius = 9 }
shot("quickstart-bend.png")

-- Step 6: two screw holes cut through the base flange, drilled from the inner
-- face (edge 2 = the L2 side wall) — that's where the screw heads will sit.
-- The side face's frame normal points out of the solid, so cutting into the
-- flange is a negative distance (the GUI gesture "drag the handle into the
-- bracket" produces the same sign).
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 2 }
bearcad.circle{ x = 19, y = 10, r = 2.5 }
bearcad.circle{ x = 19, y = 30, r = 2.5 }
bearcad.exit_sketch()
bearcad.extrude{ circles = {0, 1}, distance = -6, body = "cut" }
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
shot("quickstart-holes.png")

-- Step 7: countersink the holes — chamfer each hole's outer rim. Frame the two
-- countersunk holes up close, looking at the inner base face from above (#421),
-- so the cone-shaped seats actually read in the capture.
for face = 0, 1 do
  bearcad.chamfer_edge{ extrusion = 1,
    edge = { kind = "cap", face = face, edge = 0, top = false }, distance = 1.2 }
end
bearcad.ui.view("corner", "back_right_top")
bearcad.ui.wait(1)
bearcad.ui.camera{ target = {28, 5, 20}, distance = 90 }
bearcad.ui.wait(1)
bearcad.ui.screenshot(dir .. "/quickstart-countersink.png")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(1)

-- Step 8: round the flange tip corners (the four remaining sharp junctions).
for _, k in ipairs({0, 1, 3, 4}) do
  bearcad.fillet_edge{ extrusion = 0,
    edge = { kind = "vertical", face = 0, edge = k }, radius = 2.0 }
end
shot("quickstart-corners.png")

-- Step 9: engrave a "BearCAD" label on the outer face of the base flange (edge 0, the wall
-- opposite the countersinks), cut 1 mm deep, then turn the view around to read it.
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 0 }
bearcad.text{ text = "BearCAD", x = 6, y = 17, size = 5 }
bearcad.exit_sketch()
bearcad.extrude{ text = 0, distance = -1, body = "cut" }
bearcad.clear_selection()
bearcad.ui.view("corner", "front_right_bottom")
shot("quickstart-engrave.png")

-- Step 10: the parametric payoff — open the bend flatter by editing the
-- bend_angle parameter (index 5 in the Parameters pane).
bearcad.parameter("value", 5, "150deg")
shot("quickstart-angle.png")
bearcad.parameter("value", 5, "120deg")

-- Hero shot (the PNG the screenshot harness verifies).
bearcad.ui.view("corner", "front_left_top")
shot("quickstart.png")

bearcad.quit()
