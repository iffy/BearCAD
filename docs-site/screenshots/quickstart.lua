-- Documentation screenshot: the Quickstart's finished angle bracket.
--
-- The same part the Quickstart tutorial builds interactively: an L cross-section
-- with a rounded bend (inner r4 / outer r9 profile fillets), extruded 40 mm,
-- two 5 mm screw holes cut through the base flange, and the flange tip corners
-- rounded. Scripted equivalents stand in for the tutorial's tool clicks.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders
-- (a display, or CI Linux with xvfb + software Vulkan); otherwise the capture
-- never resolves and --timeout force-exits without a PNG, which is expected.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/quickstart.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- L cross-section on the ground plane: legs 50, thickness 5.
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

-- The rounded bend: fillet both bend vertices (inner 4, outer 4+5).
bearcad.fillet_vertex{ point = { kind = "line", index = 2, ["end"] = "end" }, radius = 4 }
bearcad.fillet_vertex{ point = { kind = "line", index = 5, ["end"] = "end" }, radius = 9 }
bearcad.exit_sketch()

local loop = {0, 1, 2, 6, 3, 4, 5, 7}
bearcad.extrude{ polygon = loop, distance = 40, name = "Bracket" }

-- Two screw holes through the base flange.
bearcad.begin_sketch{ kind = "extrude_side", extrusion = 0, profile = "polygon",
                      profile_lines = loop, edge = 0 }
bearcad.circle{ x = 31, y = 10, r = 2.5 }
bearcad.circle{ x = 31, y = 30, r = 2.5 }
bearcad.exit_sketch()
bearcad.extrude{ circles = {0, 1}, distance = -6, body = "cut" }

-- Rounded flange-tip corners. Interactively these are single clicks on the tip
-- edges; from a script the treatable-edge indices depend on internal ordering,
-- so try the low candidates and keep whichever are real square corners (the
-- bend-tangent junctions reject as degenerate).
for k = 0, 3 do
  pcall(bearcad.fillet_edge, { extrusion = 0,
    edge = { kind = "vertical", face = 0, edge = k }, radius = 2.0 })
end

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_right_top")
bearcad.ui.wait(2)
bearcad.ui.screenshot(out)

bearcad.quit()
