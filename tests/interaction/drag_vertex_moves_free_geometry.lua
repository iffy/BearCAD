-- Interaction regression (#459 family): click-to-select then press-drag must move a
-- free vertex, through the REAL pointer path (raw-input synthetic events).
bearcad.new()
bearcad.line{ x = -30, y = -20, x1 = 30, y1 = 20 }
bearcad.ui.tool("select")
-- Hide the side panes: under CI's WM-less Xvfb the window can't maximize, and with
-- all three panes open the 3D viewport is too narrow for the ground-coordinate
-- clicks below to land inside it.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.click_ground(30, 20)      -- first click selects (#239)
bearcad.ui.wait(5)
bearcad.ui.drag_ground(30, 20, 45, 35)
bearcad.ui.wait(10)
local _, _, x1, y1 = bearcad.line_endpoints(0)
assert(math.abs(x1 - 45) < 3 and math.abs(y1 - 35) < 3,
  string.format("endpoint should follow the drag, got (%.1f, %.1f)", x1, y1))
print("ok: free vertex drags")
bearcad.quit()
