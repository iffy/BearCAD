-- Documentation screenshot: the Constraint tool.
--
-- A four-line profile squared up by constraints — parallel, perpendicular, horizontal —
-- with the Constraint tool active, seen from the top in sketch mode.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". The PNG is only written where a real GPU frame renders.

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/constraint.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")

-- A sloppy open profile, then constraints square it up.
bearcad.line{ x = 0,  y = 0,  x1 = 40, y1 = 3 }   -- 0 bottom
bearcad.line{ x = 40, y = 3,  x1 = 38, y1 = 25 }  -- 1 right cap
bearcad.line{ x = 38, y = 25, x1 = 2,  y1 = 22 }  -- 2 top
for i = 0, 1 do
  bearcad.constrain("coincident",
    { kind = "line", index = i, endpoint = "end" },
    { kind = "line", index = i + 1, endpoint = "start" })
end
bearcad.constrain("horizontal", { kind = "line", index = 0 })
bearcad.constrain("parallel",
  { kind = "line", index = 0 }, { kind = "line", index = 2 })
bearcad.constrain("perpendicular",
  { kind = "line", index = 1 }, { kind = "line", index = 0 })

-- Leave the two parallel lines selected so the pane shows which constraints apply.
bearcad.ui.tool("constraint")
bearcad.select{ kind = "line", index = 0 }
bearcad.select({ kind = "line", index = 2 }, true)
-- A clean background (#667): the ground plane's quad and the grid both away. The
-- sketch drawn on that plane stays visible.
-- Hide the three datum planes a new document opens with.
bearcad.set_visible({ kind = "plane" }, false)
bearcad.ui.ground("off")
bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(1)
bearcad.ui.screenshot(out, true)
-- The document behind this picture, so the docs page can link the screenshot into
-- the web app with `?open=` pointing here.
bearcad.save((out:gsub("%.png$", ".bearcad.json")))

bearcad.quit()
