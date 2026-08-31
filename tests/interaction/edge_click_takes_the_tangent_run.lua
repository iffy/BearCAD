-- Interaction regression (#984): clicking any edge of a tangent-continuous run selects the
-- whole run — a straight line that breaks into a tangent curve and exits again as a tangent
-- line is one thing to pick. Holding Control takes only the edge under the cursor.
--
--   (0,0) --0-- (40,0) ~~1~~ (70,30) --2-- (110,30)
--                                              |
--                                              3  (a 90 degree corner: not in the run)
--                                          (110,70)
bearcad.new()
bearcad.line{ x = 0, y = 0, x1 = 40, y1 = 0 }
-- Handles continue each neighbour's direction: horizontal out of (40,0), and along line 2's
-- +x back into (70,30). That is what makes the joints tangent rather than corners.
bearcad.line{ x = 40, y = 0, x1 = 70, y1 = 30, bezier = {{58, 0}, {58, 30}} }
bearcad.line{ x = 70, y = 30, x1 = 110, y1 = 30 }
bearcad.line{ x = 110, y = 30, x1 = 110, y1 = 70 }
bearcad.clear_selection()
bearcad.ui.tool("select")
-- Hide the side panes: CI's WM-less Xvfb can't maximize, and with all three open the 3D
-- viewport is too narrow for the ground-coordinate clicks below to land inside it.
bearcad.ui.pane("elements", "hide")
bearcad.ui.pane("context", "hide")
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.view("top")
bearcad.ui.wait(5)
-- A fixed camera rather than zoom_fit, so the ground clicks land in the same place whatever
-- window size CI ends up with.
bearcad.ui.camera{ target = {55, 35, 0}, distance = 260 }
bearcad.ui.wait(5)

local function selected_lines()
  local out, n = {}, 0
  for _, e in ipairs(bearcad.selection()) do
    if e.kind == "line" then
      out[e.index] = true
      n = n + 1
    end
  end
  return out, n
end

-- Click the middle of the first straight line. The run reaches through the curve and out
-- the far side, so all three come in — and the corner line does not.
bearcad.ui.click_ground(20, 0)
bearcad.ui.wait(10)
local sel, n = selected_lines()
assert(sel[0] and sel[1] and sel[2],
  "clicking one segment should select the whole tangent run, got " .. n .. " line(s)")
assert(not sel[3], "the 90 degree corner must break the run")

-- Hovering says the same thing a click does: the run's other members light up alongside the
-- segment under the cursor.
bearcad.clear_selection()
bearcad.ui.wait(5)
bearcad.ui.move_ground(90, 30)
bearcad.ui.wait(5)
local h = bearcad.ui.hovered()
assert(h and h.kind == "line",
  "hovering a run member should highlight a line, got " .. tostring(h and h.kind))

-- Control narrows the pick to the one edge under the cursor.
bearcad.ui.click_ground(20, 0, { ctrl = true })
bearcad.ui.wait(10)
sel, n = selected_lines()
assert(sel[0] and n == 1,
  "Control+click should take only the edge under the cursor, got " .. n .. " line(s)")

-- Clicking a different member of the same run, with no modifier, takes the whole run again —
-- a plain click replaces the selection with the run.
bearcad.ui.click_ground(90, 30)
bearcad.ui.wait(10)
sel, n = selected_lines()
assert(sel[0] and sel[1] and sel[2] and n == 3,
  "a plain click on any member should replace the selection with the whole run, got " .. n)

-- The line past the corner is its own run of one.
bearcad.ui.click_ground(110, 50)
bearcad.ui.wait(10)
sel, n = selected_lines()
assert(sel[3] and n == 1, "the line past the corner is a run of one, got " .. n .. " line(s)")

-- Shift adds, and composes with Control: Shift+Ctrl adds the one edge under the cursor to
-- what is already selected, rather than its whole run.
bearcad.ui.click_ground(20, 0, { shift = true, ctrl = true })
bearcad.ui.wait(10)
sel, n = selected_lines()
assert(sel[3] and sel[0] and n == 2,
  "Shift+Ctrl+click should add just the one edge, got " .. n .. " line(s)")

-- Shift alone adds the whole run to what is already selected.
bearcad.clear_selection()
bearcad.ui.wait(5)
bearcad.ui.click_ground(110, 50)
bearcad.ui.wait(10)
bearcad.ui.click_ground(20, 0, { shift = true })
bearcad.ui.wait(10)
sel, n = selected_lines()
assert(sel[3] and sel[0] and sel[1] and sel[2] and n == 4,
  "Shift+click should add the whole run to the selection, got " .. n .. " line(s)")

print("ok: an edge click takes its whole tangent run, and Control takes just the edge")
bearcad.quit()
