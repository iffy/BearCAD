-- Documentation screenshot: the Joint tool's Context pane with help mode on.
--
-- Help mode is the documentation for these controls: it draws a note beside each row
-- saying what that row wants, and a pane capture widens to include them. The tool is
-- armed mid-pick (a slider with its A pair mated) so the pickers show real content.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-joint.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ width = 30, height = 20, name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5, name = "Base" }
bearcad.rect{ x = 40, y = 0, width = 25, height = 8, name = "Arm" }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5, name = "Arm" }
bearcad.exit_sketch()

-- Faces and edges are named by their own geometry, which `body_faces`/`body_edges` report.
local function near(p, q)
  return math.abs(p[1] - q[1]) < 0.01 and math.abs(p[2] - q[2]) < 0.01
     and math.abs(p[3] - q[3]) < 0.01
end
local function face_facing(body, n)
  for _, f in ipairs(bearcad.body_faces(body)) do
    if near(f.normal, n) then return f end
  end
  error("no face of body " .. body .. " faces {" .. table.concat(n, ", ") .. "}")
end
local function edge_between(body, a, b)
  for _, e in ipairs(bearcad.body_edges(body)) do
    if (near(e.edge[1], a) and near(e.edge[2], b))
       or (near(e.edge[1], b) and near(e.edge[2], a)) then return e end
  end
  error("no edge of body " .. body .. " runs between those corners")
end
bearcad.begin_joint{
  a = 0, b = 1, kind = "slider",
  face = { moving = face_facing(1, {-1, 0, 0}), fixed = face_facing(0, {1, 0, 0}) },
  slide_max = 20,
}
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
