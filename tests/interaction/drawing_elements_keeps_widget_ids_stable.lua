-- #1911: on the drawing workbench with the Select tool and nothing selected, Elements-pane
-- rows and the view card used auto ids. A neighbour that mounted in one of egui's two
-- passes (hover chrome, the Dimension hint) gave the same rect two ids.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.clear_selection()
bearcad.ui.wait(8)

assert(bearcad.ui.workbench() == "drawing", "the drawing is open")
assert(bearcad.debug.widget_id_warnings() == 0,
  "opening a drawing must not change widget ids between passes, got "
    .. tostring(bearcad.debug.widget_id_warnings()))

-- Hover every Elements row: hover chrome and tooltips resolve differently across passes.
for _, name in ipairs({ "Cuboid 0", "Drawings", "Drawing 0" }) do
  local row = bearcad.ui.elements_row_rect(name)
  if row then
    bearcad.ui.move(row)
    bearcad.ui.wait(4)
  end
end
bearcad.ui.wait(4)
assert(bearcad.debug.widget_id_warnings() == 0,
  "hovering Elements rows must keep their widget ids, got "
    .. tostring(bearcad.debug.widget_id_warnings()))

-- Hover the view card on and off so its chrome (border + Remove ✕) mounts and unmounts.
local card = assert(bearcad.ui.drawing_view_rect(0), "the projection has a card on the page")
for _ = 1, 3 do
  bearcad.ui.move(card)
  bearcad.ui.wait(3)
  bearcad.ui.move({ x = card.x - 20, y = card.y - 20 })
  bearcad.ui.wait(3)
end
assert(bearcad.debug.widget_id_warnings() == 0,
  "hovering the view card must keep its widget ids, got "
    .. tostring(bearcad.debug.widget_id_warnings()))

print("ok: drawing Elements rows and the view card keep their widget ids")
bearcad.quit()
