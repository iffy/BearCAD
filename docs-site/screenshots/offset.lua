-- Documentation screenshot: the Offset tool.
--
-- A rectangle offset outward and a circle offset inward as construction.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/offset.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 40, height = 20 }
bearcad.circle{ x = 34, y = 0, r = 6 }
bearcad.offset_sketch{ sketch = 0, lines = {0, 1, 2, 3}, distance = 4 }
bearcad.offset_sketch{ sketch = 0, circles = {0}, distance = -2, construction = true }

bearcad.ui.tool("offset")
-- A clean background (#667): the ground plane's quad and the grid both away. The
-- sketch drawn on that plane stays visible.
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here (#1049 pattern, from joint-kinds.lua).
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
