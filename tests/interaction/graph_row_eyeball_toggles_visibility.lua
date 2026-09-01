-- #1907: the Elements pane's Graph view has a leftmost column of visibility
-- eyeballs, to the left of the lanes, that hide/show an element without selecting it.
bearcad.new()
bearcad.ui.tool("select")
local box = bearcad.cuboid{ width = 10, depth = 10, height = 10 }
assert(bearcad.visible(box), "a new cuboid is visible")
bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)

local function row_named(kind)
  for _, row in ipairs(bearcad.ui.elements_graph().rows) do
    if row.kind == kind then return row end
  end
  return nil
end

local body = row_named("body")
assert(body, "the graph shows the cuboid body")
assert(body.x, "the pane reports where it drew the row")
assert(body.eye, "a hideable graph row has an eyeball")
assert(body.eye.x and body.eye.y and body.eye.w and body.eye.h,
  "the eyeball is a clickable window-space rect")

-- The eyeball is always the left-most column, not following the node's lane.
local g = bearcad.ui.elements_graph()
local eye_x
for _, row in ipairs(g.rows) do
  if row.eye then
    if not eye_x then
      eye_x = row.eye.x
    else
      assert(math.abs(row.eye.x - eye_x) < 1,
        "every eyeball shares the left-most column, got " .. row.eye.x
          .. " vs " .. eye_x .. " for " .. row.kind)
    end
    assert(row.eye.x <= row.x + 1, "the eyeball sits at the left of its row")
    assert(row.eye.x + row.eye.w <= row.x + row.w,
      "the eyeball stays inside the row")
  end
end
assert(eye_x, "the graph painted at least one eyeball")

local shape = row_named("shape")
if shape and shape.eye and body.lane ~= shape.lane then
  assert(math.abs(shape.eye.x - body.eye.x) < 1,
    "eyeballs stay in one column even when lanes differ")
end

bearcad.clear_selection()
bearcad.ui.click(body.eye)
assert(not bearcad.visible({ kind = "body", index = 0 }),
  "clicking the eyeball hides the body")
assert(#bearcad.selection() == 0,
  "clicking the eyeball must not select the row, got " .. #bearcad.selection() .. " item(s)")

bearcad.ui.click(body.eye)
assert(bearcad.visible({ kind = "body", index = 0 }),
  "clicking the eyeball again shows the body")
assert(#bearcad.selection() == 0,
  "a second eyeball click still must not select")

-- The rest of the row still selects, the way it did before the eyeball column.
bearcad.ui.click({ x = body.x + body.w * 0.6, y = body.y + body.h / 2 })
local selection = bearcad.selection()
assert(#selection == 1 and selection[1].kind == "body",
  "clicking the graph (not the eyeball) still selects the row")

-- Display-only rows (a drawing page) have no eyeball, but the column is still
-- reserved so hideable rows keep theirs on the far left.
bearcad.drawing{}
bearcad.ui.elements_view("graph")
bearcad.ui.wait(5)
local drawing = row_named("drawing")
assert(drawing, "the graph shows the drawing")
assert(not drawing.eye, "a drawing row has no visibility toggle")
body = row_named("body")
assert(body and body.eye, "the body still has an eyeball after adding a drawing")
assert(body.eye.x <= body.x + 1, "the eyeball is still the left-most column")

print("ok: graph-view eyeballs are a left column and toggle visibility")
bearcad.quit()
