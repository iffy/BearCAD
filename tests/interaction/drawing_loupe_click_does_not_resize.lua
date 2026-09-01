-- Interaction regression (#1909): clicking a zoom loupe's resize band used to snap the
-- radius to the click — the rim followed the pointer on press, not on drag. A click must
-- only select; the circle resizes when you drag the band.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 60, depth = 40, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.drawing_loupe{ drawing = d, view = 0, at = {5, 5}, radius = 6,
                       to = {45, -35}, to_radius = 24 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.wait(10)

bearcad.ui.tool("select")
bearcad.ui.wait(4)

local vp = bearcad.ui.viewport()
local rect = assert(bearcad.ui.drawing_loupe_rect{ view = 0, index = 0, magnified = true },
  "the page reports where it drew the magnified circle")
assert(rect.band and rect.band > 4, "the rim band has a width, got " .. tostring(rect.band))
local cx, cy = rect.x + rect.w / 2, rect.y + rect.h / 2
local r = rect.w / 2
-- Mid-band, not the stroke: that's where a press used to shrink the circle to the pointer.
local band_x = cx + r - rect.band * 0.5
local before = bearcad.drawing_loupes{ drawing = d, view = 0 }[1].to_radius

bearcad.ui.move(band_x - vp.x, cy - vp.y)
bearcad.ui.wait(3)
bearcad.ui.click(band_x - vp.x, cy - vp.y)
bearcad.ui.wait(6)

local after_click = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(math.abs(after_click.to_radius - before) < 1e-3,
  string.format("a click on the resize band must not resize: %.3f → %.3f",
    before, after_click.to_radius))
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "drawing_loupe",
  "the click still selects the loupe")

-- Dragging the same band still resizes.
bearcad.ui.drag(band_x - vp.x, cy - vp.y, band_x - vp.x + 28, cy - vp.y)
bearcad.ui.wait(6)
local grown = bearcad.drawing_loupes{ drawing = d, view = 0 }[1]
assert(grown.to_radius > after_click.to_radius + 0.5,
  string.format("a drag on the band still resizes: %.3f → %.3f",
    after_click.to_radius, grown.to_radius))

print("ok: a click on a loupe's resize band selects it; only a drag resizes")
bearcad.quit()
