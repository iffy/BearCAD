-- Documentation screenshot: the Revolve tool's Context pane with help mode on (#672).
--
-- A profile standing off the axis, with the tool active so every row it offers is on
-- screen: the profile and axis pickers, the symmetric toggle, and the output choice. The
-- sweep angle isn't a pane field — it's dragged in the 3D view.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-revolve.png"

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ x = 12, y = 0, width = 10, height = 12, name = "Profile" }
bearcad.exit_sketch()

bearcad.ui.view("top")
bearcad.ui.wait(2)
bearcad.ui.zoom_fit()
bearcad.ui.wait(2)

bearcad.ui.tool("revolve")
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
