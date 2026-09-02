-- #1639: a point snapped onto a mesh face's edge must stay on *that* edge. The face's
-- boundary loop is walked to index `ConstraintLine::FaceEdge`, and walking it from a hash
-- map's order started it at a different corner every call — so the next solve re-pointed the
-- constraint at some other edge and the line jumped somewhere else on the wall.
-- The document is the reporter's: an L-shaped +X wall (a 20x20x80 column with a 20x20x60 arm),
-- whose sketch 2 sits on that wall as a body mesh face.
bearcad.open("tests/fixtures/issue_1639.json")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.open_sketch(2)
bearcad.ui.view("right")
bearcad.ui.wait(6)
bearcad.ui.camera{ target = {20, 30, 40}, distance = 150 }
bearcad.ui.wait(8)
bearcad.ui.tool("line")
bearcad.ui.wait(3)

-- The wall is the x = 20 plane; the sketch's own frame puts (0, 0) at the face centre,
-- u along +y and v along +z. Read the centre off the sketch rather than hard-coding it —
-- it is a property of the face, and a change to how a face centre is measured moved it.
local origin = bearcad.get{ kind = "sketch", index = 2 }.origin
local function place(y, z)
  bearcad.ui.move_world(20, y, z)
  bearcad.ui.wait(4)
  bearcad.ui.click_world(20, y, z)
  bearcad.ui.wait(5)
end
local function u_of(y) return y - origin.y end

local first = bearcad.count("line")
-- Start in open space on the column, and end half a millimetre shy of the wall's y = 0 edge
-- so the endpoint snaps onto it. That edge is the face's own boundary and nothing else —
-- no projected body edge lies along it to snap to instead.
place(10, 62)
place(0.6, 62)
bearcad.ui.key("escape")
bearcad.ui.wait(6)

local _, _, x1, y1 = bearcad.line_endpoints(first)
assert(math.abs(x1 - u_of(0)) < 0.2,
  string.format("the endpoint should snap onto the y = 0 edge (u = %.2f), got u = %.2f",
    u_of(0), x1))
local held_x, held_y = x1, y1

-- Save and reopen: a fresh solve is where the snapped endpoint used to be yanked onto
-- whichever edge the boundary walk happened to index that time.
local saved = os.tmpname() .. ".bearcad"
bearcad.save(saved)
bearcad.ui.wait(4)
bearcad.open(saved)
bearcad.ui.wait(8)
os.remove(saved)

local _, _, x2, y2 = bearcad.line_endpoints(first)
assert(math.abs(x2 - held_x) < 0.2 and math.abs(y2 - held_y) < 0.2,
  string.format("the snapped endpoint moved from (%.2f, %.2f) to (%.2f, %.2f) across a reload",
    held_x, held_y, x2, y2))

print("ok: a point snapped to a mesh face edge stays on that edge")
bearcad.quit()
