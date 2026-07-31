-- #894: the Joint tool commits its armed picks on Enter — the same keyboard path every
-- other tool ends with — and Esc drops an in-progress joint instead of committing it.
bearcad.new()
bearcad.rect{ width = 10, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.rect{ x = 40, y = 0, width = 10, height = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

-- Arm the tool with both parts mated in place, then Enter commits it.
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
}
bearcad.ui.wait(3)
assert(bearcad.count("joint") == 0, "begin_joint must not commit")
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(bearcad.count("joint") == 1,
  "Enter should commit the armed joint, status: " .. bearcad.status())

-- Arm another and Esc it away: nothing further lands.
bearcad.begin_joint{ a = 0, b = 1, kind = "rigid" }
bearcad.ui.wait(3)
bearcad.ui.key("Escape")
bearcad.ui.wait(3)
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(bearcad.count("joint") == 1,
  "Esc should have dropped the second joint, status: " .. bearcad.status())

print("ok: Enter commits the armed joint; Esc drops it")
bearcad.quit()
