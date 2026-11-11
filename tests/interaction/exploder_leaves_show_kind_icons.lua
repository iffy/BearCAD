-- #1969: each Selection Exploder loupe wears an icon for what it picks — a body icon on the
-- body, a line icon on an edge, a vertex icon on a corner.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 30, depth = 30, height = 30 }
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("iso")
bearcad.ui.camera{ target = {0, 0, 15}, distance = 160 }
bearcad.ui.wait(10)
bearcad.ui.move_world(15, -15, 30)
bearcad.ui.wait(4)
bearcad.ui.key("space")
bearcad.ui.wait(20)

local icons = {}
for _, leaf in ipairs(bearcad.ui.exploder()) do
  assert(leaf.icon, "every leaf wears a kind icon, " .. leaf.kind .. " has none")
  icons[leaf.kind] = leaf.icon
end
assert(icons.body == "Body", "body leaf icon: " .. tostring(icons.body))
assert(icons.body_edge == "Line", "edge leaf icon: " .. tostring(icons.body_edge))
assert(icons.body_vertex == "Vertex", "corner leaf icon: " .. tostring(icons.body_vertex))

print("ok: exploder leaves show their kind icons")
bearcad.quit()
