-- Regression (#1845): a cutting plane's *cut depth* bounds how far it reaches. Left
-- empty it cuts all the way through, the way a lone plane always has. Given a length it
-- pairs with a second plane that far behind it, facing the other way, so only the slab
-- between the two is hidden — a chunk out of the middle of the model.
bearcad.new()
bearcad.ui.tool("select")
bearcad.rect{ width = 60, height = 60 }
bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
bearcad.exit_sketch()

local whole = bearcad.body_stats(0)
assert(math.abs(whole.volume - 72000) < 1, "the block is 60x60x20, got " .. whole.volume)

bearcad.cross_section{ name = "Slot" }
-- The plane sits 4mm below the top face, normal up: it keeps what is above it.
bearcad.section_plane{ origin = {0, 0, 16}, normal = {0, 0, 1} }
local cuts = bearcad.section_planes()
assert(#cuts == 1, "one cutting plane, got " .. #cuts)
assert(cuts[1].depth == false, "a new plane cuts all the way through")

local through = bearcad.section_stats(0)
assert(
  math.abs(through.volume - 14400) < 1,
  "an unbounded cut takes everything below z=16, leaving 60x60x4, got " .. through.volume
)

-- Bound the cut to 6mm: the plane hides 10 < z < 16 and the block comes back below that.
bearcad.edit_section_plane{ cut = 0, depth = 6 }
cuts = bearcad.section_planes()
assert(math.abs(cuts[1].depth - 6) < 1e-4, "depth reads back, got " .. tostring(cuts[1].depth))

local slab = bearcad.section_stats(0)
assert(
  math.abs(slab.volume - (72000 - 60 * 60 * 6)) < 1,
  "only the 60x60x6 slab is hidden, got " .. slab.volume
)
assert(math.abs(slab.bbox.min.z) < 1e-3, "material survives below the slab")
assert(math.abs(slab.bbox.max.z - 20) < 1e-3, "and above it")

-- `false` puts the depth back to all the way through.
bearcad.edit_section_plane{ cut = 0, depth = false }
assert(bearcad.section_planes()[1].depth == false, "depth cleared")
assert(math.abs(bearcad.section_stats(0).volume - 14400) < 1, "and the cut runs through again")

-- The depth is a plane property like any other: it round-trips through a save/open.
bearcad.edit_section_plane{ cut = 0, depth = 6 }
local saved = os.tmpname() .. ".bearcad"
bearcad.save(saved)
bearcad.ui.wait(4)
bearcad.open(saved)
bearcad.ui.wait(8)
os.remove(saved)
assert(math.abs(bearcad.section_planes(0)[1].depth - 6) < 1e-4, "depth survives a round trip")

-- And the tool's own Cut depth field drives it: open the plane's edit draft, type into
-- the field, accept. Blank in that field is the through cut.
bearcad.edit_section_plane{ cut = 0, depth = false }
bearcad.ui.begin_edit_section_plane{ cut = 0 }
bearcad.ui.wait(6)
local row = assert(
  bearcad.ui.context_row_rect("Cut depth"),
  "the cutting plane tool has a Cut depth field"
)
local vp = bearcad.ui.viewport()
bearcad.ui.click(row.x + row.w - 24 - vp.x, row.y + row.h / 2 - vp.y)
bearcad.ui.wait(4)
bearcad.ui.type("8")
bearcad.ui.wait(4)
bearcad.ui.key("Enter")
bearcad.ui.wait(6)
assert(
  math.abs(bearcad.section_planes(0)[1].depth - 8) < 1e-4,
  "the typed cut depth committed, got " .. tostring(bearcad.section_planes(0)[1].depth)
)

print("ok: a cutting plane's cut depth takes a slab out of the middle")
bearcad.quit()
