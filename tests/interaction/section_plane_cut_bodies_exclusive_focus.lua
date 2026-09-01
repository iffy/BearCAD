-- #1904: clicking Cut bodies must take exclusive focus. Offset used to keep its
-- keyboard ring (pending_focus re-request) while the picker also showed a ring.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 40, height = 40 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }

bearcad.cross_section{ name = "Cut" }
bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)
assert(picker("Cut bodies"), "an anchored draft shows Cut bodies")
assert(not picker("Cut bodies").focused, "Offset holds the keyboard after the pick (#1750)")

local row = assert(
  bearcad.ui.context_row_rect("Cut bodies"),
  "the Cut bodies input is in the context pane"
)
-- Click the combo, not the label column.
bearcad.ui.click({ x = row.x + row.w - 24, y = row.y + row.h / 2 })
bearcad.ui.wait(5)
assert(picker("Cut bodies").focused, "clicking Cut bodies arms it")

-- Typing must not land in Offset: if Offset still held the keyboard, "999" would
-- become the offset and Enter would hang the plane at 999 mm.
bearcad.ui.type("999")
bearcad.ui.wait(3)
bearcad.ui.key("Enter")
bearcad.ui.wait(5)

local cuts = bearcad.section_planes()
assert(#cuts == 1, "Enter still accepts the plane, got " .. #cuts)
assert(
  math.abs(cuts[1].offset) < 50,
  "typing while Cut bodies is focused must not edit Offset, got " .. cuts[1].offset
)

print("ok: clicking Cut bodies takes exclusive focus from Offset")
bearcad.quit()
