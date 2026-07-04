-- Documentation screenshots: the Quickstart's angle bracket, step by step.
--
-- Builds the same part the Quickstart tutorial builds interactively — an L
-- cross-section with a rounded bend, extruded, screw holes cut and countersunk,
-- tip corners rounded — capturing one PNG per tutorial step along the way plus
-- the final hero shot (quickstart.png, the one the harness verifies).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". PNGs are only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local dir = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."
local function shot(name)
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
bearcad.ui.pane("parameters", "show")
bearcad.ui.wait(1)
bearcad.ui.screenshot(dir .. "/quickstart-params.png", true)
bearcad.ui.pane("parameters", "hide")

-- Step 2: the L cross-section, drawn as a closed chain of lines.
bearcad.line{ x = 0,  y = 0,  x1 = 50, y1 = 0 }
bearcad.line{ x = 50, y = 0,  x1 = 50, y1 = 5 }
bearcad.line{ x = 50, y = 5,  x1 = 5,  y1 = 5 }
bearcad.line{ x = 5,  y = 5,  x1 = 5,  y1 = 50 }
bearcad.line{ x = 5,  y = 50, x1 = 0,  y1 = 50 }
bearcad.line{ x = 0,  y = 50, x1 = 0,  y1 = 0 }
for i = 0, 5 do
  local j = (i + 1) % 6
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = j, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end
bearcad.clear_selection()
bearcad.ui.view("top")
shot("quickstart-profile.png")

-- Step 3: the rounded bend — fillet both bend vertices (inner 4, outer 4+5).
bearcad.fillet_vertex{ point = { kind = "line", index = 2, ["end"] = "end" }, radius = 4 }
bearcad.fillet_vertex{ point = { kind = "line", index = 5, ["end"] = "end" }, radius = 9 }
shot("quickstart-bend.png")
bearcad.exit_sketch()

-- Step 4: extrude the profile into the solid bracket.
local loop = {0, 1, 2, 6, 3, 4, 5, 7}
bearcad.extrude{ polygon = loop, distance = 40, name = "Bracket" }
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
shot("quickstart-extrude.png")

-- Step 5: two screw holes cut through the base flange.
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 0 }
bearcad.circle{ x = 31, y = 10, r = 2.5 }
bearcad.circle{ x = 31, y = 30, r = 2.5 }
bearcad.exit_sketch()
bearcad.extrude{ circles = {0, 1}, distance = -6, body = "cut" }
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
shot("quickstart-holes.png")

-- Step 6: countersink the holes — chamfer each hole's outer rim.
for face = 0, 1 do
  bearcad.chamfer_edge{ extrusion = 1,
    edge = { kind = "cap", face = face, edge = 0, top = false }, distance = 1.2 }
end
shot("quickstart-countersink.png")

-- Step 7: round the flange tip corners. Interactively these are single clicks
-- on the tip edges; from a script the treatable-edge indices depend on internal
-- ordering, so try the low candidates and keep whichever are real square
-- corners (the bend-tangent junctions reject as degenerate).
for k = 0, 3 do
  pcall(bearcad.fillet_edge, { extrusion = 0,
    edge = { kind = "vertical", face = 0, edge = k }, radius = 2.0 })
end
shot("quickstart-corners.png")

-- Step 8: the parametric payoff — resize the bracket (the tutorial edits the
-- `width` parameter; the scripted equivalent pushes the extrusion to 60).
bearcad.edit_extrusion{ extrusion = 0, distance = 60 }
shot("quickstart-resize.png")
bearcad.edit_extrusion{ extrusion = 0, distance = 40 }

-- Hero shot (the PNG the screenshot harness verifies).
bearcad.ui.view("corner", "front_right_top")
shot("quickstart.png")

bearcad.quit()
