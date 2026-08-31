-- #1737: the Slice tool inside an open sketch mounted *both* its sketch section and its 3D
-- section, so the pane grew two Targets rows and two Cutters rows sharing widget ids — egui
-- painted "First/Second use of widget ID" over the pane and the body pick went nowhere.
bearcad.new()
bearcad.cuboid{ width = 40, depth = 40, height = 20 }
bearcad.ui.wait(5)
-- A sketch on the block's top face, with the Slice tool live inside it.
bearcad.begin_sketch{ kind = "primitive_face", primitive = 0, face = "top" }
bearcad.ui.wait(5)
bearcad.ui.tool("slice")
bearcad.ui.wait(6)

local seen, names = {}, {}
for _, p in ipairs(bearcad.ui.pickers()) do
  names[#names + 1] = p.name
  assert(not seen[p.name],
    "Slice in a sketch mounted two '" .. p.name .. "' pickers: " .. table.concat(names, ", "))
  seen[p.name] = true
end
assert(#names > 0, "the Slice tool should mount its own pickers, got none")

print("ok: the Slice tool shows one section inside a sketch (" .. table.concat(names, ", ") .. ")")
bearcad.quit()
