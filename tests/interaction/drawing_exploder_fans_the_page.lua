-- #1641: the Selection Exploder works on the drawing workbench too. Space over a spot on the
-- page fans out what is there — a view's edges, the card itself, a note — and clicking a loupe
-- does what clicking that thing would have done (here: the Dimension tool toggles its
-- dimension). Coincident projected edges are the crowd it exists to sort out.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
assert(vp.width > 100 and vp.height > 100,
  string.format("expected a real sheet area, got %.0fx%.0f", vp.width, vp.height))
local cx, cy = vp.width / 2, vp.height / 2

bearcad.ui.tool("select")
bearcad.ui.wait(4)
bearcad.ui.move(cx, cy)
bearcad.ui.wait(4)
assert(#bearcad.ui.exploder() == 0, "no fan before Space")

bearcad.ui.key("space")
bearcad.ui.wait(6)
local leaves = bearcad.ui.exploder()
assert(#leaves > 0, "Space over the page should fan out what is under the cursor")
local kinds = {}
for _, l in ipairs(leaves) do kinds[l.kind] = true end
assert(kinds["projection"],
  "the fan should offer the view card it is over, got " .. #leaves .. " leaves")

bearcad.ui.key("space")
bearcad.ui.wait(4)
assert(#bearcad.ui.exploder() == 0, "Space closes the fan again")

-- Now the Dimension tool: find a spot down the card where an edge fans out, and pick it.
bearcad.ui.tool("dimension")
bearcad.ui.wait(4)
local hit
for dy = -160, 160, 6 do
  bearcad.ui.move(cx, cy + dy)
  bearcad.ui.wait(3)
  bearcad.ui.key("space")
  bearcad.ui.wait(5)
  for _, l in ipairs(bearcad.ui.exploder()) do
    if (l.kind == "projected_edge" or l.kind == "drawing_dimension") and l.x then hit = l end
  end
  if hit then break end
  bearcad.ui.key("space")
  bearcad.ui.wait(2)
end
assert(hit, "expected an edge to fan out somewhere down the card")

bearcad.ui.click(hit.x, hit.y)
bearcad.ui.wait(8)
assert(#bearcad.ui.exploder() == 0, "picking a loupe dismisses the fan")
assert(bearcad.status():find("dimension"),
  "picking an edge loupe with the Dimension tool should toggle its dimension, got: "
    .. bearcad.status())

print("ok: the Selection Exploder fans out a drawing page")
bearcad.quit()
