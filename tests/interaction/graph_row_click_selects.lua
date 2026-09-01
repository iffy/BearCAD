-- #1670: the Elements pane's Graph view is one node per line, and a click anywhere on a row
-- selects that element — the whole row is the target, not just the dot at its lane.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 40, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local body = row_named("body")
assert(body, "the graph shows the extruded body")
assert(body.x, "the pane reports where it drew the row")

-- Click well past the icon at the row's lane: the whole row is the target.
bearcad.ui.click({ x = body.x + body.w * 0.6, y = body.y + body.h / 2 })
local selection = bearcad.selection()
assert(#selection == 1 and selection[1].kind == "body",
  "clicking a graph row selects its element, got " .. #selection .. " item(s)")

-- Rows are ordered by the graph: every input sits above what it feeds.
local g = bearcad.ui.elements_graph()
local seen = {}
for i, row in ipairs(g.rows) do seen[row.kind] = seen[row.kind] or i end
assert(seen.sketch < seen.extrusion and seen.extrusion < seen.body,
  "sketch above extrusion above body")
for _, e in ipairs(g.edges) do
  assert(e.from < e.to, "graph lines run downward")
end

print("ok: a click anywhere on a graph row selects that element")
bearcad.quit()
