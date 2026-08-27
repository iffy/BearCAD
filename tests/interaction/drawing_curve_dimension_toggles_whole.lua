-- #1785/#1781: a smooth curve on a drawing — the tilted cut's elliptical edge — toggles as
-- one whole curve on the Dimension tool: one click shows its length, clicking it again
-- hides it, and its tessellation facets never dimension separately. The curve is found by
-- probing (any geometry response, then a fine scan for the curve), so the test holds at
-- any window size.
bearcad.new()
bearcad.cylinder{ radius = 20, height = 40 }
bearcad.cross_section{}
bearcad.section_plane{ origin = {0, 0, 0}, normal = {0, 1, 0}, offset = 5, flip = true }
bearcad.edit_section_plane{ cut = 0, roll = 25 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_view_section{ drawing = d, view = 0, cross_section = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.wait(8)

local vp = bearcad.ui.viewport()
local cx, cy = vp.width / 2, vp.height / 2

-- The fresh view arrives selected, its ✕ showing; a stray coarse-sweep click could remove
-- it. Blank-page click clears the selection first.
bearcad.ui.click(4, vp.height - 6)
bearcad.ui.wait(4)

bearcad.ui.tool("dimension")
bearcad.ui.wait(4)

local function click(x, y)
  bearcad.ui.move(x, y)
  bearcad.ui.wait(2)
  bearcad.ui.click(x, y)
  bearcad.ui.wait(4)
  local s = bearcad.status()
  bearcad.ui.key("escape")
  bearcad.ui.wait(2)
  bearcad.ui.tool("dimension")
  bearcad.ui.wait(2)
  return s
end

-- Coarse pass: rings out from the viewport centre until a click answers with geometry —
-- an edge toggles, blank card paper arms a free measurement. The body sits at the card's
-- centre, so the rings reach it long before they reach the card corner's ✕.
local anchor, kind
local reach = math.floor(math.min(vp.width, vp.height) * 0.45)
for r = 12, reach, 24 do
  local ring = {}
  for dx = -r, r, 24 do
    ring[#ring + 1] = { dx, -r }
    ring[#ring + 1] = { dx, r }
  end
  for dy = -r + 24, r - 24, 24 do
    ring[#ring + 1] = { -r, dy }
    ring[#ring + 1] = { r, dy }
  end
  for _, p in ipairs(ring) do
    local x, y = cx + p[1], cy + p[2]
    if x > 20 and y > 20 and x < vp.width - 8 and y < vp.height - 8 then
      local s = click(math.floor(x), math.floor(y))
      if s:find("curve") then
        anchor, kind = { math.floor(x), math.floor(y) }, "curve"
        break
      end
      if s:find("edge dimension") or s:find("second point") then
        anchor, kind = { math.floor(x), math.floor(y) },
          s:find("second point") and "blank" or "edge"
        break
      end
    end
  end
  if anchor then break end
end
assert(anchor, "the projection is on the page somewhere")

-- Fine pass: ring out from the anchor until a click names the curve. The arc spans the
-- body's face, so some ring crosses it.
local hit
if kind == "curve" then
  hit = anchor
else
  for r = 6, 96, 6 do
    local ring = {}
    for dx = -r, r, 6 do
      ring[#ring + 1] = { dx, -r }
      ring[#ring + 1] = { dx, r }
    end
    for dy = -r + 6, r - 6, 6 do
      ring[#ring + 1] = { -r, dy }
      ring[#ring + 1] = { r, dy }
    end
    for _, p in ipairs(ring) do
      local s = click(anchor[1] + p[1], anchor[2] + p[2])
      if s:find("curve") then
        hit = { anchor[1] + p[1], anchor[2] + p[2] }
        break
      end
    end
    if hit then break end
  end
end
assert(hit, "the cut curve is dimensionable somewhere on the card")

local function curve_dims()
  return bearcad.drawing_views(d)[1].curve_dimensions
end
assert(curve_dims() == 1, "one curve dimension shows, got " .. tostring(curve_dims()))

-- Clicking the curve again hides it: the whole curve toggles together, facets never
-- dimension separately.
local s = click(hit[1], hit[2])
assert(s:find("Hid curve"), "clicking the curve again hides it, got: " .. s)
assert(curve_dims() == 0, "the curve dimension is gone")

print("ok: a curve dimensions whole — show, hide, never per facet")
bearcad.quit()
