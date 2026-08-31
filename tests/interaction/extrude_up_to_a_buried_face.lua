-- Interaction regression (#988): Extrude's "Up to" picker could not be filled with a face
-- buried behind the solid — the bottom cap of a box you are looking down at.
--
-- Three things stood in the way, and all three had to go:
--   * `picker_focus("Up to")` was a no-op, so the pick mode could not be armed from a script
--     at all and none of this was testable.
--   * The distance field auto-focuses so a depth can be typed the moment a profile is picked,
--     and it held the keyboard — swallowing the Space that opens the Selection Exploder, which
--     is the only way to name a buried face.
--   * Clicking the loupe then grabbed the pull-handle gizmo instead: the fan redirects the
--     pointer to the leaf's anchor, and in a top view that lands right on the handle, since
--     everything on the extrude axis projects to the same spot.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()
bearcad.clear_selection()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {20, 15, 0}, distance = 260 }
bearcad.ui.wait(5)

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
end

bearcad.ui.tool("extrude")
bearcad.ui.wait(5)
-- Pick the top cap as the profile to extrude.
bearcad.ui.click_ground(20, 15)
bearcad.ui.wait(8)
assert(#picker("Faces").items == 1, "the top cap should be the profile")
assert(#picker("Up to").items == 0, "and nothing is the target yet")

-- Arm "Up to". It is a pick *mode*, not a set, so this flips the tool's flag.
bearcad.ui.picker_focus("Up to")
bearcad.ui.wait(6)
assert(picker("Up to").focused, "the Up to picker should be armed")

-- Space must reach the exploder: the distance field gives up the keyboard while a pick is armed.
bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(4)
bearcad.ui.key("space")
bearcad.ui.wait(8)
local leaves = bearcad.ui.exploder()
assert(#leaves > 0, "Space should open the fan while Up to is armed, got " .. #leaves .. " leaves")

-- The buried bottom cap is in the fan, with a loupe to aim at.
local bottom
for _, l in ipairs(leaves) do
  if l.label and l.label:find("bottom") and l.x then bottom = l end
end
assert(bottom, "the fan should offer the bottom cap hidden behind the solid")

bearcad.ui.click(bottom.x, bottom.y)
bearcad.ui.wait(12)

assert(#picker("Up to").items == 1,
  "clicking the bottom cap's loupe should fill the Up to picker, got "
    .. #picker("Up to").items .. " item(s)")

print("ok: Extrude's Up to takes a face buried behind the solid, through the fan")
bearcad.quit()
