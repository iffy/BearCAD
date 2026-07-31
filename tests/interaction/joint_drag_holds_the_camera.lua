-- #905: dragging a part through its joint never moves the camera — auto-zoom stands down
-- for the drag instead of chasing the part around the viewport.
bearcad.new()
bearcad.rect{ width = 50, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.rect{ x = 50, y = 0, width = 10, height = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
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

-- Mate in place: the slab's left face onto the rail's right face, lined up by their shared
-- bottom edge, which is what gives the slider its +Y direction to travel along.
local mate = {
  face = { moving = face_facing(1, {-1, 0, 0}), fixed = face_facing(0, {1, 0, 0}) },
  line_up = {
    {
      moving = edge_between(1, {50, 0, 0}, {50, 10, 0}),
      fixed  = edge_between(0, {50, 0, 0}, {50, 10, 0}),
    },
  },
}
bearcad.joint{
  a = 0, b = 1, kind = "slider",
  face = mate.face, line_up = mate.line_up,
  slide_max = 200,
}
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 5, 0}, distance = 220 }
-- Auto-zoom on: it's what would otherwise chase the moving part.
bearcad.ui.auto_zoom(true)
bearcad.ui.wait(5)

local before = bearcad.ui.camera{}
-- A long pull: the slab ends up well outside the framed view.
bearcad.ui.drag_ground(55, 5, 55, 120)
bearcad.ui.wait(10)
local after = bearcad.ui.camera{}
local function same(a, b, what)
  assert(math.abs(a - b) < 0.01,
    what .. " should not move while dragging a joint: " .. a .. " -> " .. b)
end
same(before.distance, after.distance, "the camera distance")
same(before.yaw, after.yaw, "the camera yaw")
same(before.pitch, after.pitch, "the camera pitch")
for i = 1, 3 do same(before.target[i], after.target[i], "the camera target") end

print("ok: a joint drag leaves the camera alone")
bearcad.quit()
