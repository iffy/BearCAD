-- #1821: the Coloured pencil projection style is the viewport's coloured-pencil mode on the
-- page — hand-drawn edges in the body's own colour, shading strokes across every flat, and the
-- solids' shadows falling on each other. It is scene-building code the unit tests exercise only
-- in pieces; driving a real frame is what proves the whole path runs.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 90, depth = 90, height = 12 }
bearcad.cuboid{ at = { 30, 30, 12 }, width = 14, depth = 14, height = 45 }
bearcad.material{ name = "Brick", color = "#c8503c", bodies = {0, 1} }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "iso" }
-- Hide the panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(6)

for _, style in ipairs({ "wireframe", "shaded", "colorful", "loose_pencil", "colour_pencil" }) do
  bearcad.drawing_view_style{ drawing = d, view = 0, style = style }
  bearcad.ui.wait(4)
  assert(bearcad.drawing_views(d)[1].style == style,
    "the view should be in " .. style .. ", got " .. tostring(bearcad.drawing_views(d)[1].style))
end

-- The style survives an export, which walks the same geometry through the print canvas.
local out = os.tmpname() .. ".svg"
bearcad.export_drawing_svg{ drawing = d, path = out }
local f = assert(io.open(out, "r"))
local svg = f:read("a")
f:close()
os.remove(out)
assert(#svg > 1000, "the export drew something, got " .. #svg .. " bytes")

print("ok: a coloured-pencil projection draws and exports")
bearcad.quit()
