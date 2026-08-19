-- #1575: a long Elements-pane label must clip, not stretch the pane.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.pane("elements", "show")

bearcad.line{
  x = 0, y = 0, x1 = 510, y1 = 0,
  name = "this is an extremely long element name that must not stretch the elements pane",
}

bearcad.ui.wait(8)
local r = bearcad.ui.pane_rect("elements")
assert(r, "elements pane should be visible")
assert(r.w <= 240,
  "long names must clip instead of widening the Elements pane, got w=" .. tostring(r.w))

print("ok: long element names clip inside the Elements pane")
bearcad.quit()
