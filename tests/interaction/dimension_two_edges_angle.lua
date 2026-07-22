-- Interaction regression (#486/#487/#488): Dimension tool accumulates edges;
-- second edge starts angle placement; a place-click opens the value editor and
-- committing creates an angle constraint.
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
bearcad.line{ x = 0, y = 0, x1 = 0, y1 = 30 }
bearcad.clear_selection()
bearcad.ui.tool("dimension")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- First edge: select only.
bearcad.ui.click_ground(20, 0)
bearcad.ui.wait(8)
local sel = bearcad.selection()
local has_line0 = false
for _, e in ipairs(sel) do
  if e.kind == "line" and e.index == 0 then has_line0 = true end
end
assert(has_line0, "first edge should be selected")
assert(#sel == 1, "first edge only — length editor must not have consumed the pick")

-- Second edge without Shift: accumulates and starts angle placement.
bearcad.ui.click_ground(0, 15)
bearcad.ui.wait(8)
sel = bearcad.selection()
local has_line1 = false
for _, e in ipairs(sel) do
  if e.kind == "line" and e.index == 1 then has_line1 = true end
end
assert(has_line0 and has_line1 and #sel == 2, "both edges should be selected for angle")

-- Place the angle (click in a wedge) → value editor opens.
bearcad.ui.click_ground(12, 8)
bearcad.ui.wait(8)
bearcad.ui.type("90deg")
bearcad.ui.wait(2)
bearcad.ui.key("Enter")
bearcad.ui.wait(10)

local found = false
for i = 0, 30 do
  local ok, c = pcall(function()
    return bearcad.get{ kind = "constraint", index = i }
  end)
  if ok and c and tostring(c.kind):find("angle") then
    found = true
    break
  end
end
assert(found, "placing two edges under Dimension should create an angle constraint")
print("ok: dimension tool two-edge angle")
bearcad.quit()
