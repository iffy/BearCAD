-- #968: `bearcad.exploder()` reports what the Selection Exploder is fanning out, so the rule
-- that the fan should offer exactly what the focused picker can take (#957) is assertable.
-- Nothing else exposes the loupes.
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
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

assert(#bearcad.exploder() == 0, "a closed exploder fans nothing")

-- A corner of the solid: several things stack there — the corner, its edges, the faces
-- meeting at it, the sketch geometry under them, and the body itself.
bearcad.ui.move_ground(40, 30)
bearcad.ui.wait(3)
bearcad.ui.key("space")
bearcad.ui.wait(5)

local leaves = bearcad.exploder()
assert(#leaves > 1, "a crowded corner should fan several leaves, got " .. #leaves)

local kinds = {}
for _, leaf in ipairs(leaves) do kinds[leaf.kind] = true end
assert(kinds["body_vertex"], "the corner itself should be in the fan")
assert(kinds["body_edge"], "and the edges meeting at it")
-- The Select tool takes whole bodies (#902), so the body is its own leaf.
assert(kinds["body"], "and the whole body")

bearcad.ui.key("escape")
bearcad.ui.wait(5)
assert(#bearcad.exploder() == 0, "dismissing the fan empties it")

print("ok: the exploder's fan is readable from a script")
bearcad.quit()
