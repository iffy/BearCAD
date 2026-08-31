-- #1859: a circular repeat's axis is a circle's own normal. With the Path picker armed, the
-- normal shows up under the cursor near the circle's centre — nowhere else, and for no other
-- picker — and one click takes it, turning the pattern around it.
bearcad.new()
-- A guide circle to turn about, and a little block to repeat around it.
bearcad.circle{ x = 0, y = 0, r = 40 }
bearcad.rect{ width = 8, height = 8, x = 36, y = -4 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 6 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 3}, distance = 260 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

-- The Select tool must not offer the normal: an ordinary hover at a circle's centre is
-- still about the centre, not about an axis through it.
bearcad.ui.tool("select")
bearcad.ui.wait(3)
bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(not h or h.kind ~= "circle_normal",
  "Select should not offer a circle normal, got " .. (h and h.kind or "nothing"))

bearcad.ui.tool("repeat")
bearcad.ui.wait(5)
bearcad.ui.click_ground(40, 0)
bearcad.ui.wait(6)
assert(#picker("Bodies").items == 1, "the block should be gathered")
assert(picker("Path").focused, "with a body gathered, the Path picker takes over")

-- Hovering the circle's centre now shows its normal…
bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(6)
h = bearcad.hovered()
assert(h and h.kind == "circle_normal",
  "the Path picker should light the circle's normal, got " .. (h and h.kind or "nothing"))

-- …and clicking it takes it as the path.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
assert(#picker("Path").items == 1,
  "the normal should become the path, got " .. #picker("Path").items)

-- Which is a circular repeat: committing turns the copies about the circle's centre, so
-- every one lands the same distance from it — at its own angle, not strung out in a line.
bearcad.ui.key("enter")
bearcad.ui.wait(12)
local n = bearcad.count("body")
assert(n >= 3, "the pattern should make copies, got " .. n)
local angles = {}
for i = 0, n - 1 do
  local s = bearcad.body_stats(i)
  local cx = (s.bbox.min.x + s.bbox.max.x) / 2
  local cy = (s.bbox.min.y + s.bbox.max.y) / 2
  local r = math.sqrt(cx * cx + cy * cy)
  assert(math.abs(r - 40) < 1.0,
    string.format("copy %d should ride the r = 40 circle, got %.2f", i, r))
  angles[#angles + 1] = math.deg(math.atan(cy, cx))
end
table.sort(angles)
assert(math.abs(angles[#angles] - angles[1]) > 5,
  string.format("the copies should be turned to different angles, got %.1f..%.1f",
    angles[1], angles[#angles]))

print("ok: a circle's normal is the Repeat path, and turns the pattern around it")
bearcad.quit()
