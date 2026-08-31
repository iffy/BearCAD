-- #946: the world origin is a Move end point — clicking it with an end picker armed snaps
-- the body's corner onto (0, 0, 0), no body needing a corner there.
bearcad.new()
bearcad.rect{ x = 40, y = 40, width = 20, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
-- The moving body and its source corner are set; end point A is what the next click is for.
bearcad.ui.begin_move{
  bodies = {0},
  from   = { body = 0, vertex = {40, 40, 0} },
}
bearcad.ui.tool_mode("snap")
bearcad.ui.wait(3)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {25, 25, 0}, distance = 220 }
bearcad.ui.wait(8)

bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(6)
assert(bearcad.status():find("Origin"),
  "clicking the origin should land it as end point A, got: " .. bearcad.status())

bearcad.ui.key("enter")
bearcad.ui.wait(10)
assert(bearcad.count("body") == 2,
  "the move should commit a moved body, got " .. bearcad.count("body") ..
  " (status: " .. bearcad.status() .. ")")

print("ok: the world origin is a Move end point")
bearcad.quit()
