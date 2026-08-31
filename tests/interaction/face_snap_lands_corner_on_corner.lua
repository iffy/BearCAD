-- Interaction regression (#1084): two parts mated corner-to-corner must come out *coincident*.
-- They didn't, because a Face Snap pick took the exact spot under the cursor: clicking near a
-- corner mated near it, a couple of millimetres out, and the corners never met. A face now
-- offers only its nine points (#1083), so a click near a corner takes the corner.
bearcad.new()
bearcad.rect{ width = 30, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.plane{ origin = {60, 0, 0}, normal = {0, 0, 1} }
bearcad.begin_sketch("construction_plane", 3)
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {45, 15, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("move")
bearcad.ui.wait(5)
bearcad.ui.begin_move{ bodies = {1} }
bearcad.ui.wait(5)
bearcad.ui.tool_mode("face_snap")
bearcad.ui.wait(8)

-- Moving side: the block's top face, then *near* its (60, 0) corner — deliberately a few mm
-- off, which is how anyone actually clicks.
bearcad.ui.click_ground(70, 10)
bearcad.ui.wait(8)
bearcad.ui.click_ground(62, 2)
bearcad.ui.wait(8)
-- Fixed side: the slab's top face, then near its (30, 30) corner.
bearcad.ui.click_ground(15, 15)
bearcad.ui.wait(8)
bearcad.ui.click_ground(28, 28)
bearcad.ui.wait(8)

bearcad.ui.key("Enter")
bearcad.ui.wait(8)
local placed = bearcad.body_stats(bearcad.count("body") - 1).bbox
-- The slab's corner is (30, 30, 10). The block's picked corner must land exactly on it, so
-- that corner of the moved block's bounding box sits there — not two millimetres away.
local function touches(lo, hi, v)
  return math.abs(lo - v) < 0.05 or math.abs(hi - v) < 0.05
end
assert(math.abs(placed.min.z - 10) < 0.05,
  "the block sits on the slab's top face (z = 10), got " .. placed.min.z)
assert(touches(placed.min.x, placed.max.x, 30) and touches(placed.min.y, placed.max.y, 30),
  "the two corners should be coincident at (30, 30), block spans x "
  .. placed.min.x .. ".." .. placed.max.x .. ", y " .. placed.min.y .. ".." .. placed.max.y)

print("ok: a corner mates exactly on a corner")
bearcad.quit()
