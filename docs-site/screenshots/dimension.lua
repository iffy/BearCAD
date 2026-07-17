-- Documentation screenshot: the Dimension tool.
--
-- A profile with two length dimensions and an angle dimension between its tilted lines,
-- seen from the top in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/dimension.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }   -- 0 base
bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 22 }  -- 1 tilted leg
bearcad.select{ kind = "line", index = 0, ["end"] = "start" }
bearcad.select({ kind = "line", index = 1, ["end"] = "start" }, true)
bearcad.add_geometric_constraint("coincident")
bearcad.clear_selection()
bearcad.add_constraint({ kind = "line", index = 0 }, "40mm")
bearcad.add_constraint({ kind = "line", index = 1 }, "37mm")
bearcad.add_angle_constraint{ a = 0, b = 1, value = "36deg", sign = 1 }

bearcad.clear_selection()
bearcad.ui.tool("dimension")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out)

bearcad.quit()
