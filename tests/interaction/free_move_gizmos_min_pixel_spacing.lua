-- #1478: Free Move gizmos on a small object keep a minimum viewport-pixel gap
-- instead of shrinking into a clump.
bearcad.new()
bearcad.rect{ width = 2, height = 2 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 1 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- Orthographic top so in-plane handle pairs project 1:1. Far enough that a
-- 2×2×1 mm box is a few pixels across (the gizmos would otherwise clump).
bearcad.ui.view("top")
bearcad.ui.camera{ target = {1, 1, 0.5}, distance = 800, projection = "orthographic" }
bearcad.ui.wait(5)

bearcad.ui.tool("move")
bearcad.ui.tool_mode("free")
bearcad.ui.wait(5)
bearcad.begin_move{ bodies = {0} }
bearcad.ui.wait(8)

-- In-plane handles only: +Z and the Y-ring sit on the view axis and share a
-- screen point by construction.
local want = { move_x = true, move_y = true, move_rx = true, move_rz = true }
local handles = {}
for _, g in ipairs(bearcad.gizmos()) do
  if want[g.name] then
    assert(g.screen, g.name .. " should expose its screen position")
    table.insert(handles, { name = g.name, x = g.screen.x, y = g.screen.y })
  end
end
assert(#handles == 4, "expected 4 in-plane Free Move handles, got " .. #handles)

local min_px = 48
for i = 1, #handles do
  for j = i + 1, #handles do
    local dx = handles[i].x - handles[j].x
    local dy = handles[i].y - handles[j].y
    local d = math.sqrt(dx * dx + dy * dy)
    assert(d + 0.5 >= min_px,
      string.format(
        "%s and %s are %.1f px apart, want >= %d",
        handles[i].name, handles[j].name, d, min_px))
  end
end

print("ok: free move gizmos keep a minimum viewport-pixel spacing")
bearcad.quit()
