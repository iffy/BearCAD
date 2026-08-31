-- #1857: the Constraint tool's Tangent row makes two circles hug at their rims, and makes a
-- circle graze a line. Driven through real clicks so the pane row, its digit shortcut, and
-- the solve are all exercised.
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 20 }
bearcad.circle{ x = 70, y = 0, r = 10 }
bearcad.line{ x = -60, y = 50, x1 = 60, y1 = 50 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 15, 0}, distance = 300 }
bearcad.ui.wait(5)
bearcad.ui.tool("constraint")
bearcad.ui.wait(3)

-- Click each rim to select the whole circle, then apply Tangent (digit 8). Pick spots off
-- the sketch's origin axes, which are pickable reference lines in their own right.
bearcad.ui.click_ground(14.142, -14.142)
bearcad.ui.wait(6)
bearcad.ui.click_ground(70, -10, { shift = true })
bearcad.ui.wait(6)
local sel = bearcad.selection()
assert(#sel == 2 and sel[1].kind == "circle" and sel[2].kind == "circle",
  "both circles should be selected, got " .. #sel .. " (" .. sel[1].kind .. ")")
bearcad.ui.key("8")
bearcad.ui.wait(10)

local a = bearcad.get{ kind = "circle", index = 0 }
local b = bearcad.get{ kind = "circle", index = 1 }
local gap = math.sqrt((a.x - b.x) ^ 2 + (a.y - b.y) ^ 2)
assert(math.abs(gap - (a.r + b.r)) < 0.05,
  string.format("rims should touch: centres %.3f apart, radii %.1f + %.1f", gap, a.r, b.r))

-- Now the second circle against the line above them.
bearcad.clear_selection()
bearcad.ui.wait(3)
bearcad.constrain("tangent",
  { kind = "circle", index = 1 }, { kind = "line", index = 0 })
bearcad.ui.wait(6)

b = bearcad.get{ kind = "circle", index = 1 }
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
local dx, dy = x1 - x0, y1 - y0
local len = math.sqrt(dx * dx + dy * dy)
local dist = math.abs((b.x - x0) * dy - (b.y - y0) * dx) / len
assert(math.abs(dist - b.r) < 0.05,
  string.format("the line should graze the rim: %.3f from the centre, r = %.1f", dist, b.r))

print("ok: tangent makes circles hug each other and hug a line")
bearcad.quit()
