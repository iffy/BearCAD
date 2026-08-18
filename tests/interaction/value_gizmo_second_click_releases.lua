-- #1497: every value gizmo is click-to-stick. The second click *releases* the handle;
-- Enter / ✓ commits. Placement tools (Line/Rect/Circle/Shape) stay click-to-finish.
--
-- Grab each handle and assert the second click does not commit the draft.

local function hide_panes()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")
end

-- Extrude: already the #584 rule. Second click on the handle must not create the solid.
bearcad.new()
bearcad.rect{ width = 80, height = 40 }
bearcad.exit_sketch()
hide_panes()
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {40, 20, 0}, distance = 260 }
bearcad.ui.wait(5)
bearcad.ui.tool("extrude")
bearcad.ui.wait(3)
bearcad.ui.click_ground(40, 20)
bearcad.ui.wait(6)
-- Grab the handle (same screen point as the face centre from the top).
bearcad.ui.click_ground(40, 20)
bearcad.ui.wait(4)
-- Second click releases; the extrusion is still a draft.
bearcad.ui.click_ground(40, 20)
bearcad.ui.wait(4)
assert(bearcad.count("extrusion") == 0,
  "extrude second click must not commit, got " .. bearcad.count("extrusion"))
bearcad.ui.key("enter")
bearcad.ui.wait(8)
assert(bearcad.count("extrusion") == 1, "Enter should commit the extrude")

-- 2D Chamfer: used to commit on the second click (the old Extrude path).
bearcad.new()
bearcad.rect{ width = 40, height = 40 }
hide_panes()
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 20, 0}, distance = 160 }
bearcad.ui.wait(5)
bearcad.ui.tool("chamfer")
bearcad.ui.wait(3)
-- Origin corner: the two lines run +X and +Y, so the handle sits 4 mm along the
-- inward bisector at (2.83, 2.83) (gizmo_display_offset of the 2 mm default).
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
local lines_before = bearcad.count("line")
bearcad.ui.click_ground(2.83, 2.83)
bearcad.ui.wait(4)
bearcad.ui.click_ground(2.83, 2.83)
bearcad.ui.wait(4)
assert(bearcad.count("line") == lines_before,
  "2D chamfer second click must not commit, lines " .. bearcad.count("line"))
bearcad.ui.key("enter")
bearcad.ui.wait(8)
assert(bearcad.count("line") > lines_before,
  "Enter should commit the 2D chamfer")

-- 3D Chamfer: same old-Extrude second-click commit on a body edge.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
hide_panes()
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 15, 7.5}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("chamfer")
bearcad.ui.wait(3)
-- Front edge of the top cap (y = 0). Inward bisector is +Y; handle at ~4 mm in.
bearcad.ui.click_ground(20, 0)
bearcad.ui.wait(6)
local gizmos = bearcad.gizmos()
local had = false
for _, g in ipairs(gizmos) do
  if g.name == "chamfer" then had = true break end
end
assert(had, "3D chamfer should expose a handle after an edge pick")
bearcad.ui.click_ground(20, 4)
bearcad.ui.wait(4)
bearcad.ui.click_ground(20, 4)
bearcad.ui.wait(4)
had = false
for _, g in ipairs(bearcad.gizmos()) do
  if g.name == "chamfer" then had = true break end
end
assert(had, "3D chamfer second click must release the handle, not commit")
bearcad.ui.key("enter")
bearcad.ui.wait(8)
had = false
for _, g in ipairs(bearcad.gizmos()) do
  if g.name == "chamfer" then had = true break end
end
assert(not had, "Enter should commit the 3D chamfer")

-- Sketch Offset: click-to-stick (used to be hold-to-drag). Click the handle,
-- move without holding, second click must not commit.
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
-- Default 5 mm handle at (0, 5). Click to grab, then move without a button.
bearcad.ui.click_ground(0, 5)
bearcad.ui.wait(4)
bearcad.ui.move_ground(0, 14)
bearcad.ui.wait(6)
-- Second click releases; the copy is still a draft.
bearcad.ui.click_ground(0, 14)
bearcad.ui.wait(4)
assert(bearcad.count("line") == 1,
  "offset second click must not commit, got " .. bearcad.count("line"))
bearcad.ui.key("Enter")
bearcad.ui.wait(10)
assert(bearcad.count("line") == 2, "Enter should commit the offset")
local _, y0, _, y1 = bearcad.line_endpoints(1)
assert(math.abs(y0 - 14) < 0.6 and math.abs(y1 - 14) < 0.6,
  string.format("click-to-stick should follow to v=14, got y=(%.2f, %.2f)", y0, y1))

-- Shell: second click releases, same as Extrude.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 40, height = 30 }
hide_panes()
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 15}, distance = 200 }
bearcad.ui.wait(5)
bearcad.ui.tool("shell")
bearcad.ui.wait(3)
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
local found = false
for _, g in ipairs(bearcad.gizmos()) do
  if g.name == "shell" then found = true break end
end
assert(found, "shell thickness gizmo should appear after a body pick")
-- Default 1 mm wall, handle sits at the display offset along the inward normal.
-- From the top, the first face handle is near the body centre.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(4)
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(4)
assert(bearcad.count("body") == 1,
  "shell second click must not commit, got " .. bearcad.count("body"))

print("ok: value-gizmo second click releases; Enter commits")
bearcad.quit()
