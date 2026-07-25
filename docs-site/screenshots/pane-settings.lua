-- Documentation screenshot: the Settings window with help mode on (#720/#737).
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".".

local out = (os.getenv("BEARCAD_SCREENSHOT_OUT") or ".") .. "/pane-settings.png"

bearcad.new()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.help(true)
bearcad.ui.settings("show")

bearcad.ui.wait(6)
bearcad.ui.screenshot(out, "settings")
bearcad.quit()
