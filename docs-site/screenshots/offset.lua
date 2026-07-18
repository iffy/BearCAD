-- Documentation screenshot: the Offset tool.
--
-- A rectangle offset outward and a circle offset inward as construction.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/offset.png"

bearcad.new()
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 40, height = 20 }
bearcad.circle{ x = 34, y = 0, r = 6 }
bearcad.offset_sketch{ sketch = 0, lines = {0, 1, 2, 3}, distance = 4 }
bearcad.offset_sketch{ sketch = 0, circles = {0}, distance = -2, construction = true }

bearcad.ui.tool("offset")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
