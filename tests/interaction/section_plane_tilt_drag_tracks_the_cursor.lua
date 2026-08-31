-- #1766: dragging the green tilt ring (tilt_v) with an offset off the face used to flip
-- ~180 degrees back and forth: the drag re-measured the cursor against a ring centre that
-- itself swings with the tilt, so the value fed back into its own reference. The ring must
-- track the cursor continuously — around the circle, and while the cursor dives deep
-- inside the ring mid-drag.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")

-- Pick the +X side wall face-on, so the tilt rings are two in-plane axes of that wall.
bearcad.ui.view("right")
bearcad.ui.wait(8)
bearcad.cross_section{ name = "Half" }
bearcad.ui.wait(5)
bearcad.ui.tool("section_plane")
bearcad.ui.wait(5)
bearcad.ui.click_world(60, 30, 10)
bearcad.ui.wait(8)

-- Top orthographic: ground coordinates land on the handles (a perspective top view
-- parallaxes a z = 10 ring handle away from its ground point). The green ring is
-- horizontal here (it turns about the wall's vertical in-plane axis), so the sweep stays
-- grabbable.
bearcad.ui.view("top")
bearcad.ui.toggle_projection()
-- Top's own angles (-90 yaw, +90 pitch) spelled out: camera{} lands deterministically.
bearcad.ui.camera{ yaw = -90, pitch = 90, target = {60, 30, 0}, distance = 140 }
bearcad.ui.wait(30)
assert(bearcad.ui.camera().projection == "orthographic",
  "the sweep needs an orthographic top view, got " .. bearcad.ui.camera().projection)

local function gizmo(name)
  return bearcad.gizmo(name)
end

-- Value gizmos are click-to-stick: a click grabs (or releases), and while latched the
-- handle chases plain pointer moves. A release click re-reads the cursor first, so it
-- must land where the pointer already is — anywhere else drags the latched value there.
local px, py = 60, 30
local function click_here()
  bearcad.ui.click_ground(px, py)
  bearcad.ui.wait(5)
end
local function move_to(x, y)
  bearcad.ui.move_ground(x, y)
  bearcad.ui.wait(8)
  px, py = x, y
end

-- A real offset, chased onto the offset arrow: the ring centre swings with the tilt only
-- when the plane sits off the face — the configuration that used to blow up.
move_to(60, 30)
click_here()              -- grab the offset arrow at the anchor
move_to(85, 30)           -- chase it 25 mm off the face
click_here()              -- release

local offset = gizmo("offset")
assert(offset and math.abs(offset.value - 25) < 2,
  string.format("chased offset landed, got %s", tostring(offset and offset.value)))

-- Park the pointer clear of every handle: the offset tip sits at the ring centre here,
-- and the next release click must not grab it.
move_to(120, 10)

local function tilt_v() return gizmo("tilt_v") end
assert(tilt_v(), "a face anchor offers the green tilt_v ring")

-- The tilt_u handle rides the ring centre (it points up out of a horizontal green ring),
-- so from the top it reports the centre; tilt_v's handle gives the ring's radial at the
-- current tilt — rotate it back by the value to recover the ring's 0 degree direction.
local function frame()
  local tu, tv = gizmo("tilt_u"), tilt_v()
  assert(tu.position and tv.position, "tilt gizmos expose their handle positions")
  local cx, cy = tu.position.x, tu.position.y
  local deg = math.rad(tv.value)
  local dx, dy = tv.position.x - cx, tv.position.y - cy
  local c, s = math.cos(-deg), math.sin(-deg)
  return cx, cy, dx * c - dy * s, dx * s + dy * c
end

local function handle_at(cx, cy, ux, uy, deg)
  local r = math.rad(deg)
  local c, s = math.cos(r), math.sin(r)
  return cx + ux * c - uy * s, cy + ux * s + uy * c
end

-- Shortest signed difference b - a, wrapped to (-180, 180] like every rotation gizmo
-- value (#1432): a 200 degree pose may read as -160.
local function angle_diff(a, b)
  return (b - a + 180) % 360 - 180
end

local function step_by(delta)
  click_here()            -- release the previous latch where it sits
  local v = tilt_v().value
  local cx, cy, ux, uy = frame()
  local hx, hy = handle_at(cx, cy, ux, uy, v)
  move_to(hx, hy)
  click_here()            -- grab the green handle
  local tx, ty = handle_at(cx, cy, ux, uy, v + delta)
  move_to(tx, ty)
  local after = tilt_v().value
  assert(math.abs(angle_diff(v, after) - delta) < 12,
    string.format("a %.0f degree move must turn about %.0f degrees, got %.1f -> %.1f",
      delta, delta, v, after))
  return after
end

-- Sweep the green ring half a turn plus, 25 degrees at a time.
local v = tilt_v().value
assert(math.abs(v) < 1e-2, "a fresh face anchor starts untilted, got " .. tostring(v))
for _ = 1, 6 do
  v = step_by(25)
end
assert(v > 120, "the sweep climbed past 120 degrees, got " .. tostring(v))

-- The freak-out: with the ring latched, dive the cursor deep inside it and sit there.
-- The value must follow the cursor angle smoothly, not flip ~180 back and forth.
local v_before = tilt_v().value
do
  click_here()            -- release the sweep latch
  local cx, cy, ux, uy = frame()
  local hx, hy = handle_at(cx, cy, ux, uy, v_before)
  move_to(hx, hy)
  click_here()            -- grab the green handle
  move_to(cx + (hx - cx) * 0.2, cy + (hy - cy) * 0.2)
end
local v_after = tilt_v().value
assert(math.abs(angle_diff(v_before, v_after)) < 45,
  string.format("diving inside the ring must not flip it: %.1f -> %.1f", v_before, v_after))

-- And the drag still works afterwards: two more steps out, no instability.
step_by(25)
step_by(25)

print("ok: the green tilt ring tracks the cursor continuously, offset or not")
bearcad.quit()
