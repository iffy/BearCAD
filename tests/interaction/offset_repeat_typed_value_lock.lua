-- #1502: Sketch Offset and Repeat gizmos must not overwrite a typed value.
-- Grab the handle (selects the field), type, then nudge the pointer while still
-- following — the field / committed value stays what was typed.

local function hide_panes()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")
end

-- Offset: type 12 after grabbing, then move the handle out to v=20. Commit at 12.
bearcad.new()
bearcad.line{ x = -10, y = 0, x1 = 10, y1 = 0 }
hide_panes()
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 120 }
bearcad.ui.wait(8)
bearcad.ui.tool("offset")
bearcad.ui.wait(3)
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
-- Grab at the default 5 mm handle.
bearcad.ui.click_ground(0, 5)
bearcad.ui.wait(4)
bearcad.ui.type("12")
bearcad.ui.wait(4)
-- Still following: move the pointer without a new grab.
bearcad.ui.move_ground(0, 20)
bearcad.ui.wait(6)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)
assert(bearcad.count("line") == 2,
  "expected the source line plus its offset copy, got " .. bearcad.count("line"))
local _, y0, _, y1 = bearcad.line_endpoints(1)
assert(math.abs(y0 - 12) < 0.6 and math.abs(y1 - 12) < 0.6,
  string.format("typed 12 mm must stick after a handle nudge, got y=(%.2f, %.2f)", y0, y1))

-- Repeat: type Distance after grabbing, then nudge. Commit uses the typed span.
-- The Distance field lives in the Context pane (no floating input), so that pane
-- stays up; grabbing the handle focuses it.
bearcad.new()
bearcad.rect{ width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {40, 10, 0}, distance = 320 }
bearcad.ui.wait(5)
bearcad.ui.tool("repeat")
bearcad.ui.wait(4)
-- Body, then the +X edge as the path (y = 0).
bearcad.ui.click_ground(10, 10)
bearcad.ui.wait(5)
bearcad.ui.click_ground(10, 0)
bearcad.ui.wait(5)
-- CountGap 3×10 mm on a 20 mm body, distance-to-end: last copy ends at 80 mm
-- from the start plane (x = 0). Handle at (80, 10).
local function repeat_gizmo()
  for _, g in ipairs(bearcad.ui.gizmos()) do
    if g.name == "repeat" then return g end
  end
end
local g = repeat_gizmo()
assert(g and g.position, "repeat distance handle should be up after a path pick")
-- Grab at the handle, type a Distance, then nudge along the axis without a new grab.
bearcad.ui.click_ground(g.position.x, g.position.y)
bearcad.ui.wait(4)
bearcad.ui.type("50")
bearcad.ui.wait(4)
bearcad.ui.move_ground(g.position.x + 40, g.position.y)
bearcad.ui.wait(6)
bearcad.ui.key("enter")
bearcad.ui.wait(10)
assert(bearcad.count("body") >= 2, "repeat should produce copies")
-- Typed 50 mm to the end of the last item. A 120 mm nudge would have stretched it.
local stats = bearcad.body_stats(bearcad.count("body") - 1)
local xmax = stats.bbox.max.x
assert(math.abs(xmax - 50) < 1.5,
  string.format("typed repeat Distance 50 must stick, last body xmax=%.2f", xmax))

print("ok: Offset and Repeat keep a typed value when the handle moves")
bearcad.quit()
