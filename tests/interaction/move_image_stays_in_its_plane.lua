-- #1601/#1611: a Move on a tracing image is planar — only the two in-plane axes slide,
-- only the plane normal turns, and Face Snap is off the menu — and dragging a gizmo
-- previews the move on the quad itself before Enter commits it.
bearcad.new()
bearcad.import_image("rectangle_preview.png")
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
-- Orthographic top: the ground-plane image and its gizmo handles share screen points,
-- so drag_ground lands on a handle. 410x1144 px at 1 px = 1 mm, centred on the origin.
bearcad.ui.camera{ target = {0, 0, 0}, distance = 3000, projection = "orthographic" }
bearcad.ui.wait(5)

bearcad.ui.tool("select")
bearcad.ui.tool("move")
bearcad.ui.wait(8)

local bodies = bearcad.picker("Bodies")
assert(bodies and #bodies.items == 1 and bodies.items[1].kind == "image",
  "the Move tool should hold the one tracing image")
assert(bearcad.ui.tool_mode() == "free", "an image-only move lands in Free (#1587)")

local function gizmo(name)
  return bearcad.gizmo(name)
end

-- The image sits on the ground (XY) plane, so Z is the axis that isn't in it.
assert(gizmo("move_x") and gizmo("move_y"), "in-plane slides should stay on offer")
assert(not gizmo("move_z"), "an XY image must not offer an out-of-plane Z slide (#1601)")
assert(gizmo("move_rz"), "an XY image should turn about its plane normal (#1601)")
assert(not gizmo("move_rx") and not gizmo("move_ry"),
  "an XY image must not offer a turn that tilts it out of its plane (#1601)")
assert(not pcall(bearcad.set_gizmo, { name = "move_z", value = 10 }),
  "the skipped axis must refuse a scripted drag too (#1601)")

-- Only Free and the 2D Point Snap are modes for an image (#1601).
assert(not pcall(bearcad.ui.tool_mode, "face_snap"),
  "Face Snap has no faces to mate on an image (#1601)")
bearcad.ui.tool_mode("point_snap")
bearcad.ui.wait(3)
assert(bearcad.ui.tool_mode() == "point_snap", "2D Point Snap stays on offer (#1601)")
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)

-- #1611: dragging the Z rotation handle previews the turn on the quad itself.
local at_rest = bearcad.get{ kind = "image", index = 0 }
local rest = bearcad.image_corners(0)
local rzh = gizmo("move_rz").position
assert(rzh, "the Z turn handle needs a position to drag")
bearcad.ui.drag_ground(rzh.x, rzh.y, rzh.y, -rzh.x)
bearcad.ui.wait(8)

local turned = gizmo("move_rz").value
assert(math.abs(turned) > 0.2,
  "dragging the Z handle should turn the move, got " .. tostring(turned))
local preview = bearcad.image_corners(0)
local moved = 0
for i = 1, 4 do
  local dx, dy = preview[i][1] - rest[i][1], preview[i][2] - rest[i][2]
  if math.sqrt(dx * dx + dy * dy) > 1.0 then moved = moved + 1 end
  assert(math.abs(preview[i][3]) < 0.2,
    "a planar turn must keep every corner on the host plane (#1601), corner " .. i)
end
assert(moved == 4, "every corner should follow the live turn (#1611), moved " .. moved)
assert(math.abs(bearcad.get{ kind = "image", index = 0 }.origin_x - at_rest.origin_x) < 1e-3
   and math.abs(bearcad.get{ kind = "image", index = 0 }.rotation - at_rest.rotation) < 1e-3,
  "the preview must not have committed yet (#1611)")

-- The turn gizmo stays held after the drag (#1440); a second click releases it so
-- Enter reaches the tool and commits.
bearcad.ui.click_ground(rzh.y, -rzh.x)
bearcad.ui.wait(8)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)
local after = bearcad.get{ kind = "image", index = 0 }
-- Both are degrees (#1657): the gizmo's value and the image's stored rotation.
assert(math.abs(after.rotation - turned) < 1.0,
  string.format("the committed image should carry the %.3f deg turn, got %.3f deg",
    turned, after.rotation))
local committed = bearcad.image_corners(0)
for i = 1, 4 do
  assert(math.abs(committed[i][1] - preview[i][1]) < 0.5
     and math.abs(committed[i][2] - preview[i][2]) < 0.5,
    "commit should land where the preview stood (#1611), corner " .. i)
end

print("ok: an image move stays in its plane and previews as it goes")
bearcad.quit()
