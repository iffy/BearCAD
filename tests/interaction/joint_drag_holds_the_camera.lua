-- #905: dragging a part through its joint never moves the camera — auto-zoom stands down
-- for the drag instead of chasing the part around the viewport.
bearcad.new()
bearcad.rect{ width = 50, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.rect{ x = 50, y = 0, width = 10, height = 10 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
bearcad.exit_sketch()
bearcad.joint{
  a = 0, b = 1, kind = "slider",
  from   = { body = 1, vertex = {50, 0, 0} },
  to     = { body = 0, vertex = {50, 0, 0} },
  from_b = { body = 1, vertex = {50, 10, 0} },
  to_b   = { body = 0, vertex = {50, 10, 0} },
  slide_max = 200,
}
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {30, 5, 0}, distance = 220 }
-- Auto-zoom on: it's what would otherwise chase the moving part.
bearcad.ui.auto_zoom(true)
bearcad.ui.wait(5)

local before = bearcad.ui.camera{}
-- A long pull: the slab ends up well outside the framed view.
bearcad.ui.drag_ground(55, 5, 55, 120)
bearcad.ui.wait(10)
local after = bearcad.ui.camera{}
local function same(a, b, what)
  assert(math.abs(a - b) < 0.01,
    what .. " should not move while dragging a joint: " .. a .. " -> " .. b)
end
same(before.distance, after.distance, "the camera distance")
same(before.yaw, after.yaw, "the camera yaw")
same(before.pitch, after.pitch, "the camera pitch")
for i = 1, 3 do same(before.target[i], after.target[i], "the camera target") end

print("ok: a joint drag leaves the camera alone")
bearcad.quit()
