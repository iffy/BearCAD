-- Documentation screenshot: the Circle tool.
--
-- A plate profile with two circles — a bolt hole with a diameter dimension and a
-- construction bolt-circle guide — seen from the top in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/circle.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.rect{ x = 0, y = 0, width = 50, height = 50, name = "Plate" }
-- A construction bolt circle and one bolt hole on it, with its diameter dimensioned.
bearcad.circle{ x = 25, y = 25, r = 16, name = "Bolt circle" }
bearcad.set_construction(bearcad.element("circle", 0), true)
bearcad.circle{ x = 41, y = 25, r = 4, name = "Hole" }

bearcad.clear_selection()
bearcad.ui.tool("dimension")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
