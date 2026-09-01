-- #1910: a zoom loupe is an Elements-pane row you can select, and selecting it
-- offers a Style of its own — independent of the projection it sits on.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 60, depth = 40, height = 20 }
local d = bearcad.drawing{ name = "Loupes" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view_style{ drawing = d, view = 0, style = "wireframe" }
bearcad.drawing_loupe{ drawing = d, view = 0, at = {5, 5}, radius = 4,
                       to = {40, -30}, to_radius = 20 }
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(10)

assert(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style == "view",
  "a new loupe follows the view")

local row = assert(bearcad.ui.elements_row_rect("Loupe 0 (5.0×)"),
  "the loupe should appear in the Elements pane")

local function kinds()
  local t = {}
  for _, e in ipairs(bearcad.selection()) do t[#t + 1] = e.kind end
  return table.concat(t, ",")
end

-- Placing a loupe selects it; pick the projection from Elements, then the loupe.
local proj = assert(bearcad.ui.elements_row_rect("Cuboid 0 — Front")
  or bearcad.ui.elements_row_rect("Body 0 — Front"),
  "the projection should be in the Elements pane")
bearcad.ui.tool("select")
bearcad.ui.click(proj)
bearcad.ui.wait(6)
assert(kinds():find("projection"), "clicking the projection row selects it, got " .. kinds())
assert(bearcad.ui.context_row_rect("Shows"), "the projection editor is open")

bearcad.ui.click(row)
bearcad.ui.wait(6)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "drawing_loupe",
  "clicking the Elements row selects the loupe, got " .. kinds())

assert(bearcad.ui.context_row_rect("Style"),
  "a selected loupe has a Style row")
assert(not bearcad.ui.context_row_rect("Shows"),
  "and not the projection's View editor")

-- Scripts can name the same row.
bearcad.select{ kind = "drawing_loupe", drawing = d, view = 0, index = 0 }
bearcad.ui.wait(4)
sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "drawing_loupe",
  "bearcad.select names the loupe")

-- Pick Shaded from the Style combo: the loupe restyles, the view does not.
local style = assert(bearcad.ui.context_row_rect("Style"), "the Style combo is on the pane")
bearcad.ui.click(style)
bearcad.ui.wait(6)
local function has(label)
  for _, it in ipairs(bearcad.ui.menu_items()) do
    if it:gsub("^✓ ", "") == label then return it end
  end
end
assert(has("Same as the view"), "the combo offers following the view, got "
  .. table.concat(bearcad.ui.menu_items(), ", "))
local shaded = assert(has("Shaded"), "and every drawing style, got "
  .. table.concat(bearcad.ui.menu_items(), ", "))
local r = assert(bearcad.ui.menu_item_rect(shaded), "Shaded reports where it drew")
bearcad.ui.click(r)
bearcad.ui.wait(4)
assert(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style == "shaded",
  "picking Shaded restyles the loupe, got "
    .. tostring(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style))
assert(bearcad.drawing_views(d)[1].style == "wireframe",
  "the view keeps its own style")

print("ok: a zoom loupe is an Elements-pane item with its own style")
bearcad.quit()
