-- #1834: the open drawing's default projection style is set from the Context pane with
-- nothing selected, and a view placed afterwards starts in it. The row steps aside once a
-- view is selected — that view's own Style row is the one that matters then.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local d = bearcad.drawing{}
bearcad.ui.wait(6)

assert(bearcad.ui.context_row_rect("drawing_default_style"),
  "with nothing selected the pane offers the page's default style")

bearcad.drawing_style{ drawing = d, style = "colorful" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.wait(6)
assert(bearcad.drawing_views(d)[1].style == "colorful",
  "a view added after the default takes it, got " .. bearcad.drawing_views(d)[1].style)

-- Placing a view selects it, so the page-wide row gives way to the view's own Style row.
assert(bearcad.ui.context_row_rect("Style"), "the selected view has its own Style row")
assert(not bearcad.ui.context_row_rect("drawing_default_style"),
  "and the page-wide row is out of the way")

-- Overriding one view leaves the page default alone.
bearcad.drawing_view_style{ drawing = d, view = 0, style = "watercolor" }
bearcad.clear_selection()
bearcad.ui.wait(6)
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
bearcad.ui.wait(4)
assert(bearcad.drawing_views(d)[1].style == "watercolor")
assert(bearcad.drawing_views(d)[2].style == "colorful", "the default is still colorful")

assert(bearcad.ui.widget_id_warnings() == 0, "and the new row keeps its widget ids stable")
print("ok: a drawing sets the style its new projections start in")
bearcad.quit()
