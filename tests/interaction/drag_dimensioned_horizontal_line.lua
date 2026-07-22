-- Interaction regression (#485): a horizontal, length-dimensioned free line must
-- still drag via the select tool — vertex or whole-line — with length preserved.
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = "50" }
bearcad.select{ kind = "line", index = 0 }
bearcad.add_geometric_constraint("horizontal")
bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Vertex drag: select endpoint, then press-drag perpendicular to the line.
bearcad.ui.click_ground(50, 0)
bearcad.ui.wait(5)
bearcad.ui.drag_ground(50, 0, 50, 20)
bearcad.ui.wait(10)
local x0, y0, x1, y1 = bearcad.line_endpoints(0)
local len = math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
assert(math.abs(len - 50) < 0.5, string.format("length must stay 50, got %.2f", len))
assert(math.abs(y0 - y1) < 0.5, string.format("must stay horizontal, y0=%.1f y1=%.1f", y0, y1))
assert(math.abs(y1) > 5 or math.abs(y0) > 5,
  string.format("vertex drag must translate the line, got (%.1f,%.1f)-(%.1f,%.1f)", x0, y0, x1, y1))
print("ok: dimensioned horizontal vertex drag")

-- Whole-line drag from the midpoint.
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0, dimension = "50" }
bearcad.select{ kind = "line", index = 0 }
bearcad.add_geometric_constraint("horizontal")
bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)
bearcad.ui.click_ground(25, 0)
bearcad.ui.wait(5)
bearcad.ui.drag_ground(25, 0, 25, 20)
bearcad.ui.wait(10)
x0, y0, x1, y1 = bearcad.line_endpoints(0)
len = math.sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2)
assert(math.abs(len - 50) < 0.5, string.format("length must stay 50, got %.2f", len))
assert(math.abs(y0) > 5 or math.abs(y1) > 5,
  string.format("line drag must translate, got y0=%.1f y1=%.1f", y0, y1))
print("ok: dimensioned horizontal line drag")
bearcad.quit()
