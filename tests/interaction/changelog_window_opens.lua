-- Interaction regression (#1328): Help → Changelog is scriptable and shows this
-- build's embedded changelog.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")

bearcad.ui.changelog("show")
assert(bearcad.status():find("Changelog"),
  "opening the changelog should report itself, got: " .. bearcad.status())

local text = bearcad.ui.changelog("text")
assert(type(text) == "string" and #text > 0,
  "changelog text should be the markdown baked into this build")
assert(text:find("# v"), "embedded changelog should have a version heading, got: " .. text)

bearcad.ui.changelog("hide")
assert(bearcad.status():find("closed"),
  "closing should report itself, got: " .. bearcad.status())

print("ok: the Changelog window opens, exposes its text, and closes")
bearcad.quit()
