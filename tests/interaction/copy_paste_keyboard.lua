-- #1408: Copy and Paste answer the normal platform shortcut keys (⌘C/⌘V on macOS,
-- Ctrl+C/Ctrl+V elsewhere). A scripted Ctrl+C puts the selection on the clipboard,
-- Ctrl+V starts an interactive paste, and Enter commits it as a new independent body.
bearcad.new()
bearcad.rect{ width = 50, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
bearcad.exit_sketch()
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.camera{ target = {25, 15, 0}, distance = 220 }
bearcad.ui.wait(10)

-- Select the extruded body (declaratively: this test is about the keyboard path).
bearcad.select{ kind = "body", index = 0 }
bearcad.ui.wait(3)
local sel = bearcad.selection()
assert(#sel == 1 and sel[1].kind == "body",
  "expected a body selected, got " .. (#sel > 0 and sel[1].kind or "nothing"))

local before = bearcad.count("body")

-- Ctrl+C (the platform primary modifier reads back as `cmd` in scripts; #1408).
bearcad.ui.key("c", { cmd = true })
bearcad.ui.wait(3)
assert(bearcad.status():find("Copied"),
  "Ctrl+C should copy the selection, status: " .. bearcad.status())

-- Ctrl+V starts the interactive paste.
bearcad.ui.key("v", { cmd = true })
bearcad.ui.wait(3)
assert(bearcad.status():find("Enter to place"),
  "Ctrl+V should start paste placement, status: " .. bearcad.status())

-- Enter commits the paste at its current offset.
bearcad.ui.key("enter")
bearcad.ui.wait(8)

assert(bearcad.count("body") == before + 1,
  "expected " .. (before + 1) .. " bodies after paste, got " .. bearcad.count("body"))

print("ok: Ctrl/Cmd+C copies the selection and Ctrl/Cmd+V pastes a new body")
bearcad.quit()
