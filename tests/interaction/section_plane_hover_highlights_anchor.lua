-- #1745: the Cutting plane tool hover-highlights what its Anchor picker would take.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.camera{ target = {0, 0, 10}, distance = 260 }

bearcad.cross_section{ name = "Front half" }
bearcad.ui.wait(5)
assert(bearcad.ui.workbench() == "view")

bearcad.ui.tool("section_plane")
bearcad.ui.wait(3)

local anchor
for _, p in ipairs(bearcad.pickers()) do
  if p.name == "Anchor" then anchor = p end
end
assert(anchor, "the cutting plane tool must register an Anchor picker")
assert(anchor.focused, "the anchor is the only thing this tool picks, so it is armed")
local takes = {}
for _, k in ipairs(anchor.accepts) do takes[k] = true end
assert(takes["face"] or takes["profile"] or takes["plane"],
  "the anchor takes a face or plane")
assert(not takes["body"], "but not a whole body")

-- Hover the block's top: a face (or the plane it sits on) should light up.
bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(h, "hovering a pickable anchor should highlight it, got nothing")
assert(h.kind == "face" or h.kind == "plane" or h.kind == "profile",
  "the cutting plane should hover a face or plane, got " .. tostring(h.kind))

print("ok: the cutting plane tool hover-highlights its Anchor options")
bearcad.quit()
