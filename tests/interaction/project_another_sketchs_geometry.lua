-- #1979: with a sketch open, the Project tool takes another sketch's line (clicked in the
-- viewport) and Enter projects it into the open sketch.
bearcad.new()
bearcad.begin_sketch{ kind = "plane", index = 1 }   -- XZ
bearcad.line{ x = 10, y = 0, x1 = 10, y1 = 30 }     -- runs along world Z at x = 10
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "plane", index = 2 }   -- YZ
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("right")
bearcad.ui.camera{ target = {0, 0, 15}, distance = 120 }
bearcad.ui.wait(5)
bearcad.ui.tool("project")
bearcad.ui.wait(3)
-- Seen along X the source line is a vertical stroke at y = 0 above the origin.
bearcad.ui.move_world(10, 0, 15)
bearcad.ui.wait(4)
bearcad.ui.click_world(10, 0, 15)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "line", "the other sketch's line should be selected, got " .. #sel)
bearcad.ui.key("enter")
bearcad.ui.wait(5)
assert(bearcad.count("line") == 2, "a projected line should exist, have " .. bearcad.count("line"))
local l = bearcad.get{ kind = "line", index = 1 }
assert(math.abs(l.length - 30) < 1e-2, "projected length 30, got " .. l.length)
