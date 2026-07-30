-- #967: the drawing workbench had its own selection world — a label-only combo box over
-- `DrawingElementRef`s, a nested `Option<Option<..>>` standing in for a single-pick input, and
-- no scene element for anything on a page. Its items are elements now, so its inputs are
-- ordinary pickers that hover, register and report like every other.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.tool("select")
bearcad.ui.wait(5)
local sel = picker("Selection")
assert(sel, "the drawing Select tool should register a Selection picker")
assert(sel.focused, "and it should be armed")
-- The three page-item kinds are their own kinds, which is what keeps each row's icon (#363)
-- and lets the Aligned-view tool ask for projections alone.
local takes = {}
for _, k in ipairs(sel.accepts) do takes[k] = true end
assert(takes["projection"] and takes["annotation"] and takes["dimension"],
  "it takes the three drawing kinds")
assert(not takes["body"], "and nothing from the model")

-- The Aligned-view tool's base view is a single-pick projection input.
bearcad.ui.tool("drawing_align")
bearcad.ui.wait(5)
local base = picker("Base view")
assert(base, "the Aligned-view tool should register a Base view picker")
assert(base.limit == 1, "it takes one projection, got limit " .. tostring(base.limit))
assert(#base.accepts == 1 and base.accepts[1] == "projection",
  "and only projections")

print("ok: the drawing workbench's inputs are element pickers")
bearcad.quit()
