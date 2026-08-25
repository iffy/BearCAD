-- #1701: with the Extrude tool armed on a sketch text, the Selection Exploder fanned the
-- crowd around the letters but never the letters themselves — even though a click there picks
-- them. What the armed picker can take, the fan has to offer.
bearcad.new()
bearcad.rect{ width = 60, height = 40 }
bearcad.text{ text = "BEAR", size = 12, x = 2, y = 2 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)

assert(bearcad.count("sketch_text") == 1, "the sketch has a text")

local function fan_at(x, y)
  bearcad.ui.move_ground(x, y)
  bearcad.ui.wait(4)
  bearcad.ui.key("space")
  bearcad.ui.wait(6)
  local kinds = {}
  for _, leaf in ipairs(bearcad.exploder()) do
    kinds[#kinds + 1] = leaf.kind
  end
  bearcad.ui.key("escape")
  bearcad.ui.wait(4)
  return kinds
end

-- Walk across the string until a glyph is under the cursor, then fan there.
local found = nil
for x = 1, 40 do
  local kinds = fan_at(x, 6)
  for _, k in ipairs(kinds) do
    if k == "sketch_text" then found = { x = x, kinds = kinds } break end
  end
  if found then break end
end
assert(found, "the fan should offer the letters the Extrude tool can pick")

-- And picking that loupe takes the whole string, exactly as a click on the letters does.
bearcad.ui.move_ground(found.x, 6)
bearcad.ui.wait(4)
bearcad.ui.key("space")
bearcad.ui.wait(6)
local loupe
for _, leaf in ipairs(bearcad.exploder()) do
  if leaf.kind == "sketch_text" and leaf.x then loupe = leaf end
end
assert(loupe, "the fan still holds the letters, with a loupe to click")
bearcad.ui.click(loupe.x, loupe.y)
bearcad.ui.wait(8)
local picked = 0
for _, p in ipairs(bearcad.pickers()) do
  if p.name == "Faces" then picked = #p.items end
end
assert(picked >= 1, "picking the loupe should feed the Extrude tool the letters, got " .. picked)

print("ok: the exploder offers the sketch text the Extrude tool takes")
bearcad.quit()
