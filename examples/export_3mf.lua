-- Example — extrude a rectangle into a box and export it as a 3MF package.
-- Multi-body docs export each body as its own coloured object (material → basematerials).
-- Run: cargo run -- --script examples/export_3mf.lua --exit

bearcad.new()

bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20, name = "Block" }

-- Export every body in the document to a 3MF file.
bearcad.export_3mf("block.3mf")

-- A single named body can be exported on its own:
-- bearcad.export_3mf("block.3mf", "Block")

bearcad.quit()
