-- Interaction regression (#1022): the first command palette command that takes an argument.
-- Choosing "Search McMaster-Carr" doesn't run it — the palette turns into a prompt for the
-- search, in its own pane, and the next Enter runs it with what was typed. Escape backs out
-- to the command list rather than closing the palette.
bearcad.new()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.wait(5)

-- Open the palette and pick the command by typing enough of it to be the top match.
bearcad.ui.palette{ open = true }
bearcad.ui.wait(5)
bearcad.ui.type("mcmaster")
bearcad.ui.wait(5)
bearcad.ui.key("Enter")
bearcad.ui.wait(5)
-- Enter chose the command but must not have run it: nothing opened, and the palette is
-- still up, now asking.
assert(not bearcad.status():find("catalog opened"),
  "choosing the command should ask, not run: " .. bearcad.status())

-- Escape goes back to the command list, leaving the palette open with what you had typed
-- still there — one keystroke to undo a wrong turn, rather than starting over. Pressing
-- Enter again therefore lands on the same command, which is the proof it went *back* rather
-- than merely somewhere.
bearcad.ui.key("Escape")
bearcad.ui.wait(5)
assert(not bearcad.status():find("catalog opened"), "backing out runs nothing")

bearcad.ui.key("Enter")
bearcad.ui.wait(5)
bearcad.ui.type("socket head screw")
bearcad.ui.wait(5)
bearcad.ui.key("Enter")
bearcad.ui.wait(30)
assert(bearcad.status():find("catalog opened"),
  "answering the prompt should open the catalog, got: " .. bearcad.status())

bearcad.ui.mcmaster("hide")
bearcad.ui.wait(10)

print("ok: a palette command can ask for an argument before it runs")
bearcad.quit()
