-- #1577: the first tutorial step parks by the view-cube HUD (Bear's mouth),
-- not in the middle of the viewport.
bearcad.ui.tool("select")
bearcad.ui.tutorial("combine")
bearcad.ui.wait(8)

assert(bearcad.ui.tutorial_step() == 0, "combine starts on the intro")
assert(bearcad.ui.tutorial_orb() == nil, "intro has no click ring")

local b = bearcad.ui.tutorial_bubble()
assert(type(b) == "table", "intro bubble should be drawn")

local elements = bearcad.ui.pane_rect("elements")
local context = bearcad.ui.pane_rect("context")
assert(type(elements) == "table" and type(context) == "table",
  "side panes are shown")

local cx = b.x + b.w / 2
local viewport_mid = (elements.x + elements.w + context.x) / 2
assert(cx > viewport_mid,
  string.format(
    "first step should sit by the HUD (right of the viewport), cx=%.0f mid=%.0f",
    cx, viewport_mid))
assert(b.y < 200,
  string.format("first step should be up by the bear, not mid-viewport: y=%.0f", b.y))
assert(b.x + b.w <= context.x + 8,
  string.format(
    "bubble should not overlap Context: right=%.0f context.x=%.0f",
    b.x + b.w, context.x))

print("ok: first combine step sits by the bear HUD")
bearcad.quit()
