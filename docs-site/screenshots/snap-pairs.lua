-- Documentation screenshot: what each Snap point pair decides.
--
-- The same slab landing on the same plate three times, from the same camera, so the shots
-- read as one series: the A pair alone slides it, B turns it, C spins it upright. The last
-- degree of freedom is the point — with A and B the slab is still free to roll about the
-- end A → end B line, and only C settles it.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Three PNGs, one per pair, named `<this script>-<pair>` as the
-- harness expects of a scene that takes several shots.

local out = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

-- The slab is 24 long, 10 wide, 4 thick, parked clear of the plate. Its near-bottom corner
-- is start A, the far one along its length start B, and the one straight above start A is
-- start C.
local START_A = { 80, 0, 0 }
local START_B = { 104, 0, 0 }
local START_C = { 80, 0, 4 }

-- Where they land, all on the plate's top face (6 up).
local TOP = 6
local END_A = { 12, 8, TOP }
-- End B lies 24 from end A — the distance start B has to reach — at 40° across the plate.
local BEARING = math.rad(40)
local END_B = { END_A[1] + 24 * math.cos(BEARING), END_A[2] + 24 * math.sin(BEARING), TOP }
-- End C only says which way round the slab sits about that line. Square to it, on the side
-- that stands the slab up rather than sinking it into the plate.
local END_C = { END_A[1] + 4 * math.sin(BEARING), END_A[2] - 4 * math.cos(BEARING), TOP }

local function scene()
  bearcad.new()
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  bearcad.rect{ x = 0, y = 0, width = 60, height = 40, name = "Plate" }
  bearcad.exit_sketch()
  bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = TOP, name = "Plate" }

  bearcad.rect{ x = 80, y = 0, width = 24, height = 10, name = "Slab" }
  bearcad.exit_sketch()
  bearcad.extrude{ polygon = { 4, 5, 6, 7 }, distance = 4, name = "Slab" }
end

-- Same framing for all three, so only the slab's pose changes between them.
local function shoot(name)
  for i = 0, 2 do bearcad.set_visible({ kind = "construction_plane", index = i }, "hide") end
  bearcad.ui.ground("off")
  bearcad.ui.auto_zoom(false)
  bearcad.ui.view("corner", "front_left_top")
  bearcad.ui.wait(2)
  bearcad.ui.camera{ target = { 26, 14, 4 }, distance = 150 }
  bearcad.ui.wait(2)
  bearcad.ui.screenshot(out .. "/" .. name .. ".png")
end

-- A alone: the slab slides until start A sits on end A, facing exactly as it did.
scene()
bearcad.move_bodies{
  bodies = { 1 },
  from = { body = 1, vertex = START_A },
  to   = { body = 0, on_edge = END_A },
  name = "Landed",
}
shoot("snap-pairs-a")

-- A and B: it also turns about end A until start B points at end B.
scene()
bearcad.move_bodies{
  bodies = { 1 },
  from   = { body = 1, vertex = START_A },
  to     = { body = 0, on_edge = END_A },
  from_b = { body = 1, vertex = START_B },
  to_b   = { body = 0, on_edge = END_B },
  name = "Landed",
}
shoot("snap-pairs-ab")

-- A, B and C: it also spins about the end A → end B line until start C points at end C,
-- which stands the slab on its long edge. Nothing is left to choose.
scene()
bearcad.move_bodies{
  bodies = { 1 },
  from   = { body = 1, vertex = START_A },
  to     = { body = 0, on_edge = END_A },
  from_b = { body = 1, vertex = START_B },
  to_b   = { body = 0, on_edge = END_B },
  from_c = { body = 1, vertex = START_C },
  to_c   = { body = 0, on_edge = END_C },
  name = "Landed",
}
shoot("snap-pairs-abc")

bearcad.quit()
