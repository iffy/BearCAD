-- Interaction regression (#1013): a round wall is one element, not a fan of facets, and its
-- centre line is pickable in its own right — which is what "put this hole on that shaft"
-- and "slide down this bore" are actually about.
bearcad.new()
-- A 40×40×6 plate with a 10 mm hole through the middle.
bearcad.rect{ width = 40, height = 40 }
bearcad.circle{ x = 20, y = 20, r = 5 }
bearcad.extrude{
  boolean = { op = "difference", a = { polygon = {0, 1, 2, 3} }, b = { circle = 0 } },
  distance = 6,
}
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Hide the sketch: its circle centre sits exactly on the bore's axis in this view, and a
-- sketch point outranks everything.
bearcad.set_visible({ kind = "sketch", index = 0 }, "hide")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 20, 0}, distance = 160 }
bearcad.ui.wait(5)

-- The wall reads as one cylinder, with its axis reported alongside.
local cyls = bearcad.body_cylinders(0)
assert(#cyls == 1, "the plate has exactly one round wall, got " .. #cyls)
assert(math.abs(cyls[1].radius - 5) < 0.05, "radius " .. cyls[1].radius)
assert(math.abs(cyls[1].direction[3] - 1) < 0.01, "the hole runs along +Z")

-- Looking straight down the hole, the cursor at its centre is on the centre line.
bearcad.ui.click_ground(20, 20)
bearcad.ui.wait(8)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body_axis",
  "clicking down the bore should select its centre line, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

-- And the wall is no longer offered as a flat face: the plate has six of those — four
-- outside walls and two caps — with the hole counted separately as what it is.
local faces = bearcad.body_faces(0)
assert(#faces == 6, "the plate has six flat faces, got " .. #faces)
for _, f in ipairs(faces) do
  local n = f.normal
  assert(math.abs(math.abs(n[1]) + math.abs(n[2]) + math.abs(n[3]) - 1) < 0.01,
    "every flat face is axis-aligned here")
end

print("ok: a hole is one cylinder, and its centre line is pickable")
bearcad.quit()
