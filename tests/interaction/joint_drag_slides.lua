-- #897: dragging a jointed part with the Select tool moves it through its joint — here a
-- slider along +Y — stopping at its limits, landing as one edit. The held side refuses.
bearcad.new()
-- A 50×10 rail and a 10×10 slab beside it, sharing the corner (50, 0): mated in place,
-- so the joint moves nothing until dragged.
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
  slide_max = 20,
}
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 5, 0}, distance = 220 }
bearcad.ui.wait(5)

local before = bearcad.body_stats(1).bbox.min[2]
-- Select-then-drag (#239): the first click only selects the slab.
bearcad.ui.click_ground(55, 5)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body",
  "the click should select the slab, got " .. (#sel > 0 and sel[1].kind or "nothing"))

-- Dragging the selected slab slides it through the joint; the 20 mm limit stops a 40 mm
-- pull at 20.
bearcad.ui.drag_ground(55, 5, 55, 45)
bearcad.ui.wait(8)
assert(bearcad.status():find("Edited joint"),
  "the drag should land as one edit, status: " .. bearcad.status())
local after = bearcad.body_stats(1).bbox.min[2]
local moved = after - before
assert(moved > 19 and moved < 21,
  "the slab should stop at the 20 mm limit, moved " .. moved)

-- The joint's badge (#899) sits where the mate lands the part — the middle of the two
-- faces it put together, (50, 5) — and clicking it selects the joint itself, outranking
-- the rail behind it.
bearcad.ui.click_ground(50, 5)
bearcad.ui.wait(5)
sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "joint",
  "clicking the badge should select the joint, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

print("ok: a jointed part drags through its slider and stops at the limit")
bearcad.quit()
