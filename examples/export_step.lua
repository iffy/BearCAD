-- Example — extrude a rectangle into a box and export it as a STEP file.
-- Run: cargo run -- --script examples/export_step.lua --exit

bearcad.new()

bearcad.rect{ width = 80, height = 50, name = "Base" }
bearcad.extrude{ rect = 0, distance = 20, name = "Block" }

bearcad.export_step("block.step")

-- A single named body can be exported on its own:
-- bearcad.export_step("block.step", "Block")

bearcad.quit()