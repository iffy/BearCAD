-- Documentation screenshots: one shot per joint kind, each a base slab and an arm joined
-- at the shared corner and posed so the kind's motion reads.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh), falling
-- back to ".". Eight PNGs, joint-kinds-<kind>.png.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/joint-kinds"

local kinds = {
  { kind = "rigid" },
  { kind = "slider", axis_to = {0, 0, 0}, position = 12 },
  { kind = "revolute", position = 45 },
  { kind = "cylindrical", position = 6, position2 = 30 },
  { kind = "planar", position = 8, position2 = 5, position3 = 15 },
  { kind = "ball", position = 20, position2 = 15 },
  { kind = "pin_slot", position = 8, position2 = 25 },
  { kind = "screw", lead = 2, position = 540 },
}

for _, spec in ipairs(kinds) do
  bearcad.new()
  bearcad.ui.pane("elements", "hide")
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  bearcad.rect{ width = 30, height = 20 }
  bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
  bearcad.rect{ x = 40, y = 0, width = 25, height = 8 }
  bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5 }
  bearcad.exit_sketch()

  bearcad.joint{
    a = 0, b = 1, kind = spec.kind, lead = spec.lead,
    from   = { body = 1, vertex = {40, 0, 0} },
    to     = { body = 0, vertex = {30, 0, 0} },
    from_b = { body = 1, vertex = {40, 0, 5} },
    to_b   = { body = 0, vertex = spec.axis_to or {30, 0, 5} },
    position = spec.position, position2 = spec.position2, position3 = spec.position3,
  }

  for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
  bearcad.ui.ground("off")
  bearcad.ui.view("corner", "front_left_top")
  bearcad.ui.wait(2)
  bearcad.ui.zoom_fit()
  bearcad.ui.wait(2)
  bearcad.ui.screenshot(out .. "-" .. spec.kind .. ".png")
end

bearcad.quit()
