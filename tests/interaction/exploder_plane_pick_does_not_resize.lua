-- Interaction regression (#986): selecting a construction plane through the Selection
-- Exploder used to *resize* it — a corner jumped out to where the loupe had been.
--
-- The fan redirects the tool's pointer to the picked thing's anchor, and a plane's anchor is
-- its **origin**, which sits within a corner grip's radius once the view is zoomed out far
-- enough. The click selected the plane part-way through the frame, the plane's grips went live
-- in that same frame still holding the redirected pointer, and one got grabbed; holding the
-- button for the rest of a real click then dragged that corner to the cursor.
--
-- The camera distance matters: it is what puts the plane's origin within ~13px of its (5,5)
-- corner. Held with `drag` to the same point rather than `click`, because a click that presses
-- and releases in consecutive frames never gives the grab a frame to move anything.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- A fixed pose, not zoom_fit: the repro depends on the plane's origin landing near its grip.
bearcad.ui.camera{ target = {20, 15, 0}, distance = 600 }
bearcad.ui.wait(5)

local function extent_of(i)
  local e = bearcad.get{ kind = "construction_plane", index = i }.extent
  return string.format("%.2f %.2f %.2f %.2f", e.u_min, e.u_max, e.v_min, e.v_max)
end

local before = {}
for i = 0, 2 do before[i] = extent_of(i) end

bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(4)
bearcad.ui.key("space")
bearcad.ui.wait(6)

-- Find a construction plane's loupe. The fan publishes where it put each one; nothing else can
-- say where to aim.
local plane_leaf
for _, leaf in ipairs(bearcad.exploder()) do
  if leaf.kind == "plane" and leaf.x then
    plane_leaf = leaf
    break
  end
end
assert(plane_leaf, "the fan over the model should offer a construction plane with a loupe")

-- A held click on the loupe, which is what a real one is.
bearcad.ui.drag(plane_leaf.x, plane_leaf.y, plane_leaf.x, plane_leaf.y)
bearcad.ui.wait(12)

-- The click selects the plane...
local picked = false
for _, e in ipairs(bearcad.selection()) do
  if e.kind == "plane" and e.index == plane_leaf.index then picked = true end
end
assert(picked, "clicking a plane's loupe should select that plane")

-- ...and no plane may have been resized on the way.
for i = 0, 2 do
  assert(extent_of(i) == before[i],
    "picking through the fan must not resize plane " .. i .. ": "
      .. before[i] .. " became " .. extent_of(i))
end

print("ok: picking a plane through the exploder selects it without resizing it")
bearcad.quit()
