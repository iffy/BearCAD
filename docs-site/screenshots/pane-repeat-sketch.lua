-- Documentation screenshot: the Repeat tool's Context pane in sketch mode, help mode on
-- (#672/#835).
--
-- Two sketch entities picked and a direction line set, so every row shows: the entities
-- picker, the direction picker, and the count/gap/distance trio with its locks.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-repeat-sketch.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.circle{ x = 0, y = 0, r = 4 }
-- A line along +X, used as the repeat direction.
bearcad.line{ x = 0, y = -10, x1 = 40, y1 = -10 }

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("repeat")
bearcad.ui.wait(2)
-- Click the circle into the entity set, then Shift+click the line as the direction.
bearcad.ui.click_ground(0, 4)
bearcad.ui.wait(4)
bearcad.ui.click_ground(20, -10, { shift = true })
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
