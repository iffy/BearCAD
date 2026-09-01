-- #1927: an Elements graph row drags onto a component row and files there, the same
-- way List-view rows do (#423). Switching to the graph used to lose that path.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local c = bearcad.component{ name = "Frame" }
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.elements_view("graph")
bearcad.ui.wait(8)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local body = row_named("body")
local comp = row_named("component")
assert(body and body.x, "the graph shows the body row and where it drew it")
assert(comp and comp.x, "the graph shows the component row and where it drew it")

-- The cuboid was made before the component, so it still lives at the document root:
-- hiding the (empty) component must not hide the body yet.
bearcad.set_visible({ kind = "component", index = 0 }, false)
assert(bearcad.visible({ kind = "body", index = 0 }),
  "an unfiled body stays visible when the empty component is hidden")
bearcad.set_visible({ kind = "component", index = 0 }, true)

bearcad.ui.drag(
  { x = body.x + body.w * 0.6, y = body.y + body.h / 2 },
  { x = comp.x + comp.w * 0.6, y = comp.y + comp.h / 2 }
)
bearcad.ui.wait(4)

-- Filed: hiding the component now hides the body too (#423).
bearcad.set_visible({ kind = "component", index = 0 }, false)
assert(not bearcad.visible({ kind = "body", index = 0 }),
  "dragging the body row onto the component should file it there")

-- The graph regroups: the component sits above the body, with a parent line between them.
bearcad.set_visible({ kind = "component", index = 0 }, true)
bearcad.ui.wait(2)
local g = bearcad.ui.elements_graph()
local body_i, comp_i
for i, row in ipairs(g.rows) do
  if row.kind == "body" then body_i = i end
  if row.kind == "component" then comp_i = i end
end
assert(comp_i and body_i and comp_i < body_i,
  "the component sits above the body it now holds")
local parented = false
for _, e in ipairs(g.edges) do
  if e.from == comp_i and e.to == body_i and (e.kind == "parent" or e.kind == "dependency") then
    parented = true
  end
end
assert(parented, "a parent line runs from the component to the filed body")

print("ok: a Graph-view body row drags into a component")
bearcad.quit()
