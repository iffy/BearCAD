-- Example — extrude a rectangle into a box and export it as an STL file.
-- Run: cargo run -- --script examples/export_stl.lua --exit

bearcad.new()

-- 80 x 50 mm rectangle on the default ground plane...
local sides = bearcad.rect{ width = 80, height = 50, name = "Base" }

-- ...extruded 20 mm into a solid body.
local box = bearcad.extrude{ profiles = sides, distance = 20, name = "Block" }

-- Export every body in the document to an STL file.
bearcad.export_stl("block.stl")

-- A single named body can be exported on its own:
-- bearcad.export_stl("block.stl", "Block")

bearcad.quit()
