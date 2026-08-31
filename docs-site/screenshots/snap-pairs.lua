-- Documentation screenshot: what each Snap point pair decides.
--
-- The same slab landing on the same plate three times, from the same camera, so the shots
-- read as one series: the A pair alone slides it, B turns it, C spins it upright. The last
-- degree of freedom is the point — with A and B the slab is still free to roll about the
-- end A → end B line, and only C settles it.
--
-- Each is the tool's **live preview** (`bearcad.ui.begin_move`, not `move_bodies`): the slab
-- still sits where it started, the ghost shows where it's going, and the picked pairs are
-- marked and joined — start A green to end A red, the B and C pairs in blue with the dashed
-- path each point travels. That's the thing being explained, so it's what the shot shows.
--
-- Output dir: $BEARCAD_SCREENSHOT_OUT (set by scripts/gen-doc-screenshots.sh),
-- falling back to ".". Three PNGs, one per pair, named `<this script>-<pair>` as the
-- harness expects of a scene that takes several shots.

local out = os.getenv("BEARCAD_SCREENSHOT_OUT") or "."

-- Hide the viewport usage overlay so it doesn't cover the model (#1509).
bearcad.ui.tool_hints(false)

-- The slab is 22 long, 9 wide, 5 thick, parked just off the plate so both it and where
-- it's going fit the same frame — and above it, on a construction plane, so the line from
-- start A to end A runs through open air instead of being buried in the plate it's heading
-- for. Its near-bottom corner
-- is start A, the far one along its length start B, and the one straight above start A is
-- start C.
local PARK = 10
local START_A = { 46, 2, PARK }
local START_B = { 68, 2, PARK }
local START_C = { 46, 2, PARK + 5 }

-- Where they land, all on the plate's top face.
local TOP = 4
local END_A = { 9, 7, TOP }
-- End B lies 22 from end A — the distance start B has to reach — at 35° across the plate.
local BEARING = math.rad(35)
local END_B = { END_A[1] + 22 * math.cos(BEARING), END_A[2] + 22 * math.sin(BEARING), TOP }
-- End C only says which way round the slab sits about that line. Square to it, on the side
-- that stands the slab up rather than sinking it into the plate.
local END_C = { END_A[1] + 5 * math.sin(BEARING), END_A[2] - 5 * math.cos(BEARING), TOP }

local function scene()
  bearcad.new()
  bearcad.ui.pane("context", "hide")
  bearcad.ui.pane("parameters", "hide")

  bearcad.rect{ x = 0, y = 0, width = 40, height = 30, name = "Plate" }
  bearcad.exit_sketch()
  bearcad.extrude{ polygon = { 0, 1, 2, 3 }, distance = TOP, name = "Plate" }

  bearcad.plane{ offset = PARK, name = "Park" }
  bearcad.begin_sketch{ kind = "plane", index = 3 }
  bearcad.rect{ x = 46, y = 2, width = 22, height = 9, name = "Slab" }
  bearcad.exit_sketch()
  bearcad.extrude{ polygon = { 4, 5, 6, 7 }, distance = 5, name = "Slab" }
end

-- Same framing for all three, so only the ghost's pose changes between them: a clean stage
-- (no datum planes, no grid, no sketch profiles drawn over the solids) and a pinned camera
-- holding both the parked slab and where it's going. The tool colors the cast itself —
-- cyan ghost, dimmed source, green/red/blue marks — so nothing here needs a material.
local function shoot(name)
  bearcad.set_visible({ kind = "plane" }, false)
  bearcad.set_visible({ kind = "sketch" }, false)
  bearcad.ui.ground("off")
  bearcad.ui.auto_zoom(false)
  bearcad.ui.view("corner", "front_left_top")
  bearcad.ui.wait(2)
  bearcad.ui.camera{ target = { 27, 10, 6 }, distance = 115 }
  bearcad.ui.wait(3)
  bearcad.ui.screenshot(out .. "/" .. name .. ".png")
  -- The document behind each picture, for the docs page to link into the web app.
  bearcad.save(out .. "/" .. name .. ".bearcad.json")
end

-- A alone: the ghost sits where start A meets end A, facing exactly as the slab does.
scene()
bearcad.ui.begin_move{
  bodies = { 1 },
  from = { body = 1, vertex = START_A },
  to   = { body = 0, on_edge = END_A },
}
shoot("snap-pairs-a")

-- A and B: the ghost also turns about end A until start B points at end B.
scene()
bearcad.ui.begin_move{
  bodies = { 1 },
  from   = { body = 1, vertex = START_A },
  to     = { body = 0, on_edge = END_A },
  from_b = { body = 1, vertex = START_B },
  to_b   = { body = 0, on_edge = END_B },
}
shoot("snap-pairs-ab")

-- A, B and C: the ghost also spins about the end A → end B line until start C points at
-- end C, standing it on its long edge. Nothing is left to choose.
scene()
bearcad.ui.begin_move{
  bodies = { 1 },
  from   = { body = 1, vertex = START_A },
  to     = { body = 0, on_edge = END_A },
  from_b = { body = 1, vertex = START_B },
  to_b   = { body = 0, on_edge = END_B },
  from_c = { body = 1, vertex = START_C },
  to_c   = { body = 0, on_edge = END_C },
}
shoot("snap-pairs-abc")

bearcad.quit()
