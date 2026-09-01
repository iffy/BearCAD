-- #958: the hover match only ever knew the *tool*, so a tool's two pickers hovered
-- identically. The Slice tool is the clearest case: Targets takes bodies and Cutters takes
-- planes and flat faces, but both used to light up the whole body under the cursor.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.zoom_fit()

bearcad.ui.tool("slice")
bearcad.ui.wait("picker")

-- Targets is armed first: a click anywhere on the solid takes the whole body (#218), so
-- that is what hovering it shows.
bearcad.ui.move_ground(20, 15)
local h = bearcad.ui.hovered()
assert(h and h.kind == "body",
  "with Targets armed the body should hover, got " .. tostring(h and h.kind))

-- Arm Cutters. It takes planes and flat faces, never a whole body — so the same cursor
-- position must stop reading as "a body you can pick".
bearcad.ui.picker_focus("Cutters")
local cutters
for _, p in ipairs(bearcad.ui.pickers()) do
  if p.name == "Cutters" then cutters = p end
end
assert(cutters and cutters.focused, "the Cutters picker should be armed")
bearcad.ui.move_ground(20, 15)
h = bearcad.ui.hovered()
assert(h == nil or h.kind ~= "body",
  "Cutters cannot take a body, so it should not hover one — got " .. tostring(h and h.kind))

print("ok: two pickers on one tool hover the different things they take")
bearcad.quit()
