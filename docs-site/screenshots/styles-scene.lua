-- Documentation screenshot: the Styles page hero — a cube with a cylinder poking out,
-- body selected so the selection aura shows (#160). Hover states can't be scripted, so
-- the per-style swatches on that page are generated directly (src/style_swatches.rs).

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/styles-scene.png"

bearcad.new()
-- Hide the side panes so the captured viewport is landscape (#150).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- The cube.
bearcad.rect{ x = 0, y = 0, width = 80, height = 80, name = "Base" }
bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = 50, name = "Cube" }

-- A cylinder poking out of its top face, merged into the same body.
bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = { 0, 1, 2, 3 }, top = true }
bearcad.circle{ x = 40, y = 40, radius = 18 }
bearcad.extrude{ circle = 0, distance = 35, body = "merge", name = "Boss" }
bearcad.exit_sketch()

bearcad.set_visible({ kind = "sketch", index = 0 }, "hide")
bearcad.set_visible({ kind = "sketch", index = 1 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")

bearcad.select{ kind = "body", index = 0 }
-- The OS cursor parks mid-viewport; the Dimension tool has no pick hover, keeping the
-- capture deterministic.
bearcad.ui.tool("dimension")

bearcad.ui.wait_ms(1500)
bearcad.ui.camera{ yaw = 0.7, pitch = 0.45, distance = 330, target = { 40, 40, 40 } }
bearcad.ui.wait(10)
bearcad.ui.screenshot(out)
bearcad.quit()
