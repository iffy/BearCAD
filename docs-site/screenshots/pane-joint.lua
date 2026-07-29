-- Documentation screenshot: the Joint tool's Context pane with help mode on.
--
-- Help mode is the documentation for these controls: it draws a note beside each row
-- saying what that row wants, and a pane capture widens to include them. The tool is
-- armed mid-pick (a slider with its A pair mated) so the pickers show real content.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-joint.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)

bearcad.rect{ width = 30, height = 20, name = "Base" }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5, name = "Base" }
bearcad.rect{ x = 40, y = 0, width = 25, height = 8, name = "Arm" }
bearcad.extrude{ polygon = {4, 5, 6, 7}, distance = 5, name = "Arm" }
bearcad.exit_sketch()

bearcad.begin_joint{
  a = 0, b = 1, kind = "slider",
  from = { body = 1, vertex = {40, 0, 0} },
  to   = { body = 0, vertex = {30, 0, 0} },
  slide_max = 20,
}
bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "context")

bearcad.quit()
