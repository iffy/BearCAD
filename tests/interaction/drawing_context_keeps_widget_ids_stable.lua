-- #1828: the drawing view's context section mounted two rows with *no* label, and
-- `labeled_row` salts its id with the label text — so "Projection lines" (only there for an
-- aligned view) and "Remove view" shared one id. egui logged a multipass id clash every time
-- the pane laid out, which is the kind of warning that buries the real ones.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
-- An aligned child is the case that mounts both rows at once.
bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "below", pos = 0.72 }
bearcad.ui.wait(6)

-- Select the aligned child so the context pane shows its section, projection-lines row and all.
local vp = bearcad.ui.viewport()
local card = bearcad.ui.drawing_view_rect(1)
assert(card, "the aligned child has a card on the page")
bearcad.ui.click(card.x + card.w / 2 - vp.x, card.y + card.h / 2 - vp.y)
bearcad.ui.wait(10)

local w = bearcad.debug.widget_id_warnings()
assert(w == 0,
  "an aligned view's context section must keep its widget ids stable, got " .. tostring(w))

-- The ✕ chrome on a card mounts only while it is selected or hovered (#1229), and whether a
-- card is hovered can resolve differently between egui's two passes — so its id has to be its
-- own rather than one counted off its siblings.
for _ = 1, 3 do
  bearcad.ui.move(card.x + card.w - 14 - vp.x, card.y + 14 - vp.y)
  bearcad.ui.wait(3)
  bearcad.ui.move(card.x + card.w / 2 - vp.x, card.y + card.h / 2 - vp.y)
  bearcad.ui.wait(3)
end
w = bearcad.debug.widget_id_warnings()
assert(w == 0, "hovering a card's remove button must not either, got " .. tostring(w))

-- Every row the pane draws for a view, driven by name (#1828): hovering each one, and
-- opening the Style dropdown so a popup layer lands over the pane, is the shape of the
-- session the warnings were reported from.
for _, view in ipairs({ 0, 1 }) do
  local c = bearcad.ui.drawing_view_rect(view)
  bearcad.ui.click(c.x + c.w / 2 - vp.x, c.y + c.h / 2 - vp.y)
  bearcad.ui.wait(8)
  for _, name in ipairs({ "Shows", "Style", "Scale", "align_lines", "Label", "Text" }) do
    local r = bearcad.ui.context_row_rect(name)
    if r then
      bearcad.ui.move(r.x + r.w / 2 - vp.x, r.y + r.h / 2 - vp.y)
      bearcad.ui.wait(3)
    end
  end
  local style = assert(bearcad.ui.context_row_rect("Style"), "the view has a Style row")
  bearcad.ui.click(style.x + style.w - 20 - vp.x, style.y + style.h / 2 - vp.y)
  bearcad.ui.wait(6)
  for i = 0, 8 do
    bearcad.ui.move(style.x + style.w - 40 - vp.x, style.y + style.h + 4 + i * 20 - vp.y)
    bearcad.ui.wait(2)
  end
  bearcad.ui.key("Escape")
  bearcad.ui.wait(4)
end
w = bearcad.debug.widget_id_warnings()
assert(w == 0, "walking every row and the Style popup must not either, got " .. tostring(w))

print("ok: the drawing context section keeps its widget ids stable")
bearcad.quit()
