-- #970: Combine, Slice and Mirror each re-did resolve-pick-and-toggle by hand, and drifted:
-- Combine's "a body lives on one side only" rule existed only in its viewport handler, and the
-- Mirror tool's bodies were unreachable from the Elements pane at all. All three now offer the
-- click to the focused picker, which is the same path the pane and the hover use.
bearcad.new()
bearcad.rect{ width = 30, height = 20 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
bearcad.exit_sketch()
bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 40, y = 0, width = 30, height = 20 }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 10 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.zoom_fit()

local function picker(name)
  for _, p in ipairs(bearcad.ui.pickers()) do
    if p.name == name then return p end
  end
  return nil
end

-- Combine: a click on each body fills side A.
bearcad.ui.tool("combine")
bearcad.ui.wait("picker")
bearcad.ui.click_ground(15, 10)
bearcad.ui.click_ground(55, 10)
local a = picker("Bodies") or picker("Side A")
assert(a and #a.items == 2, "both bodies should be on side A, got " .. #(a and a.items or {}))
-- Clicking one again takes it back out.
bearcad.ui.click_ground(15, 10)
a = picker("Bodies") or picker("Side A")
assert(#a.items == 1, "re-clicking a picked body drops it, got " .. #a.items)

-- Mirror: the plane comes first, then bodies — the pickers' own order, not a branch in the
-- tool. The XY datum plane is under the solids in the top view.
bearcad.ui.tool("mirror")
bearcad.ui.wait("picker")
local plane = picker("Mirror plane")
assert(plane and plane.focused, "the mirror plane is the first pick")
bearcad.ui.click_ground(55, 10)
plane = picker("Mirror plane")
assert(#plane.items == 1, "the click should set the mirror plane, got " .. #plane.items)
local bodies = picker("Bodies")
assert(bodies and bodies.focused, "with a plane set, the bodies picker takes over")
-- Switching from Combine carried its side-A body across (#956), so count the change rather
-- than the total: the click gathers the body it lands on.
local before = #bodies.items
bearcad.ui.click_ground(15, 10)
assert(#picker("Bodies").items == before + 1,
  "the next click should gather a body, got " .. #picker("Bodies").items)

-- Slice: which set a click feeds is the armed picker's business. The first target then
-- hands focus to Cutters (#1154), matching Mirror's "plane then bodies" step-through.
bearcad.ui.tool("slice")
bearcad.ui.wait("picker")
assert(picker("Targets").focused, "Targets is armed first")
bearcad.ui.click_ground(15, 10)
assert(#picker("Targets").items == 1,
  "Targets is armed first, so the body lands there, got " .. #picker("Targets").items)
assert(#picker("Cutters").items == 0, "and not in Cutters")
assert(picker("Cutters").focused,
  "after the first target, Cutters should take focus")

print("ok: one click path gathers for Combine, Mirror and Slice")
bearcad.quit()
