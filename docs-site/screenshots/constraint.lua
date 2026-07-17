-- Documentation screenshot: the Constraint tool.
--
-- A four-line profile squared up by constraints — parallel, perpendicular, horizontal —
-- with the Constraint tool active, seen from the top in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/constraint.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")

-- A sloppy open profile, then constraints square it up.
bearcad.line{ x = 0,  y = 0,  x1 = 40, y1 = 3 }   -- 0 bottom
bearcad.line{ x = 40, y = 3,  x1 = 38, y1 = 25 }  -- 1 right cap
bearcad.line{ x = 38, y = 25, x1 = 2,  y1 = 22 }  -- 2 top
for i = 0, 1 do
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = i + 1, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
end
bearcad.clear_selection()
bearcad.select{ kind = "line", index = 0 }
bearcad.add_geometric_constraint("horizontal")
bearcad.clear_selection()
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 2 }, true)
bearcad.add_geometric_constraint("parallel")
bearcad.clear_selection()
bearcad.select{ kind = "line", index = 1 }
bearcad.select({ kind = "line", index = 0 }, true)
bearcad.add_geometric_constraint("perpendicular")
bearcad.clear_selection()

-- Leave the two parallel lines selected so the pane shows which constraints apply.
bearcad.ui.tool("constraint")
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 2 }, true)
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out, true)

bearcad.quit()
