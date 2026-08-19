-- Interaction regression (#1619): the AI pane reports its own vertical scroll, a real
-- wheel over the pane drives it, and the offset clamps at both ends.
--
-- Whether the content actually overflows depends on the window this runs in, so the
-- assertions are written against the pane's own numbers. The overflow case is pinned to a
-- fixed-size window by `ai::panel::scroll_tests` instead.
bearcad.new()
bearcad.ui.tool("select")
bearcad.ui.pane("ai", "show")
bearcad.ui.wait(3)

local s = bearcad.ui.pane_scroll("ai")
assert(s, "an open AI pane reports its scroll state")
assert(s.offset == 0, "a freshly opened pane starts at the top, got " .. s.offset)
assert(s.content > 0 and s.viewport > 0,
  "the pane reports content and viewport heights, got " .. s.content .. "/" .. s.viewport)

-- Every section open is the tallest the pane gets. The headers animate open, so let the
-- content settle before measuring how far there is to scroll.
bearcad.ui.ai_sections("open")
bearcad.ui.wait(60)
s = bearcad.ui.pane_scroll("ai")
local max = math.max(0, s.content - s.viewport)

-- A wheel over the pane takes it to the bottom, and no further.
bearcad.ui.scroll_pane("ai", 10000)
bearcad.ui.wait(10)
local bottom = bearcad.ui.pane_scroll("ai")
assert(math.abs(bottom.offset - max) < 1.0,
  "the wheel should reach the bottom (" .. max .. "), got " .. bottom.offset)

-- And back up: the top is as far as it goes, never a negative offset.
bearcad.ui.scroll_pane("ai", -10000)
bearcad.ui.wait(10)
local top = bearcad.ui.pane_scroll("ai")
assert(top.offset == 0, "scrolling back up returns to the top, got " .. top.offset)

-- Collapsing every section fits the content again: nothing left to scroll.
bearcad.ui.ai_sections("close")
bearcad.ui.wait(60)
local closed = bearcad.ui.pane_scroll("ai")
assert(closed.content < s.content,
  "collapsing the sections should shorten the content, got " .. closed.content)

-- A hidden pane has no scroll state.
bearcad.ui.pane("ai", "hide")
bearcad.ui.wait(3)
assert(bearcad.ui.pane_scroll("ai") == nil, "a hidden pane reports no scroll state")

print("ok: the AI pane scrolls vertically and clamps at both ends")
bearcad.quit()
