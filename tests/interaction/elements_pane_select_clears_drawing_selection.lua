-- #1906: selecting in the Elements pane replaces drawing-view selection that isn't
-- the selected element. Two bodies selected in the pane must not leave the page
-- projection selected.
bearcad.new()
bearcad.ui.tool("select")
bearcad.cuboid{ width = 30, depth = 20, height = 20 }
bearcad.cuboid{ width = 20, depth = 20, height = 20, at = {40, 0, 0} }
local d = bearcad.drawing{}
bearcad.drawing_view{ drawing = d, bodies = {0, 1}, orientation = "front" }
bearcad.ui.pane("parameters", "hide")
bearcad.ui.auto_zoom(false)
bearcad.ui.elements_view("tree")
bearcad.ui.wait(8)

local function kinds()
  local t = {}
  for _, e in ipairs(bearcad.selection()) do
    t[#t + 1] = e.kind
  end
  return table.concat(t, ",")
end

local function has_kind(kind)
  for _, e in ipairs(bearcad.selection()) do
    if e.kind == kind then return true end
  end
  return false
end

local function row(...)
  for i = 1, select("#", ...) do
    local label = select(i, ...)
    local r = bearcad.ui.elements_row_rect(label)
    if r then return r end
  end
  error("no Elements row labelled " .. table.concat({...}, " / "))
end

-- Select the projection on the page first (the state the report started from).
local card = bearcad.ui.drawing_view_rect(0)
assert(card, "the projection is on the page")
bearcad.ui.click(card)
bearcad.ui.wait(5)
assert(has_kind("projection"),
  "clicking the card should select the projection, got " .. kinds())

-- A plain click on a body row should replace that, not accumulate with it.
bearcad.ui.click(row("Body 0", "Cuboid 0"))
bearcad.ui.wait(5)
assert(not has_kind("projection"),
  "selecting a body in Elements should drop the projection, got " .. kinds())
assert(#bearcad.selection() >= 1, "the body should be selected")

-- Shift-clicking both bodies after the projection is selected again: still drop it.
bearcad.ui.click(card)
bearcad.ui.wait(5)
assert(has_kind("projection"), "the projection is selected again")
bearcad.ui.click(row("Body 0", "Cuboid 0"), { shift = true })
bearcad.ui.click(row("Body 1", "Cuboid 1"), { shift = true })
bearcad.ui.wait(5)
assert(not has_kind("projection"),
  "shift-selecting bodies in Elements should drop the projection, got " .. kinds())
assert(#bearcad.selection() >= 2, "both bodies should stay selected, got " .. kinds())

-- Selecting the projection row in Elements drops the bodies: it isn't those things.
local proj = row("Body 0 + Body 1 — Front", "Cuboid 0 + Cuboid 1 — Front")
bearcad.ui.click(proj)
bearcad.ui.wait(5)
assert(has_kind("projection"),
  "clicking the projection row should select it, got " .. kinds())
local n = #bearcad.selection()
assert(n == 1,
  "and only it — cuboids should drop, got " .. n .. " (" .. kinds() .. ")")

print("ok: Elements-pane selection replaces unrelated drawing-view selection")
bearcad.quit()
