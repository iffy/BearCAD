-- #1198: Shift held *before* opening the Selection Exploder makes the first leaf pick
-- additive (as if Shift were still down) and dismisses the fan immediately. That is
-- different from opening the fan first and then Shift-clicking loupes, which keeps the
-- fan open for multi-select until you dismiss it.
bearcad.new()
bearcad.rect{ width = 40, height = 30 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 15 }
bearcad.exit_sketch()
-- Hide the side panes (CI's WM-less Xvfb can't maximize; see tests/interaction).
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.ground("off")
bearcad.ui.tool("select")
bearcad.ui.view("top")
bearcad.ui.wait(5)
bearcad.ui.zoom_fit()
bearcad.ui.wait(5)

local function has_kind(sel, kind)
  for _, e in ipairs(sel) do
    if e.kind == kind then return true end
  end
  return false
end

local function body_loupe()
  for _, leaf in ipairs(bearcad.ui.exploder()) do
    if leaf.kind == "body" and leaf.x then
      return leaf
    end
  end
  return nil
end

-- ── Shift held when the fan opens: one additive pick, then dismiss ──────────
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(3)
assert(has_kind(bearcad.selection(), "construction_plane"),
  "precondition: a plane is selected before the fan")

bearcad.ui.move_ground(40, 30)
bearcad.ui.wait(3)
-- Shift+Space opens the fan while Shift is still down; then Shift is released.
bearcad.ui.key("space", { shift = true })
bearcad.ui.wait(6)
assert(#bearcad.ui.exploder() > 0, "Shift+Space should open the fan over the crowded corner")

local leaf = body_loupe()
assert(leaf, "the fan should offer the body as a leaf")
-- Plain click (no Shift) — must still *add* because the fan was opened with Shift.
bearcad.ui.click(leaf.x, leaf.y)
bearcad.ui.wait(8)

assert(#bearcad.ui.exploder() == 0,
  "a pick after Shift-opened fan should dismiss it immediately")
local sel = bearcad.selection()
assert(has_kind(sel, "construction_plane"),
  "the plane selected before the fan must still be selected (additive)")
assert(has_kind(sel, "body"),
  "the body picked through the fan must be added to the selection")
assert(#sel == 2, "expected plane + body, got " .. #sel)

-- ── Open first, then Shift-click: fan stays up for multi-select ─────────────
bearcad.clear_selection()
bearcad.ui.wait(3)
bearcad.select{ kind = "construction_plane", index = 0 }
bearcad.ui.wait(3)

bearcad.ui.move_ground(40, 30)
bearcad.ui.wait(3)
bearcad.ui.key("space")
bearcad.ui.wait(6)
assert(#bearcad.ui.exploder() > 0, "Space should open the fan")

leaf = body_loupe()
assert(leaf, "the fan should offer the body again")
-- Shift held only on the click — multi-select mode keeps the fan open.
bearcad.ui.click(leaf.x, leaf.y, { shift = true })
bearcad.ui.wait(8)

assert(#bearcad.ui.exploder() > 0,
  "Shift-clicking a loupe after a plain open should keep the fan up")
sel = bearcad.selection()
assert(has_kind(sel, "construction_plane"),
  "plane should remain selected after Shift-click through the fan")
assert(has_kind(sel, "body"),
  "body should be added by the Shift-click")

print("ok: Shift-before-open is one-shot additive; Shift-while-clicking keeps the fan open")
bearcad.quit()
