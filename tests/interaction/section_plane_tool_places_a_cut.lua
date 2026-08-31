-- Interaction regression (#1687/#1745): in the View workbench, the Cutting plane tool
-- picks an anchor into the shared picker (click does not commit), then Enter / the blue
-- accept button hangs the plane. Each further pick+accept adds another.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
-- Hide the side panes and pin the camera so ground coordinates land where we expect.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 260 }

bearcad.cross_section{ name = "Front half" }
bearcad.ui.wait(5)
assert(bearcad.ui.workbench() == "view")
assert(#bearcad.section_planes() == 0, "a new view cuts with nothing")

bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

local function anchor()
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == "Anchor" then return p end
  end
  return nil
end

local a = anchor()
assert(a, "the cutting plane tool should register an Anchor picker")
assert(a.focused, "the anchor is armed")
assert(#a.items == 0, "starting empty, got " .. #a.items)

-- Click the top face of the block: fills the picker, does not commit.
bearcad.ui.click_ground(0, 0)
bearcad.ui.wait(5)
a = anchor()
assert(a and #a.items == 1, "the click filled the Anchor picker, got " .. tostring(a and #a.items))
assert(#bearcad.section_planes() == 0, "a click must not hang the plane; the blue accept / Enter does")

bearcad.ui.key("Enter")
bearcad.ui.wait(5)
local cuts = bearcad.section_planes()
assert(#cuts == 1, "Enter committed the cutting plane, got " .. #cuts)

-- The context pane's numbers drive a committed plane: slide, turn, flip.
bearcad.edit_section_plane{ cut = 0, offset = 4, roll = 30, flip = true }
cuts = bearcad.section_planes()
assert(math.abs(cuts[1].offset - 4) < 1e-4, "offset " .. cuts[1].offset)
assert(math.abs(cuts[1].roll - 30) < 1e-3, "roll " .. cuts[1].roll)
assert(cuts[1].flip, "flip")

-- A second pick+accept adds a second plane rather than replacing the first.
bearcad.ui.click_ground(20, 20)
bearcad.ui.wait(5)
assert(#bearcad.section_planes() == 1, "the second click is another draft, not a second cut")
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
assert(#bearcad.section_planes() == 2, "a second accept joins the first")

bearcad.delete_section_plane{ cut = 1 }
assert(#bearcad.section_planes() == 1, "and can be dropped again")

print("ok: the cutting plane tool picks, accepts, adjusts, and stacks planes")
bearcad.quit()
