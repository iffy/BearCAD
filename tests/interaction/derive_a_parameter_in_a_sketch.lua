-- #1730: the Dimension tool's "Derive parameter" block only showed in 3D, so a length
-- measured off sketch geometry could not be turned into a parameter without leaving the
-- sketch. The block follows the tool.
bearcad.new()
-- A plain line: a dimensioned rectangle side already has its length constrained.
bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

assert(bearcad.count("sketch") == 1, "the sketch is open")
bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

-- Pick the rectangle's bottom edge, then measure it.
bearcad.select{ kind = "line", index = 0 }
bearcad.ui.wait(4)
local before = bearcad.count("parameter")
bearcad.derive_parameter{ from = "selection", name = "width" }
bearcad.ui.wait(6)
assert(bearcad.count("parameter") == before + 1,
  "deriving from a sketch line should add a parameter, got " .. bearcad.count("parameter"))
local p = bearcad.get{ kind = "parameter", index = before }
assert(p.name == "width", "named as asked, got " .. tostring(p.name))
assert(p.expression:find("40"),
  "and it measures the line it was derived from, got " .. tostring(p.expression))

print("ok: a derived parameter can be made from inside a sketch")
bearcad.quit()
