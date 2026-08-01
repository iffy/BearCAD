-- Documentation screenshots: the Combine tool, before and after a Cut.
--
-- combine-before.png: two overlapping boxes — the block and the bite that will be
-- carved out of it. combine.png: the notched result after the Cut; the inputs live
-- on as shadow bodies in the Elements pane.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNGs are only written where a real GPU frame renders.

local dir = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 30, height = 20, name = "Block" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 12, name = "Block" }

bearcad.begin_sketch{ kind = "plane", index = 0 }
bearcad.rect{ x = 18, y = 6, width = 24, height = 8, name = "Bite" }
bearcad.exit_sketch()
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 20, name = "Bite" }

-- Hide the three datum planes a new document opens with.
for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
-- Hide the ground grid too for a clean background (#579).
bearcad.ui.ground("off")
bearcad.ui.tool("dimension")
bearcad.ui.view("corner", "front_left_top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
-- Before: the block and the overlapping cutting body.
bearcad.ui.screenshot(dir .. "/combine-before.png")
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here.
bearcad.save(dir .. "/combine-before.bearcad.json")

bearcad.combine{ op = "cut", a = {0}, b = {1}, name = "Notched block" }
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
-- After: the notched result.
bearcad.ui.screenshot(dir .. "/combine.png")
bearcad.save(dir .. "/combine.bearcad.json")

bearcad.quit()
