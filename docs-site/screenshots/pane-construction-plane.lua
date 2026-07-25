-- Documentation screenshot: the Construction Plane tool's Context pane with help mode
-- on (#672).
--
-- A body whose top face anchors the plane, so the offset input shows.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-construction-plane.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ x = 0, y = 0, width = 14, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8 }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("construction_plane")
bearcad.ui.wait(2)
-- Anchor on the body's top face.
bearcad.ui.click_ground(7, 5)
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
