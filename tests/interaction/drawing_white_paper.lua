-- #1831: the drawing editor's sheet is dark to match the app, but a drawing can be put on
-- white paper to see what the print will actually look like without exporting it. It is a
-- property of the drawing, so it survives leaving the workbench and coming back.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 40, depth = 30, height = 20 }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, body = 0, orientation = "front" }
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(8)
assert(bearcad.get{ kind = "drawing", index = d }.paper == "dark", "the sheet starts dark")

bearcad.drawing_paper{ drawing = d, paper = "white" }
bearcad.ui.wait(8)
assert(bearcad.get{ kind = "drawing", index = d }.paper == "white", "and switches to white")

-- Real frames on the white sheet: everything the page draws — cards, captions, dimensions,
-- the styled geometry — goes through the same ink, and a panic in any of it fails here.
bearcad.ui.workbench("model")
bearcad.ui.wait(5)
bearcad.ui.workbench("drawing")
bearcad.ui.wait(8)
assert(bearcad.get{ kind = "drawing", index = d }.paper == "white",
  "the paper is the drawing's own, so it survives leaving and coming back")

for _, style in ipairs({ "shaded", "colorful", "loose_pencil", "color_pencil", "watercolor" }) do
  bearcad.drawing_view_style{ drawing = d, view = 0, style = style }
  bearcad.ui.wait(4)
end

bearcad.drawing_paper{ drawing = d, paper = "dark" }
bearcad.ui.wait(6)
assert(bearcad.get{ kind = "drawing", index = d }.paper == "dark", "and back to the dark sheet")

print("ok: a drawing can be previewed on white paper")
bearcad.quit()
