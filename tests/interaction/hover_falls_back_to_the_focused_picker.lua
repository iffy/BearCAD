-- #958: the hover path was a per-tool match ending in `_ => None`, so a dozen tools showed no
-- hover feedback at all — a click that worked lit up nothing. Tools with no hand-written arm
-- now ask the focused picker what a click would take, and light that.
--
-- The Mirror tool outside a sketch is one: the hover match only has an in-sketch Mirror arm, so
-- in 3D — where its pickers take a plane and bodies — hovering showed nothing at all.
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
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

-- The Select tool has its own hover arm; check the harness sees a highlight at all.
bearcad.ui.tool("select")
bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(5)
local h = bearcad.hovered()
assert(h and h.kind == "body", "Select should hover the body, got " .. tostring(h and h.kind))

-- The 3D Mirror tool has no hover arm. Its Bodies picker takes whole bodies, so the body under
-- the cursor is what a click would take — and it should light up.
bearcad.ui.tool("mirror")
bearcad.ui.wait(5)
bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(5)
h = bearcad.hovered()
assert(h, "the Mirror tool should hover something its pickers can take, got nothing")
assert(h.kind == "body" or h.kind == "construction_plane" or h.kind == "face",
  "and it should be a plane, face or body — got " .. h.kind)

-- The Joint tool renders its Parts picker in place among its own controls, but it is
-- registered like every other picker (#958) — so hover, the handoff and scripts can all see
-- it, and a body under the cursor is what a click would take.
bearcad.ui.tool("joint")
bearcad.ui.wait(5)
local parts
for _, p in ipairs(bearcad.pickers()) do
  if p.name == "Parts" then parts = p end
end
assert(parts, "the Joint tool's Parts picker should be visible to scripts")
assert(parts.focused, "and focused with nothing picked yet")
bearcad.ui.move_ground(20, 15)
bearcad.ui.wait(5)
h = bearcad.hovered()
assert(h and h.kind == "body",
  "the Joint tool should hover the body its picker takes, got " .. tostring(h and h.kind))

print("ok: a tool with no hover arm shows what its focused picker would take")
bearcad.quit()
