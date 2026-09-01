-- Interaction regression (#1850): a zoom loupe draws its detail in whatever style the view
-- uses, which is often not the one you magnified it for — a wireframe view's loupe shows the
-- same bare edges, bigger. Right-clicking a loupe now offers every drawing style, plus "Same
-- as the view" to hand it back.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 60, depth = 40, height = 20 }
local d = bearcad.drawing{ name = "Loupes" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view_style{ drawing = d, view = 0, style = "wireframe" }
bearcad.drawing_loupe{ drawing = d, view = 0, at = {5, 5}, radius = 4, to = {40, -30}, to_radius = 20 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.wait(10)

local l = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(l.style == "view", "a new loupe follows the view, got " .. tostring(l.style))

local rect = assert(bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true },
  "the page reports where it drew the magnified circle")

bearcad.ui.right_click(rect)
local items = bearcad.ui.menu_items()
assert(#items > 0, "right-clicking a loupe should open its menu")
local function has(label)
  for _, it in ipairs(items) do if it:gsub("^✓ ", "") == label then return it end end
end
assert(has("Same as the view"), "the menu offers following the view, got "
  .. table.concat(items, ", "))
local shaded = assert(has("Shaded"), "and every drawing style, got " .. table.concat(items, ", "))

local r = assert(bearcad.ui.menu_item_rect(shaded), "the Shaded item reports where it drew")
bearcad.ui.click(r)
assert(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style == "shaded",
  "picking Shaded restyles the loupe, got "
  .. tostring(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style))
-- The view itself is untouched — only the loupe changed.
assert(bearcad.drawing_views(d)[1].style == "wireframe", "the view keeps its own style")

-- Scripts set it the same way, and hand it back.
bearcad.edit_drawing_loupe{ drawing = d, view = 0, index = 0, style = "colorful" }
assert(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style == "colorful",
  "the style is scriptable")
bearcad.edit_drawing_loupe{ drawing = d, view = 0, index = 0, style = "view" }
assert(bearcad.drawing_loupes{ drawing = d, view = 0 }[1].style == "view",
  "and can follow the view again")

print("ok: a loupe picks its own drawing style")
bearcad.quit()
