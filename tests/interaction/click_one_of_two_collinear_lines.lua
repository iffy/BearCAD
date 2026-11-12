-- #1978: two collinear lines meeting at the origin; clicking one selects only that one.
bearcad.new()
bearcad.begin_sketch{ kind = "plane", index = 1 }
bearcad.line{ x = 0, y = 0, x1 = 10.5, y1 = 0 }
bearcad.line{ x = 0, y = 0, x1 = -10.5, y1 = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(5)
bearcad.ui.view("front")
bearcad.ui.camera{ target = {0, 0, 0}, distance = 60 }
bearcad.ui.wait(5)
bearcad.ui.tool("select")
bearcad.ui.wait(3)
for _, u in ipairs({ 6, -6 }) do
  bearcad.clear_selection()
  bearcad.ui.wait(3)
  bearcad.ui.move_sketch(u, 0)
  bearcad.ui.wait(4)
  bearcad.ui.click_sketch(u, 0)
  bearcad.ui.wait(5)
  local sel = bearcad.selection()
  assert(#sel == 1, "clicking at u=" .. u .. " selected " .. #sel .. " things")
  assert(sel[1].kind == "line" and sel[1].index == (u > 0 and 0 or 1), "wrong line at u=" .. u)
end
