-- #1976: the Line tool must start a line at the centre of a circle that sits on the
-- origin, under its diameter label (sketched on a datum plane, viewed face-on).
bearcad.new()
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.circle{ x = 0, y = 0, r = 8.5 }
bearcad.dimension{ kind = "circle", index = 0, value = "17" }
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
bearcad.ui.move_sketch(0, 0)
bearcad.ui.wait(4)
bearcad.ui.click_sketch(0, 0)
bearcad.ui.wait(5)
bearcad.ui.move_sketch(20, 10)
bearcad.ui.wait(4)
bearcad.ui.click_sketch(20, 10)
bearcad.ui.wait(5)
assert(bearcad.count("line") == 1, "expected 1 line, got " .. bearcad.count("line"))
local l = bearcad.get{ kind = "line", index = 0 }
assert(math.abs(l.x0) < 1e-3 and math.abs(l.y0) < 1e-3,
  string.format("line should start at the centre, got (%.3f, %.3f)", l.x0, l.y0))
