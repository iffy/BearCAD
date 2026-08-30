-- Interaction regression (#1846): the Zoom loupe tool. Four clicks lay one down — the
-- detail circle's centre on a projection then its rim, the magnified circle's centre then
-- its rim. Afterwards either circle selects on its own, moves by its middle, and resizes
-- by its rim; deleting either drops the pair.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 60, depth = 40, height = 20 }
local d = bearcad.drawing{ name = "Loupes" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(8)

assert(#bearcad.drawing_loupes{ drawing = d, view = 0 } == 0, "a new view has no loupes")

local vp = bearcad.ui.viewport()
local card = assert(bearcad.ui.drawing_view_rect(0), "the projection is on the page")
local function click(x, y)
  bearcad.ui.move(x - vp.x, y - vp.y)
  bearcad.ui.wait(3)
  bearcad.ui.click(x - vp.x, y - vp.y)
  bearcad.ui.wait(4)
end

bearcad.ui.tool("drawing_loupe")
bearcad.ui.wait(4)
assert(bearcad.ui.tool() == "drawing_loupe", "the loupe tool is armed, got " .. bearcad.ui.tool())

-- Centre the detail circle on the card's top-left corner region, then size it.
local cx, cy = card.x + card.w * 0.3, card.y + card.h * 0.4
click(cx, cy)
assert(#bearcad.drawing_loupes{ drawing = d, view = 0 } == 0, "one click does not commit a loupe")
click(cx + 20, cy)
-- The magnified circle goes lower on the page, well clear of the card.
local mx, my = card.x + card.w * 0.5, card.y + card.h + 90
click(mx, my)
assert(#bearcad.drawing_loupes{ drawing = d, view = 0 } == 0, "three clicks still make no loupe")
click(mx + 55, my)

local loupes = bearcad.drawing_loupes{ drawing = d, view = 0 }
assert(#loupes == 1, "the fourth click commits the loupe, got " .. #loupes)
local l = loupes[1]
assert(l.radius > 0 and l.to_radius > l.radius,
  string.format("the magnified circle is the bigger one: %.2f vs %.2f", l.to_radius, l.radius))
assert(math.abs(l.zoom - l.to_radius / l.radius) < 1e-4, "zoom is the ratio of the radii")

-- Placing one selects its magnified circle, so it can be nudged straight away.
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "drawing_loupe",
  "the new loupe is selected, got " .. tostring(sel[1] and sel[1].kind))

-- Back on Select, dragging the magnified circle's middle moves it.
bearcad.ui.tool("select")
bearcad.ui.wait(4)
local before = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
bearcad.ui.drag(mx - vp.x, my - vp.y, mx - vp.x + 40, my - vp.y - 15)
bearcad.ui.wait(6)
local moved = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(math.abs(moved.to[1] - before.to[1]) > 1,
  string.format("the magnified circle moved: %.2f → %.2f", before.to[1], moved.to[1]))
assert(math.abs(moved.to_radius - before.to_radius) < 1e-3,
  "a move from the middle does not resize it")
assert(math.abs(moved.at[1] - before.at[1]) < 1e-3, "and leaves the detail circle alone")

-- Dragging the rim resizes instead of moving. The page reports where it drew each circle,
-- so the drag can start exactly on the rim.
local rect = assert(
  bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true },
  "the page reports where it drew the magnified circle"
)
local ccx, ccy = rect.x + rect.w / 2, rect.y + rect.h / 2
bearcad.ui.drag(rect.x + rect.w - vp.x, ccy - vp.y, rect.x + rect.w + 25 - vp.x, ccy - vp.y)
bearcad.ui.wait(6)
local grown = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(grown.to_radius > moved.to_radius + 0.5,
  string.format("the rim drag enlarged it: %.2f → %.2f", moved.to_radius, grown.to_radius))
assert(math.abs(grown.to[1] - moved.to[1]) < 1e-3, "and left the centre where it was")

-- #1851: a selected circle says where to grab. The page reports the two zones — `band` px
-- of rim resize, everything inside moves — at exactly the sizes it paints them, so a script
-- aims at what a user sees. Grabbing just inside the band must move, not resize.
local zones = assert(bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true })
assert(zones.band and zones.band > 0, "the rim band has a width, got " .. tostring(zones.band))
assert(zones.handle and zones.handle > 0, "the centre handle has a size")
assert(zones.band < zones.w / 2, "the band is a rim, not the whole disc")
local inner = zones.x + zones.w - zones.band - 3
bearcad.ui.drag(inner - vp.x, ccy - vp.y, inner - vp.x + 18, ccy - vp.y)
bearcad.ui.wait(6)
local nudged = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(math.abs(nudged.to_radius - grown.to_radius) < 1e-3,
  string.format("a grab inside the band moves rather than resizes: %.2f → %.2f",
    grown.to_radius, nudged.to_radius))
assert(math.abs(nudged.to[1] - grown.to[1]) > 1, "and it did move")

-- Scripts move and resize either circle too.
bearcad.edit_drawing_loupe{ drawing = d, view = 0, index = 0, radius = 6, to_radius = 24 }
local sized = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(math.abs(sized.radius - 6) < 1e-4 and math.abs(sized.to_radius - 24) < 1e-4,
  "the radii are scriptable")
assert(math.abs(sized.zoom - 4) < 1e-4, "and the zoom follows, got " .. sized.zoom)

-- A loupe is a plane element like any other: it round-trips through a save/open.
local saved = os.tmpname() .. ".bearcad"
bearcad.save(saved)
bearcad.ui.wait(4)
bearcad.open(saved)
bearcad.ui.wait(8)
os.remove(saved)
local reloaded = bearcad.drawing_loupes{ drawing = 0, view = 0 }
assert(#reloaded == 1 and math.abs(reloaded[1].to_radius - 24) < 1e-4,
  "the loupe survives a round trip")

bearcad.delete_drawing_loupe{ drawing = 0, view = 0, index = 0 }
assert(#bearcad.drawing_loupes{ drawing = 0, view = 0 } == 0, "and can be dropped again")

print("ok: the zoom loupe tool places, moves, resizes, and drops loupes")
bearcad.quit()
