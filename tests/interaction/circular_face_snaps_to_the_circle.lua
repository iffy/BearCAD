-- #1858: a sketch on a circular cap must treat the rim as a circle. The cap's boundary is
-- stored as 48 chords, so a point dropped near the rim used to latch onto a chord's
-- *midpoint* — 48 stray snap dots around the edge — and got pinned there with a Midpoint
-- constraint. Now the rim snaps as one true circle, with a point-on-circle coincidence.
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 20 }
bearcad.extrude{ circle = 0, distance = 10 }
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "circle",
  profile_index = 0,
  top = true,
}
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- The cap sits at z = 10; its sketch origin is the extruded circle's centre.
bearcad.ui.camera{ target = {0, 0, 10}, distance = 160 }
bearcad.ui.wait(5)
bearcad.ui.tool("line")
bearcad.ui.wait(3)

local before = bearcad.count("constraint")
local function place(x, y)
  bearcad.ui.move_world(x, y, 10)
  bearcad.ui.wait(4)
  bearcad.ui.click_world(x, y, 10)
  bearcad.ui.wait(5)
end

-- Start in open space, then finish a hair outside the rim at 3.75° — dead on a chord's
-- midpoint, the angle that used to produce a Midpoint snap.
local a = math.rad(3.75)
place(5, -12)
place(20.1 * math.cos(a), 20.1 * math.sin(a))
bearcad.ui.key("escape")
bearcad.ui.wait(6)

local added = bearcad.count("constraint") - before
assert(added == 1, "the rim snap should add exactly one constraint, got " .. added)
local c = bearcad.get{ kind = "constraint", index = before }
assert(c.kind == "coincident",
  "the rim should pin with a point-on-circle coincidence, got " .. tostring(c.kind))

-- And the endpoint lands on the circle itself, not inside it on a chord.
local _, _, x1, y1 = bearcad.line_endpoints(0)
local r = math.sqrt(x1 * x1 + y1 * y1)
assert(math.abs(r - 20) < 0.2,
  string.format("the endpoint should sit on the r = 20 rim, got r = %.3f", r))

print("ok: a circular cap's rim snaps as a circle, not as 48 chords")
bearcad.quit()
