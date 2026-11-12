-- #1977: a line end snaps onto a sketch circle's rim, and to where the rim crosses an
-- origin axis.
bearcad.new()
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.circle{ x = 0, y = 0, r = 8.5 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(5)
bearcad.ui.view("front")
bearcad.ui.camera{ target = {0, 0, 0}, distance = 60 }
bearcad.ui.wait(5)
bearcad.ui.tool("line")
bearcad.ui.wait(3)

local function place(u, v)
  bearcad.ui.move_sketch(u, v)
  bearcad.ui.wait(4)
  bearcad.ui.click_sketch(u, v)
  bearcad.ui.wait(5)
end

-- Rim: end a hair outside the circle at 45°.
place(25, -20)
local a = math.rad(45)
place(8.9 * math.cos(a), 8.9 * math.sin(a))
local l = bearcad.get{ kind = "line", index = 0 }
local r = math.sqrt(l.x1 * l.x1 + l.y1 * l.y1)
assert(math.abs(r - 8.5) < 1e-2, "end should sit on the rim, radius " .. r)

-- Crossing: a fresh line ending near the top of the circle lands exactly on the Y axis.
bearcad.ui.key("escape")
bearcad.ui.wait(3)
place(25, 20)
place(0.3, 8.8)
local m = bearcad.get{ kind = "line", index = 1 }
assert(math.abs(m.x1) < 1e-2 and math.abs(m.y1 - 8.5) < 1e-2,
  string.format("end should be at (0, 8.5), got (%.3f, %.3f)", m.x1, m.y1))
