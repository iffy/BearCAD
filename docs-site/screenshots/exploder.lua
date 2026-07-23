-- Documentation screenshot: the Selection Exploder (#551) fanned open over a crowd.
--
-- Several lines share one endpoint at the origin and a circle is centred there too,
-- so a whole crowd of pickable things (a vertex, several lines, a circle) stacks
-- inside the cursor's pick radius. Parking the cursor there and pressing Space fans
-- them out to their own spaced handles, each tagged with a little kind icon.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/exploder.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

-- A crowd at the origin: spokes sharing one vertex, plus a circle whose rim passes
-- through it (its centre — and its Ø label — sit below, clear of the cursor, so the
-- circle still joins the crowd without a dimension label under the pointer).
bearcad.line{ x = 0, y = 0, x1 = 42, y1 = 4 }
bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 30 }
bearcad.line{ x = 0, y = 0, x1 = -34, y1 = 18 }
bearcad.line{ x = 0, y = 0, x1 = -20, y1 = -30 }
bearcad.circle{ x = 0, y = -13, r = 13 }

bearcad.clear_selection()
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

-- Park the cursor on the stacked corner and explode it.
bearcad.ui.move_ground(0, 0)
bearcad.ui.wait(2)
bearcad.ui.key("space")
bearcad.ui.wait(3)
bearcad.ui.screenshot(out)

bearcad.quit()
