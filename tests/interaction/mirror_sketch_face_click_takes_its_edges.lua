-- #1539: hovering a closed sketch face during Mirror shape pick lights its four
-- edges (what a click should take). The click used to ignore the face interior
-- and only accept a line/circle under the cursor, so a hover that promised the
-- whole outline did nothing.
bearcad.new()
bearcad.rect{ x = -20, y = -15, width = 40, height = 30 }
-- Mirror axis well away from the rectangle so a click inside the face cannot
-- land on it.
bearcad.line{ x = 50, y = -40, x1 = 50, y1 = 40 }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 200 }
bearcad.ui.wait(8)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("mirror")
bearcad.ui.wait(5)
assert(picker("Mirror line") and picker("Mirror line").focused,
  "the mirror line is the first pick")

-- Feed the axis through the pane so this test is only about the face click.
bearcad.select{ kind = "line", index = 4 }
bearcad.ui.wait(5)
assert(#picker("Mirror line").items == 1, "the axis should be the mirror line")
assert(picker("Shapes") and picker("Shapes").focused,
  "with an axis set, Shapes takes the next click")
assert(#picker("Shapes").items == 0, "Shapes starts empty")

-- Click well inside the rectangle, away from every edge.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(8)
local shapes = picker("Shapes")
assert(shapes and #shapes.items == 4,
  "a click inside the face should take its four edges, got " ..
  #(shapes and shapes.items or {}))
for _, item in ipairs(shapes.items) do
  assert(item.kind == "line", "each shape should be a line, got " .. item.kind)
end

print("ok: clicking a hover-highlighted sketch face picks its edges as mirror shapes")
bearcad.quit()
