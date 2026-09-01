-- #1819: with a drawing open, a body row drags from the Elements pane onto the page and drops
-- a projection there (#290). Only the List/Tree rows armed that payload — the Graph view's
-- rows dragged nothing, so switching to the graph lost a way of building a drawing.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local d = bearcad.drawing{}
-- Hide the panes the drag doesn't need (CI's WM-less Xvfb can't maximize).
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.elements_view("graph")
bearcad.ui.wait(8)

assert(#bearcad.drawing_views(d) == 0, "the drawing starts empty")

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local body = row_named("body")
assert(body and body.x, "the graph shows the body row and where it drew it")

-- Drop it on the middle of the page, well right of the pane.
local vp = bearcad.ui.viewport()
bearcad.ui.drag(
  { x = body.x + body.w * 0.6, y = body.y + body.h / 2 },
  { x = vp.x + vp.width * 0.6, y = vp.y + vp.height * 0.5 }
)

local views = bearcad.drawing_views(d)
assert(#views == 1,
  "dragging the body row onto the page should place a projection, got " .. #views)

print("ok: a Graph-view body row drags onto the drawing page")
bearcad.quit()
