-- Documentation screenshot: aligned views (#296/#361).
--
-- An L-bracket with a Top view (the parent) and two aligned children — a Right
-- view lined up along their shared edge, and a Front view directly below — so
-- the orthographic alignment reads at a glance. Captures the WHOLE WINDOW.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/aligned-views.png"
bearcad.new()
-- An L-profile so the top/bottom view reads as a clearly different shape from the front.
bearcad.line{ x = 0,  y = 0,  x1 = 40, y1 = 0 }
bearcad.line{ x = 40, y = 0,  x1 = 40, y1 = 12 }
bearcad.line{ x = 40, y = 12, x1 = 12, y1 = 12 }
bearcad.line{ x = 12, y = 12, x1 = 12, y1 = 30 }
bearcad.line{ x = 12, y = 30, x1 = 0,  y1 = 30 }
bearcad.line{ x = 0,  y = 30, x1 = 0,  y1 = 0 }
for i = 0, 5 do
  local j = (i + 1) % 6
  bearcad.select{ kind = "line", index = i, ["end"] = "end" }
  bearcad.select({ kind = "line", index = j, ["end"] = "start" }, true)
  bearcad.add_geometric_constraint("coincident")
  bearcad.clear_selection()
end
bearcad.exit_sketch()
bearcad.extrude{ polygon = { 0, 1, 2, 3, 4, 5 }, distance = 20, name = "Bracket" }

local d = bearcad.drawing{ name = "Bracket" }
bearcad.drawing_view{ drawing = d, body = 0, orientation = "top" }   -- view 0: shows the L
bearcad.drawing_move_view{ drawing = d, view = 0, x = 0.32, y = 0.38 }
bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "right" } -- view 1
bearcad.drawing_move_view{ drawing = d, view = 1, x = 0.66, y = 0.38 }
bearcad.drawing_align_view{ drawing = d, parent = 0, dir = "below" } -- view 2
bearcad.drawing_move_view{ drawing = d, view = 2, x = 0.32, y = 0.74 }

bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(3)
bearcad.ui.screenshot(out, true)
bearcad.quit()
