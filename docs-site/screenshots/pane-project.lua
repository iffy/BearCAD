-- Documentation screenshot: the Project tool's Context pane with help mode on (#672/#718).
--
-- A body plus a sketch on an offset plane, the tool active: the selection picker.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-project.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ x = 0, y = 0, width = 14, height = 10 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 8 }
bearcad.exit_sketch()
bearcad.plane{ offset = 15 }
bearcad.begin_sketch("construction_plane", 1)

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("project")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
