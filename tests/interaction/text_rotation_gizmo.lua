-- #1570: the Text tool shows a rotation gizmo while creating/editing. Drag the
-- handle to turn the selected text about its origin.
bearcad.new()
bearcad.text{ text = "Bear", x = 0, y = 0, size = 10 }
bearcad.ui.tool("text")
-- Select after the tool switch so the Text tool's prune doesn't drop it.
bearcad.select{ kind = "sketch_text", index = 0 }
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {0, 0, 0}, distance = 160, projection = "orthographic" }
bearcad.ui.wait(8)

local rot
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name == "text_rotation" then rot = g end
end
assert(rot, "Text tool should show a text_rotation gizmo on the selected text")
assert(rot.position, "rotation handle needs a world position")

local p = rot.position
bearcad.ui.drag_ground(p.x, p.y, p.x, p.y + 20)
bearcad.ui.wait(8)

local after
for _, g in ipairs(bearcad.ui.gizmos()) do
  if g.name == "text_rotation" then after = g end
end
assert(after, "rotation gizmo still present after the drag")
assert(math.abs(after.value) > 1e-3,
  "dragging the text rotation handle should turn the text, got " .. tostring(after.value))

print("ok: Text tool rotation gizmo turns the selected text")
bearcad.quit()
