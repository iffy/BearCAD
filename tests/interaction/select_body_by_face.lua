-- #902: with the Select tool, clicking a body's flat face selects the **whole body** —
-- bodies outrank faces — while its edges still outrank the body they belong to.
bearcad.new()
bearcad.rect{ width = 50, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {25, 15, 0}, distance = 220 }
bearcad.ui.wait(5)

-- The middle of the top cap: the whole body.
bearcad.ui.click_ground(40, 15)
bearcad.ui.wait(5)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body",
  "clicking a face should select the body, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

-- An edge still outranks the body it belongs to.
bearcad.clear_selection()
bearcad.ui.click_ground(50, 15)
bearcad.ui.wait(5)
sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body_edge",
  "clicking an edge should select the edge, got " ..
  (#sel > 0 and sel[1].kind or "nothing"))

print("ok: the select tool picks a body through its face, and its edges first")
bearcad.quit()
