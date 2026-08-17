-- #1436: on a sketch atop an extruded circle, Dimension can measure how far a
-- little circle sits from the cap's centre (the sketch origin).
bearcad.new()
bearcad.circle{ x = 0, y = 0, r = 20 }
bearcad.extrude{ circle = 0, distance = 10 }
bearcad.begin_sketch{
  kind = "extrude_cap",
  extrusion = 0,
  profile = "circle",
  profile_index = 0,
  top = true,
}
bearcad.circle{ x = 8, y = 0, r = 3 }

-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- The cap sits at z = 10; its origin is the extruded circle's centre.
bearcad.ui.camera{ target = {0, 0, 10}, distance = 220 }
bearcad.ui.wait(5)
bearcad.ui.tool("dimension")
bearcad.ui.wait(3)

-- Click the cap centre (the origin), then Shift+click the little circle's centre.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
bearcad.ui.click_ground(8, 0, { shift = true })
bearcad.ui.wait(8)
local sel = bearcad.selection()
assert(#sel == 2, "origin + circle centre should both be selected, got " .. #sel)
assert(bearcad.status():find("place the dimension"),
  "expected a placeable origin-to-circle distance, got: " .. bearcad.status())

print("ok: a circle on a circular cap dimensions from the centre")
bearcad.quit()
