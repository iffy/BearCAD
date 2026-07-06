-- Documentation screenshot: the Slice tool.
--
-- Cuts a block in two with a construction plane through its middle — the two fragments
-- separate slightly (moved apart) so the cut reads clearly; the original lives on as a
-- shadow body in the Elements pane.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/slice.png"

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 30, height = 20, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 16, name = "Block" }

-- A cutting plane parallel to the ground, halfway up the block.
bearcad.plane{ offset = 8 }
bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 1 }},
               name = "Halved" }

-- Nudge the two fragments apart so the cut is visible.
bearcad.move_bodies{ bodies = {1}, z = 6 }

bearcad.set_visible({ kind = "construction_plane", index = 0 }, "hide")
bearcad.set_visible({ kind = "construction_plane", index = 1 }, "hide")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
