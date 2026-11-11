-- #1965: a sketch's Elements row drags onto a component row and files the sketch there —
-- even one on a datum plane, which used to have no way in.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 20, height = 10 }
bearcad.exit_sketch()
local c = bearcad.component{ name = "Part" }
bearcad.clear_selection()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local sketch = bearcad.ui.elements_row_rect("Sketch 0")
local comp = bearcad.ui.elements_row_rect("Part")
assert(sketch, "the sketch has an Elements row")
assert(comp, "the component has an Elements row")

bearcad.ui.drag(sketch, comp)
bearcad.ui.wait(4)

-- Filed: hiding the component hides the sketch.
bearcad.set_visible(c, false)
assert(not bearcad.visible({ kind = "sketch", index = 0 }),
  "dragging the sketch row onto the component should file it there")

print("ok: a sketch row drags into a component")
bearcad.quit()
