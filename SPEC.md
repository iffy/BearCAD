# BearCAD — Specification

BearCAD is an on-device, parametric CAD program comparable to Autodesk Fusion, FreeCAD,
and OpenSCAD. This document is the implementation specification: it should contain
enough detail for an engineer to build BearCAD without further design decisions. Where a
section says **TBD**, that detail is deliberately deferred and must be resolved before
the relevant work begins.

---

## 1. Technology decisions (fixed)

These are settled. Do not re-litigate them during implementation.

| Concern | Decision | Notes |
|---|---|---|
| Implementation language | **Rust** | Produces a single self-contained executable; strong cross-platform GUI/3D ecosystem; good C/C++ FFI for the geometry kernel. |
| Geometry kernel | **OpenCASCADE (OCCT)** | B-rep solids, NURBS, booleans, fillets, and native STEP/IGES I/O. Used from Rust via FFI bindings (see §10). |
| Embedded scripting | **Lua** | Small, fast, sandboxable. No custom DSL. See §8. |
| GUI toolkit | **egui** | Immediate-mode; easy tiling/docking, command palette, theming. |
| 3D rendering | **wgpu** | Cross-platform GPU backend; the 3D viewport is a wgpu surface composited with egui. |
| Save file | **SQLite**, extension `.bearcad` | Schema in §7. |
| License | **MIT OR Apache-2.0** (dual) | BearCAD's own code is permissively licensed. OCCT is LGPL 2.1 and is **statically linked** under the LGPL's relink provision — BearCAD ships the pinned OCCT source (submodule), a build script, and an `OCCT_DIR` relink override (see §10). Bundle the LGPL + OCCT-exception text and all dependency notices via `THIRD_PARTY_LICENSES.md` (Help ▸ Licenses). Audit STEP/3MF/AMF library licenses for the same constraint. |

### 1.1 Platforms

Must build and run on **macOS, Linux, and Windows**, producing a single self-contained
executable per platform (kernel and other native libs may be dynamically linked but must
be bundled with the distributable). The executable launches the GUI by default and acts
as a CLI when given a subcommand (see §9).

**macOS packaging:** the `.app` bundle inside the distributed `.dmg` must be code-signed.
Absent a paid Apple Developer certificate, it must at minimum be **ad-hoc signed**
(`codesign --force --deep --sign -`) so that a quarantined download is not rejected by
Gatekeeper as *"'BearCAD' is damaged and can't be opened"* (the message macOS shows for an
unsigned or signature-invalidated bundle on Apple Silicon). The signature must be applied to
the fully assembled bundle (after the executable, icons, and `Info.plist` are in place) and
verified with `codesign --verify --deep --strict`. The `.dmg` volume must also contain an
`Applications` symlink (→ `/Applications`) alongside the app so the user can drag
`BearCAD.app` straight into Applications from the mounted volume.

---

## 2. Core concepts and domain model

### 2.1 Document

A document is one `.bearcad` file. A document contains:

- One or more **components**.
- A set of document-level **parameters** (see §5).
- The full **action DAG** (see §4).
- **UI/view state** (pane layout, camera, theme, custom shortcuts).

### 2.2 Component

A **component** is an independent unit of geometry with its own coordinate system,
its own parameters, its own sketches and features, and its own subgraph within the
action DAG (see §4.2). A component may **reference** other components; such a reference
creates a dependency edge in the DAG, and the referenced component's geometry/parameters
become inputs to the referencing component.

**Implemented today (#423) — components as organizational groups:** `model::Component`
(`name`, `parent`, per-component `length_unit`/`angle_unit` overrides, tombstoned) plus
`Document::component_members` mapping top-level elements
(`model::ComponentMember`: planes, extrusions, bodies, lofts, boolean/move/repeat/slice
ops, revolutions, drawings) to a component; the document acts as the root component.
Grouping never changes geometry.

- **Active component (#429):** `AppState::active_component` (UI-only) is set when a
  component is created (the new component is also selected) or clicked; while set, the
  outermost `apply` files every newly created top-level element (planes, extrusions,
  lofts, ops, revolutions — bodies derive via their source) into it
  (`member_vec_lens`/`assign_new_members`, inside the same undo step). The active
  component's row shows a ● accent marker (the Document row carries it when none is
  active); clicking the Document row deactivates.
- **Elements pane:** the header **+** (icon button) opens an add menu with **New
  component**; component rows show a painted collapse **triangle**, an eye, the
  `component.svg` icon, and nest their contents one indent level
  (`hierarchy::component_list_rows`; nested assigned entries are extracted from wherever
  they sit into their component's entry, `group_roots_into_components`). Rows **drag**
  onto a component row (`ComponentDragPayload`; a floating name tag follows the cursor
  and drop targets are rect-based so releasing over a row's child widgets still lands,
  #430) or use right-click → **Move to**; the Document root row is the drop target for
  un-filing. Right-click a component: **New
  component inside**, **Move to document root**, **Delete** (deleting re-homes contents
  to the parent, `document_lifecycle::tombstone_element`).
- **Visibility:** hiding a component hides everything inside it — members resolve
  through `hierarchy::owning_component` (a body via its producing op/extrusion, an
  extrusion or image via its sketch's host plane) and every ancestor component must be
  visible (`ElementVisibility::effective_visible`).
- **Units:** each component may override length/angle units; contents inherit sketch
  override → component chain → document default (`effective_length_unit`,
  `effective_component_length_unit`). The context pane shows **Component units** pickers
  (with an **Inherit** entry) for a selected component.
- **Graph view:** components are not nodes but **areas** — smooth, lightly shaded convex
  hulls (`rounded_hull`) drawn beneath the member nodes, labeled at the top edge; nested
  components layer their tints.
- **Persistence:** `components`/`component_members` meta JSON in the SQLite format, serde
  fields in the JSON format.
- **Scripting:** `bearcad.component{ name =, parent = }` (returns the index),
  `bearcad.move_to_component{ kind =, index =, component = i|false }`,
  `bearcad.set_units{ component = i, … }`, `bearcad.select{ kind = "component" }`,
  `bearcad.count("component")`.

The full referenced-component/assembly model above remains future work.

### 2.3 Assembly

Components can be placed into an **assembly**: instances of components positioned in
space and related by **joints/mates** (e.g. rigid, revolute, slider, coincident-face).
Joints are themselves parametric and participate in the DAG. A document may contain
multiple assemblies. (Detailed joint catalog: **TBD**, but at minimum rigid and revolute
for v1.)

### 2.4 Feature

A **feature** is a single modeling operation that produces or modifies geometry — a
sketch, an extrude, a fillet, a boolean, etc. Features are the primary nodes of the
action DAG (§4). The current geometry of a component is the result of evaluating its
features in dependency order.

### 2.5 World coordinate system

- The world is **right-handed with Z up**. The **ground plane is XY** (z = 0) and is the
  default sketching plane when none is chosen. X and Y span the ground; Z is height.
- Internal canonical length unit is millimetres (§5.3); the ground plane and all geometry
  are expressed in this convention.
- **Adaptive ground grid & origin axes (#353):** the grid and the origin X/Y/Z axis triad scale
  with the camera so they stay a usable on-screen size for parts of any magnitude — the axes are
  kept at least ~90 px long and the grid spacing snaps to a "nice" 1/2/5×10ⁿ step
  (`gpu_viewport::nice_grid_step`) sized to the zoom, so a 20 ft part doesn't shrink them to a
  dot. Line count stays bounded (extent is a fixed multiple of the current step).

---

## 3. Geometry & modeling operations (v1 scope)

All geometry is B-rep via OCCT. The following operations are **in scope for v1**:

### 3.1 Sketching (2D)
- Sketches are created on a datum plane or a planar face.
- **Draw tools begin sketches:** with no sketch open, the Rectangle, Line, Circle, and Text
  (#383) tools hover-highlight sketchable faces and a click begins a sketch on the clicked
  face — the tool then draws there immediately, no separate Sketch-tool step needed.
- **Sketching on body faces:** the planar cap faces of an extruded body (the base and
  offset ends of each extruded profile) are selectable sketch faces. Clicking one with the
  Sketch tool starts a sketch on that face — its frame inherits the profile's in-plane axes,
  offset along the extrusion normal — and the geometry drawn there behaves exactly like any
  other sketch. Such a sketch (and anything built from it) nests under, and depends on, the
  extrusion whose face it sits on. A flat **side wall**'s frame runs u along its profile edge
  and v up the extrusion, with the normal pointing **out of the solid** — derived from the
  profile loop's winding order (not a centroid heuristic), so the frame stays right-handed as
  seen from outside even on the concave inner walls of a non-convex (e.g. L-shaped) profile,
  and text or geometry sketched there reads correctly rather than mirrored (#362). A solid cap
  occludes the datum plane behind it for picking.
  When several faces project onto the cursor (e.g. the near and far faces of a solid), face
  picking resolves to the one nearest the camera, so a hover/click never selects a face hidden
  behind the body. Entering a sketch reorients the camera head-on to the face and orients the
  plane's own axes to the screen: the u-axis points screen-right and the v-axis screen-up, so a
  **Horizontal** constraint (which fixes a line along u) reads horizontal and a **Vertical**
  constraint (along v) reads vertical, regardless of the prior camera roll (#187). For a
  near-vertical face (such as a side wall) the view instead orients with world up (+Z) toward the
  top of the screen so the ground stays at the bottom and orbit behaves normally, rather than
  rolling sideways.
- **Constraining to the sketched-on face itself (#26/#27):** while a sketch is open on one of
  a body's own faces (an extrusion cap or side wall — not a construction plane), that face's
  own analytic boundary loop (the same one used for its cap/side-wall geometry) is available as
  constraint targets: `ConstraintPoint::FaceVertex` for a corner and `ConstraintLine::FaceEdge`
  for an edge, both resolved by projecting the face's world-space boundary into the sketch's
  frame. They plug into the existing constraint machinery like any other point/line — a sketch
  point can be **Coincident** to a face vertex, and the **Midpoint**/**PointLineDistance**
  constraints work against a face edge unchanged (e.g. "30mm from the top edge"). Both are
  fixed by the body's geometry (not draggable/settable), the same treatment `Coincident`'s
  `Origin` entity already gets. Picking is scoped to the *active sketch's own face* only (not
  arbitrary other faces), with vertices taking precedence over edges like other sketch points.
  Out of scope: imported STL/STEP bodies have no analytic face/edge structure to reference.
- **Projections (#140):** with a sketch open, selecting external 3D geometry (a body's
  edges via 3D selection, #156 — or a whole body/extrusion, which projects all of its
  feature edges) and pressing **Y** (or "Project Selection into Sketch" in the palette)
  projects it onto the sketch plane, along the plane normal. Discoverable via
  **`Tool::Project`**: a toolbar button that appears only in sketch mode — with it
  active, outside body edges/faces hover-glow and a click projects the edge (a face or
  vertex projects the whole body) through `Action::ProjectElement`; both entry points
  share `Action::ProjectSources`. Lua tool name `"project"`. Each projected edge becomes a
  construction-style line drawn dashed in its own **projection color** (teal, distinct from
  construction's color) and usable like construction geometry (snapping, constraints).
  Projections are **associative**: each geometry recompute re-resolves the source edge and
  rewrites the projected line, so it follows its source body. Sources are geometry-keyed
  (mesh edges have no stable topological name), so if a rebuild moves/removes the source
  edge the projection keeps its last resolved shape rather than dangling; projected lines
  are fixed (not draggable). Edges edge-on to the sketch plane (zero projected length) are
  skipped. Standalone vertex projection is not yet supported (a projected edge's endpoints
  already serve as snap targets).
- Sketch entities: line, arc, circle, ellipse, spline, point, and construction-geometry
  variants. Convenience primitives (e.g. **rectangle**, drawn as four constrained lines)
  may be offered as tools that emit the underlying entities.
- **Line tool chaining:** the line tool draws connected polylines — after a segment is
  committed, the next segment starts automatically at that endpoint (coincident with it), so
  a polygon is drawn with successive clicks. Chaining stops when the segment's end snaps onto
  an existing vertex (closing/joining the shape); **Esc** finishes the polyline, keeping the
  segments already drawn.
- Sketches are fully constraint-driven (see §6).
- **Constraint-state line color (#172):** solid sketch lines draw in blue while they still
  have freedom, and in **near-white once fully constrained** — using the same signal that
  blocks dragging (dimensioned, and the solver's DOF analysis finds no joint endpoint
  freedom), so "white = can't move" is consistent between styling and interaction. The set
  is memoized per document state (the DOF analysis builds a solver system per sketch).
  Construction (dashed grey) and projected (dashed teal, #140) styling take precedence.
- **Snapping:** while drawing or dragging sketch geometry, the cursor snaps to nearby
  vertices, line midpoints, lines, the sketch **origin**, and the sketch's two in-plane
  **origin axes** (the X axis `v = 0` and Y axis `u = 0`, #189) — vertices/origin take
  priority, then midpoints, then anywhere on a line or axis. Leaving a point on a snap adds
  the implied constraint (coincident for a vertex/origin/on-line/on-axis snap, midpoint for a
  midpoint snap), deduped against existing constraints. A point-on-axis snap is a point-on-line
  coincidence against the origin axis, pinning that coordinate to 0. A ring marks the active
  snap. Snapping is toggleable from the context pane and the toggle only appears for tools that
  snap (Select, Line, Rectangle, Circle) while a sketch is open. The origin (`SceneElement::Origin`,
  drawn as a small marker where the axes cross) and the origin axes (`ConstraintLine::OriginAxis`)
  are also directly viewport-selectable in the constraint tool — not just reachable by snapping —
  so a point can be constrained coincident with the origin, or onto an axis, by clicking them. A
  selected origin brightens to the selection colour and a selected axis highlights along its full
  length so the pick is visible.
- **Inference / extension snapping:** hovering a vertex while drawing arms its incident edges
  as extension guides; pulling away then snaps the point onto the **infinite extension** of
  those edges (within a perpendicular tolerance), with a dashed guide line from the edge to the
  point. Leaving the point there adds a point-on-line coincidence (collinear with the edge), so
  e.g. touching a rectangle corner lets the next point be placed in line with one of its sides.
- **Inference snapping onto a normal-at-midpoint guide (#41):** touching a line/edge's
  **midpoint** arms it as a normal-inference anchor; pulling away then snaps the point onto the
  **infinite line perpendicular to that edge, through its midpoint** (same touch-then-track
  interaction as the extension guide above, with its own dashed guide line). There's no single
  constraint primitive for "perpendicular through a midpoint", so leaving the point there instead
  invents a construction `Line` from the anchor's midpoint out toward the placed point (dashed,
  `construction: true`) and pins it with three existing constraints: `Midpoint` (its start at the
  anchor's midpoint), `Perpendicular` (to the anchor), and `Coincident` (the placed point onto the
  new line's carrier) — no new `ConstraintKind` needed.
- **Polygon faces from closed line loops (#66):** any set of plain `Line`s that connect
  end-to-end into a closed loop, via `Coincident` constraints on their endpoints, is itself a
  usable face — filled the same as a circle profile (shared blue styling, construction
  loops dashed/dimmed like other construction geometry), pickable for sketching-on-face, and
  extrudable. Loops are detected on the fly (not a stored entity) as every simple cycle in the
  sketch's line-connectivity graph; a line shared by two loops (e.g. a rectangle split by a
  diagonal) yields multiple selectable polygon faces. Scriptable via
  `bearcad.extrude{ polygon = {line_index, ...} }`, which takes an explicit ordered line list
  rather than relying on auto-detection.
- **Bezier curves (#54):** a curve is a `Line` with an optional pair of cubic tangent-handle
  control points (`[0]` near `(x0,y0)`, `[1]` near `(x1,y1)`) — its two endpoints stay ordinary
  constrainable vertices, so coincidence/distance constraints, dragging, undo, and persistence
  all work unchanged. Curves are made three ways:
  - **Curve-mode toggle with the Line tool (#73):** the Line tool always places points with
    plain click-click (no click-drag gesture). Two independent toggles, shown as checkboxes in
    the Context pane (above Construction) while the Line tool is active and bound to keyboard
    shortcuts Cmd/Ctrl+`B` and `T`, control what happens at each shared vertex of a drawn
    polyline:
    - **Curve mode (Cmd/Ctrl+`B` — a primary-modifier shortcut, #127, unlike the plain-letter
      toggles elsewhere, since a bare `B` collided with typing a length expression containing
      the letter b; default off):** when on, the *next* point placed gets bezier handles on
      both sides of it (or just the outgoing side, if it's a fresh chain's starting point, since
      there's no previous segment to derive a tangent from yet). Concretely: committing the
      *n*-th point of a chain (n ≥ 3) retroactively smooths the shared vertex between the
      (n-2)→(n-1) and (n-1)→n segments — so a segment only curves once a further point makes its
      tangent meaningful. The toggle persists across chained segments (like Construction) and is
      read/written by `Action::ApplyCurveMode`/`ToggleCurveMode`.
    - **Tangent constraint (`T`, default on):** while curve mode is on, controls *how* each
      shared vertex is curved. On: both sides' handles are mirrored/tangent-continuous via the
      same smoothing used by "Convert to bezier curve" below. Off: the previous segment's handle
      is left alone and the new segment gets an independent "corner" handle a third of the way
      along its own chord — a barely-curved starting shape meant to be reshaped by hand via the
      draggable handles below.
    - **Live preview:** as the mouse moves before the next point is placed, the in-progress
      segment previews its live curve toward the cursor, and — when curve mode smooths a shared
      vertex — the previous segment's end visibly bends to stay smooth/corner-consistent with it,
      updating every frame.
    - Both toggles also work retroactively: with the Select tool, in sketch mode, with one or
      more vertices selected, Cmd/Ctrl+`B` toggles the selected vertex(es) between curved and straight
      (straightens both incident lines if either is already curved, else smooths them — see
      `Action::SetVertexTangent`/`ConvertVertexToBezier`/`StraightenLine`) and `T` toggles
      between tangent-continuous (re-smoothed) and independent handles at the vertex. Vertices
      that don't join exactly two plain lines are skipped (no-op).
  - **Draggable handles:** once committed, a curved line's two tangent handles are shown (in the
    active sketch) as small discs with dashed guides back to their endpoint; dragging one
    reshapes the curve live. Clicking (rather than dragging) a handle selects it; pressing
    Delete/Backspace, or right-clicking it and choosing "Delete handle", straightens the line
    (#75) — a curve is either both handles or neither, so there's no independent per-handle
    state to remove, only the whole curve.
  - **Right-click a vertex:** right-clicking a vertex where exactly two plain lines meet offers
    "Convert to bezier curve", which smooths the joint into a tangent-continuous pair of curves
    (Catmull-Rom-style, using the two lines' far endpoints to set the tangent direction through
    the shared vertex). The reverse, "Straighten curve", is offered when right-clicking an
    existing curved line.
  - A curved line is faceted into `BEZIER_SEGMENTS` (24) straight sub-segments for rendering,
    hit-testing, and — when part of a closed polygon loop — extrusion tessellation (the same
    style of approximation already used for circular profiles). A side-wall face is addressed
    by its **profile-line index** (analytic, #178): each straight profile line has one flat
    side wall, sketchable and pickable by that line's position in the loop, regardless of how
    many facets any curved bridge between walls carries. A curved (bezier) profile edge sweeps
    a multi-faceted, non-flat wall, so — like a circular profile's curved wall — it isn't a
    flat sketch face. Inference/extension snapping onto a curved line
    still uses its straight chord (not the true curve) for the midpoint/on-line snap targets.
  - **Length semantics (#111):** a curved line's reported length is its true **arc length**
    (summed over the same `BEZIER_SEGMENTS` tessellation) everywhere it's displayed or
    introspected — Elements-pane labels, computed parameters, `bearcad.get{}.length` — but a
    length **dimension** on a curved line constrains the endpoint (**chord**) distance, since
    the sketch solver moves endpoints, not bezier handles. Exception: a *fillet-bridge arc*
    (a line with `chamfer_fillet_parent` and handles) has its handles re-fit after every
    solve to stay a circular arc tangent to its neighbours, so constraint-driven reshaping
    (e.g. a parameter-driven angle change) keeps the bend smooth instead of folding it.
  - Scriptable via `bearcad.line{ x=, y=, x1=, y1=, bezier = { {cx0, cy0}, {cx1, cy1} } }`.
- **Chamfer and fillet (#37/#38), 2D sketch vertices only:** both are tools ("push/pull" gizmo
  + text-entry input, mirroring the extrude tool) that operate on a sketch vertex where exactly
  two plain lines meet. Both truncate each line's endpoint back along itself and bridge the two
  new endpoints with a new `Line`: a **chamfer** truncates by the typed distance and bridges with
  a **straight** line; a **fillet** truncates by the tangent length implied by the requested
  radius and bridges with a line whose `bezier` field is set to a **single-cubic-bezier
  approximation of the circular arc** (accurate for realistic corner angles, not a true NURBS
  arc) — this reuses the bezier-curve machinery above (rendering, hit-testing, extrusion
  tessellation) for free, since a filleted corner is, to the rest of the app, just another curved
  `Line`. The tangent length is clamped so it never cuts back past either adjacent line's own far
  endpoint; a corner within ~1° of straight (0°/180°, i.e. parallel/anti-parallel edges) is
  rejected as degenerate. On commit, the `Coincident` constraint directly between the two
  treated endpoints is removed and the bridging line's two endpoints are tied to the trimmed
  lines with fresh `Coincident` constraints — so a treated polygon **stays a closed loop**
  (still a fillable, extrudable face; loop detection walks the constraint graph). The whole
  gesture (bridge line + its two constraints) is one undo group. Other constraints that
  happened to reference the old vertex position are **not** automatically fixed up (a known,
  documented limitation; the resulting sketch may need manual re-constraining). This is specifically the **2D sketch-vertex** case;
  the same Chamfer/Fillet tool also does a **3D solid-edge** mesh-bevel approximation on an
  extrusion's analytic side/cap edges when no sketch is open — see §3.4, which is *not* a true
  kernel-backed BREP fillet (BearCAD has no BREP/NURBS kernel — see §10). Scriptable via
  `bearcad.chamfer_vertex{ point = {...}, distance = }`
  and `bearcad.fillet_vertex{ point = {...}, radius = }`, where `point` is the usual
  `ConstraintPoint` table (e.g. `{ kind = "line", index = 0, ["end"] = "end" }`).
  - **Live geometry preview (#76):** while the gizmo is being placed or dragged (before commit),
    the actual treated-corner shape is drawn as a preview overlay — the two truncated points and
    the bridge between them (straight for a chamfer, sampled from the fillet's bezier arc) — not
    just the gizmo arrow. It's recomputed every frame from the live drag amount, so pulling the
    handle further visibly grows the cut/round before you commit.
  - **Elements pane nesting (#76):** the bridging `Line` a chamfer/fillet creates is nested under
    the trimmed line it came from, instead of appearing as an ordinary flat sibling. Since a
    corner is shared by two trimmed lines, the tie is broken deterministically by nesting under
    whichever of the two has the lower index in `doc.lines` (recorded once at commit time via
    `Line.chamfer_fillet_parent: Option<usize>`); if that parent line is later deleted, the
    bridging line falls back to a top-level row rather than disappearing. Its default label is
    also "Chamfer N"/"Fillet N" instead of the generic "Line N".
  - **Document root row (#87):** the Elements pane's sole top-level row is a synthetic
    **Document** node (not individually selectable or hideable); every root construction
    plane, orphaned extrusion, and orphaned body (e.g. STL/STEP imports) nests under it
    instead of appearing as a separate root.

### 3.2 Solid creation from sketches
- **Extrude** — blind, symmetric, to-object, with optional draft angle.
  - An **Extrusion** is a first-class feature element (own hierarchy row, nameable, undoable):
    it references one or more coplanar sketch faces (closed circle/polygon profiles — a
    rectangle is a four-line polygon loop) and a signed distance along the plane normal, and
    generates a solid mesh (prism per polygon, cylinder per circle). Each extrusion produces a
    **Body** (the solid result) that depends on it: the body nests under the extrusion in the
    Elements pane and is removed if the extrusion is deleted.
    Created in script via
    `bearcad.extrude{ circle|polygon|circles|text, distance?, name?, body?, to? }` (`text = i`
    extrudes/engraves a whole sketch text — all its glyph regions, #355).
    **Extrude to object (#114):** instead of a fixed distance, `to = { plane = i }` /
    `{ face = <face spec> }` / `{ vertex = <point> }` snaps the extrusion to that object's
    extended plane, and the link is parametric — the snapped extrusion follows when the
    target moves. `face` accepts either a flat sketch profile (`{circle=i}`/`{polygon={..}}`/
    `{boolean={..}}`) or, for a 3D body's cap/side wall (#126) — including another body
    entirely, not just the extrusion's own sketch — the same `{kind = "extrude_cap" |
    "extrude_side", extrusion, profile, top?/edge?}` shape `begin_sketch` uses.
    **Semantic push/pull (#114):** `bearcad.edit_extrusion{ extrusion, distance?,
    by?, to? }` edits a committed extrusion like dragging its gizmo — `by` nudges from the
    current effective depth, `distance` sets an absolute depth (clearing any snap target),
    `to` (re)snaps.
  - Implemented: the data model (Extrusion + Body) with `.bearcad` persistence; mesh generation;
    both hierarchy elements; depth-tested flat-shaded rendering; and the interactive **Extrude
    tool** (`E`): click coplanar faces to toggle inclusion (hover-highlighted), drag the normal
    gizmo or type a distance (expressions/variables) to set the depth (positive or negative),
    with a live **semi-transparent** preview solid that updates as you type; Enter commits, Esc
    cancels; double-click / right-click → Edit re-opens an extrusion for changing faces or
    length. **Cut preview (#142):** when the extrusion is a cut (see the body-mode choice
    below, including the #141 auto-cut on backward drag), the preview isn't an additive block —
    it's the target body *with the cut already subtracted*, rendered semi-transparently in
    place of the intact body, so the ghost looks like the finished cut. This needs the kernel
    to build the subtraction; if it can't, the intact body and the additive-block preview are
    kept. **Preview performance (#386):** both live previews are cached per
    (document, in-progress extrusion) so unchanged frames rebuild nothing, and **text**
    extrusions preview through the fast tessellated mesher instead of the kernel — a
    per-glyph boolean chain per frame made dragging an engraving's gizmo unusably laggy (a
    text *cut* previews as the additive block; the committed result still builds the real
    kernel solid). While an extrusion is being edited its committed body is hidden, so only the
    semi-transparent ghost preview is shown (the preview, not the old solid, reflects the
    in-progress edit). The gizmo handle floats a little above the solid's top face (rather than
    sitting on it), and typing a digit while the tool is active focuses the distance field and
    overwrites its value. Clicking a face also **focuses the distance field with its value
    selected** (#437) — every face pick re-focuses it — so an amount like `4ft` can be typed
    immediately, replacing the default. The extrusion (and its body) nests under the sketch it was built from.
  - **Push/pull a bare body face directly (#122):** the Extrude tool also accepts a click
    directly on an existing body's own cap or side wall (an `ExtrudeCap`/`ExtrudeSide`), no
    separate sketch needed — "drag a face straight off a solid," like many CAD tools. This
    creates an implicit sketch hosted on that exact face and mirrors its boundary into it (a
    circular cap gets a real circle, not a tessellated approximation), then starts a fresh
    single-face extrusion from it — a body face is never grouped with other faces into one
    multi-face extrusion, unlike coplanar sketch profiles. Sketching on an existing body's
    face merges into that body by default (§3.2's `body?` choice, #32), so pushing/pulling a
    face this way naturally extends the solid rather than creating a disjoint one.
  - **Flip direction (#354):** the extrude distance popup has a **Flip** button that reverses which
    side of the sketch plane the profile extrudes to (it negates the distance, keeping the typed
    depth), so a profile can be extruded to either side without having to drag the gizmo back
    through the plane. Dragging the gizmo past the plane still works too.
  - **Extrude-to-object**: during a gizmo drag, hovering a vertex/face/plane snaps the depth to
    that object and, on release, constrains the extrusion to it (`ExtrudeTarget`). This includes
    another body's cap/side wall (#126), not just a construction plane or flat sketch profile —
    except the cap/side faces of the extrusion currently being dragged itself, which would be a
    meaningless self-reference and are excluded from the snap candidates. **Repeated
    instances' faces snap too (#452)**: when the analytic pick misses, each repeat copy's
    translated cap/side faces are hit-tested directly (`pick_repeated_face`), producing
    `ExtrudeTarget::RepeatedFace { face, op, instance }` — the source face's plane
    translated by that instance's offset (`extrude::repeated_face_plane`; instance counts
    from 1, matching `repeat_offsets` listing only the copies) — which stays parametric
    when the repeat's spacing changes. Scriptable: `to = { face = {...}, repeat_op = i,
    instance = n }`. The effective
    depth is then derived from the target's extended plane — to a vertex's perpendicular plane,
    or where the extrusion axis meets a face/construction-plane — and recomputes if that geometry
    moves. A free gizmo drag (no object) leaves a plain unconstrained distance. The live ghost
    preview reflects the snapped target immediately while still dragging (not just after
    release), so extruding to a slanted or irregular target shows the actual resulting shape —
    e.g. a slanted top cap — rather than a generic blind/rectangular extrude (#63).
  - **Body target (#32/#35)**: a `Body`'s source is one or more extrusions (`BodySource::Extrusion`
    for one, `BodySource::Extrusions` for several; `BodySource::Solid { add, cut }` once some of
    its extrusions are subtracted rather than added — see §3.3). Extruding from a sketch on an
    existing body's face (a cap or side face) defaults to joining that body instead of creating a
    new one; the context pane shows three (icon-labelled) choices while extruding or editing an
    extrusion — **New body**, **Add to `<body>`**, and **Cut `<body>`** — to override the choice
    (editing can also split a merged/cut extrusion back out into its own body). The **Cut** option
    is only offered when the OCCT kernel is compiled in, since a non-kernel build can't perform
    the subtraction (see §3.3). **A cut must bite (#380):** committing a cut first checks
    (kernel builds, `extrude::cut_tool_bites`) that the tool solid actually overlaps the
    target body — a positive distance on a side face points *out* of the solid, which used to
    commit a silent no-op cut. An outward cut whose flipped direction would bite is
    **auto-flipped inward** (the commit-time analogue of the backward-drag auto-cut) with a
    status note; one that can't remove material in either direction commits as given with a
    **status warning**. Target-driven or expression-bound depths are never flipped, only
    warned. **Auto-cut on backward drag (#141):** when the sketch sits on a
    face of a body, that body lies on the negative-normal side, so dragging the extrude gizmo
    *backward* (negative distance) drives the profile into it — the mode auto-switches to **Cut**
    of that body; pulling forward again reverts to **Add to**. This only flips the cut toggle
    (an explicit **New body** choice is left alone on forward drags) and, like the manual Cut
    option, only engages when the OCCT kernel is present. Deleting one extrusion of a multi-extrusion body only drops that
    extrusion's contribution — the body survives as long as it still has at least one added
    extrusion. Scriptable via `bearcad.extrude{ ..., body = "merge" | "cut" }` (`"merge"` joins,
    `"cut"` subtracts from, the face's body). An explicit `"merge"`/`"cut"` requires the sketch
    to sit on a body face: with no such body it is a hard error (#178), never a silent
    fall-through to a new body. Omitted or any other value always creates a new body, matching
    the declarative/OpenSCAD-style default.
  - **Boolean-region face picking (#16/#62)**: when exactly two coplanar sketch shapes overlap
    with nonzero area (and no third shape also overlaps that pair — see scope below), clicking
    inside their combined footprint with the Extrude tool resolves to the specific atomic region
    under the cursor instead of a whole shape: their shared intersection, or one shape minus the
    other, via two point-in-polygon tests against the picked point. This is `ExtrudeFace::
    Boolean { op: BooleanOp::Intersection | Difference, a, b }` (`a`/`b` boxed `ExtrudeFace`s,
    recursive so the type stays general, though the interactive picker only ever constructs
    depth-1 combinations of two raw `Circle`/`Polygon` shapes) — toggled into
    `Extrusion::faces` exactly like any other face (multi-face selection already lets a union of
    two whole shapes be built by toggling both, so no separate `Union` variant is needed). The
    region's boundary is computed on demand through the single seam
    `crate::polygon_boolean::face_boolean` (#88): **kernel builds delegate to OCCT** (planar
    faces on z=0, `BRepAlgoAPI_Cut`/`Common`, result accepted only as exactly one hole-free
    face whose outer wire is walked in loop order), while `--no-default-features` builds use
    the retained hand-rolled two-simple-polygon Weiler-Atherton clip (`Difference` reverses
    the clip polygon's winding — the standard trick that turns the same intersection-walk into
    a subtraction), which is slated for deletion once Windows ships the kernel (#96). Both
    paths honor the same strictness contract and are held together by an occt-gated parity
    test matrix; the resolved loop feeds mesh generation, fill rendering, and
    hover-highlighting the same way a `Polygon` face's loop already does. Scriptable via `bearcad.extrude{ boolean = { op = "intersection" |
    "difference", a = <face spec>, b = <face spec> }, distance }`, where a face spec is
    `{circle=i}`/`{polygon={...}}` (a rectangle is a four-line polygon)/a nested `{boolean={...}}`.
    - **Rings / faces-with-holes (#268/#263)**: a `Difference` whose subtrahend lies strictly
      inside the minuend (concentric circles, a shape fully inside another) is an **annulus** —
      no longer rejected. `extrude::extrude_face_uv_region` resolves such a face into a
      **`UvRegion`** (an outer loop + interior **hole** loops), and both solid builders honor it:
      the **kernel** extrudes/revolves a `Boolean` face by building each operand's solid and
      applying the same boolean to the *solids* (`Difference`→cut, `Intersection`→common), so a
      concentric ring becomes a true **tube** (outer cylinder minus inner cylinder — exact walls,
      single circular rims); the **`--no-default-features` mesh fallback** builds hole-aware caps
      (`polygon::triangulate_planar_with_holes`, hole loops bridged into the outer loop and
      ear-clipped) plus inner side walls. This works for extrude (`ExtrudeFace`) and revolve alike.
    - **Scope (deliberate, not yet general N-way arrangements)**: only ever two shapes at a
      time — a sketch with three or more mutually-overlapping shapes falls back to today's
      whole-shape picking instead. The single-seam `polygon_boolean::face_boolean` (used for the
      click-resolution *boundary* and the fill/hover display) still only produces a result when
      the combination reduces to a **single simple polygon loop** — it returns `None` for a
      multi-part (disjoint-piece) or near-zero-area result — but an annulus is now recovered as a
      face-with-hole region for building (above) rather than rejected. No flat side-wall sketching is offered on a boolean-derived extrusion
      (`side_face_count` is 0 for it, mirroring `Circle`'s curved walls) since its edge count
      depends on the resolved (Document-dependent) geometry; the extrusion mesh itself is
      unaffected, since it walks the resolved profile loop directly.
- **Revolve** — about an axis, full or partial angle.

### 3.3 Combining solids
- **Boolean**: union, cut (subtract), intersect.
- **Extrude body modes (#32/#35)**: an extrusion commits into a body one of three ways — **New
  body** (its own body), **Add to body** (fused into an existing body's solid), or **Cut body**
  (subtracted from an existing body's solid). A body records its additive vs. subtracted
  extrusions in `BodySource::Solid { add, cut }`; `body_solid_mesh` fuses the added extrusions
  into one solid and then subtracts each cut extrusion via the kernel's `Shape::boolean(_,
  BoolOp::Cut)`, producing one watertight result instead of overlapping triangle soup. **Cut
  requires the OCCT kernel**: the hand-rolled non-kernel mesher can't subtract solids, so in a
  non-`occt` build a body with cut extrusions renders its additive geometry only (the cut is
  ignored) and the GUI doesn't offer the Cut option — a known limitation resolved once the kernel
  is the default (#89). The cut list round-trips through save/load regardless of build.

- **Combine tool (whole-body booleans):** operates on committed bodies rather than
  extrusions. Four operations: **Combine** (union of the picked set), **Cut** (A − B),
  **Intersect** (only what's common), **Difference** (symmetric difference — only what's
  *not* common). Combine uses a single picker; the two-sided operations have A and B
  pickers (multi-select each, clicking a body in the viewport toggles it into the active
  side) plus a **Keep B** toggle that leaves the B-side inputs as real bodies.
  - Committing creates a **boolean operation element** (`Document::boolean_ops`,
    `ShapeKind::BooleanOperation`) and one **output body per result solid**
    (`BodySource::Boolean { op, solid }`) — a cut or difference that severs a body into
    pieces yields one body per piece. The output count is fixed at commit; a parametric
    rebuild that produces *more* solids folds the extras into the last output body, fewer
    leaves trailing outputs empty, so the Elements pane stays stable.
  - The input bodies become **shadow bodies** (`Body::shadow`): still listed in the pane
    with their own dashed-cube icon, but hidden in the viewport (and excluded from picking
    and occlusion) except while hovered or selected in the pane, where they render as a
    translucent ghost with a wireframe. Hovering the operation row ghosts all of its
    inputs at once.
  - Dependencies: outputs nest under the operation in the pane; the operation depends on
    its inputs (enforced: an operation may only consume bodies that exist before it —
    outputs of *earlier* operations are fine, so booleans chain acyclically; shadow
    bodies can't be re-picked unless the operation being edited already owns them).
  - The operation element is **editable**: selecting it offers "Edit operation", which
    re-opens the pickers (kind, sides, keep-B) and applies in place, re-shadowing inputs
    accordingly. Deleting the operation tombstones its outputs and releases its inputs
    from shadow (unless another live operation still consumes them). Undo of a commit
    restores inputs and removes the operation and its outputs as one step.
  - Scripting: `bearcad.combine{ op = "combine"|"cut"|"intersect"|"difference", a = {…},
    b = {…}, keep_b?, name? }` and `bearcad.edit_boolean{ index, … }`; session-command
    export replays both. The result geometry is kernel-computed (difference is
    (A∪B) − (A∩B); multi-solid results split via `Shape::solids`), on desktop and web
    alike via the kernel module.

- **Move tool (#176/#183):** rigid translation and/or rotation of whole bodies. One
  multi-select body picker (viewport clicks toggle); translation X/Y/Z and the rotation
  angle are **expressions** (parameters work — moves rebuild parametrically); the rotation
  axis is a global axis or any clicked line. Committing creates an editable **move
  operation element** (`Document::move_ops`, `ShapeKind::MoveOperation`) with one moved
  output body per input (`BodySource::Moved { op, target }`); inputs become shadow bodies,
  exactly like the Combine tool. "Edit move" re-opens the tool (outputs grow/shrink with
  the target list; removed ones tombstone). Meshes transform on every target (works in the
  lean build); the BREP shape transforms through the kernel (`Shape::transformed`,
  `bearcad_shape_transform` natively and in the web kernel module) so moved bodies chain
  into booleans and export as real BREP. **Translation drag gizmos (#215):** with bodies
  picked, three axis arrows (X red, Y green, Z blue) at the targets' bounding-box centre drag
  to set the translation — the same offset-arrow handle as the extrude gizmo, driving the
  `move_x`/`move_y`/`move_z` values (so scriptable/testable via the gizmo API, §8).
  **Rotation ring (#216):** once a rotation axis is picked, a circle in the plane perpendicular
  to that axis (at the centroid, sized to the bodies) drags round to set the angle, driving
  `move_angle`. A **line selected while the Move tool is active** (Elements pane or viewport)
  sets the rotation axis, alongside the context pane's X/Y/Z buttons. Scripting:
  `bearcad.move_bodies{ bodies = {…}, x?, y?, z?, axis?, angle?, name? }` and
  `bearcad.edit_move{ index, … }`. **Moving construction planes (#217):** a Move op can also
  target a construction plane (`MoveOperation::plane_targets`) — at recompute the plane's frame
  is its base definition composed with the move, so everything anchored to it (sketches,
  images) follows, since that geometry is stored plane-local and projected through the plane
  frame. Planes are picked into the move set from the Elements pane / selection like bodies.
  **Moving tracing images (#217):** a Move op can likewise target a tracing image
  (`MoveOperation::image_targets`) — at recompute the image's plane-local `origin` is its
  pristine authored base (`TracingImage::base_origin`, the base/cache split planes have between
  `definition` and their cached frame) projected onto its host plane frame and pushed through the
  move, then read back in the plane's u/v axes. In-plane translation slides the image; out-of-plane
  translation is dropped (the image can't leave its plane); an image on a plane that also moved
  follows the plane and then takes its own move on top (image recompute runs after plane recompute).
  Dropping an image from a move restores its authored base. Images join the move set from the
  Elements pane / selection like bodies and planes. **Coalescing (#217):** re-moving the same
  element (the same planes, the same images, or the moved-output bodies of an existing move) folds
  into that move op instead of stacking a new one, so a run of test nudges stays a single operation
  — for the representable cases: translations add, and same-axis rotations add their angles (a
  differing-axis rotation or a translate+rotate mix starts a fresh op, since `MoveOperation`'s
  single-axis representation can't express an arbitrary composition). Moving sub-body geometry
  (faces/edges/vertices) remains a follow-up (#185).
  **In-sketch selection gizmo (#306):** inside an open sketch the Move tool is a different
  thing entirely — the body-move controls are hidden, and instead a gizmo appears at the
  selected geometry's bounding-box centre: a **centre disc** drags the whole selection freely
  across the plane, and a **horizontal (u, red)** and **vertical (v, green)** push-pull arrow
  constrain the drag to that sketch axis. It translates every selected line and circle (with
  their coincident closures) and any selected sketch texts together, re-solving constraints
  each step and rolling back if a pin would stretch a selected edge (the #243 guard), reusing
  the line-drag machinery (`vertex_drag::begin_selection_drag_session` / `drag_selection`,
  driven by `Action::BeginSelectionDrag`/`DragSelection`/`EndSelectionDrag`). Select the
  geometry first (Select tool), then switch to Move — the selection persists across the tool
  switch.

- **Linear repeat tool (#182/#257):** copies of whole bodies spaced along an axis, chosen with
  an **element picker** of one edge/axis (a global X/Y/Z axis or a clicked straight sketch
  line; the ✕ clears it) (#257). **Pane polish (#440–#447):** the Gap/Distance
  measure-toggle icons hover **gold**; the X/Y/Z quick buttons highlight the picked axis
  (clicking the active one clears it) and the picker's ✕ clears it too; each of
  Count/Gap/Distance carries a **pencil toggle** — two editable (pencil lit), one
  computed (pencil dimmed; clicking an off pencil makes that field editable via the MRU,
  clicking an on pencil makes it the computed one, `CreatingRepeat::set_computed`);
  editable fields are expression inputs with autocomplete/error display and a `= value`
  computed preview beside them; the "N instances" label is gone (Count shows it) and the
  commit button sits in the input column. **Esc** drops the in-progress repeat (clearing
  the ghost previews, #450). **Selection seeding (#439):** activating the tool seeds its
  targets from the current selection (bodies/planes/sketches), the axis starts **unset**
  (`CreatingRepeat::axis: Option` — commit refuses without one), and exactly one picker reads
  focused: the axis while it's unset and targets exist, the bodies picker otherwise. The Default-units section is hidden while the tool is active.
  One multi-select body picker; the original stays as instance 0; each
  further instance of each target is an output body (`BodySource::Repeated { op, target,
  instance }`) nested under an editable **repeat operation element**
  (`Document::repeat_ops`, `ShapeKind::RepeatOperation`). The context pane exposes three
  interlinked variables — **count**, **gap**, and **distance** (#257): the user edits any two
  and the third is **computed** and shown read-only in its field (the least-recently-edited one
  is the computed one; `RepeatMode::from_repeat_ui`). Two **picture toggles** (clickable SVGs of
  two rectangles with a measurement line) switch how gap and distance are measured — gap as a
  clear space *between* items vs a start-to-start **offset** (pitch), and distance measured to
  the last item's **end** vs its **start**. These map onto the spacing modes count × gap /
  count × pitch, count fit-to-end / fit-start-to-start, and fill-by-length / fill-by-span (gap
  or pitch), plus a legacy fill-with-*maximum*-pitch stud-spacing mode. Gaps may be negative.
  Count/spacing/length are **expressions** (parameters
  work); the context pane shows the live instance count as they change, and the viewport shows
  translucent **ghost previews** of the would-be instances while count/spacing change (#223, the
  picked bodies' meshes translated to each `repeat_offsets` offset); instance counts
  clamp at 512. End-to-start measurements use the targets' combined extent along the axis.
  "Edit repeat" re-opens the tool and resizes the output list. The fill length `L` may instead
  be **bound to a target's extended plane** (`RepeatOperation::length_target`, an
  `ExtrudeTarget` like an extrusion's "up to face" #126): `L` is then the along-axis distance
  from the pattern start to that plane and follows the face if it moves, overriding the `length`
  expression (#186). Scripting:
  `bearcad.repeat_bodies{ bodies, axis, mode, count?, spacing?, length?, name? }` /
  `bearcad.edit_repeat{ index, … }`.
  - **Repeating construction planes (#221):** a repeat can also target construction planes
    (`RepeatOperation::plane_targets`), picked from the Elements pane / viewport with the tool
    active. Each further instance is a generated `ConstructionPlane` carrying a
    `RepeatPlaneInstance { op, target, instance }`; its cached frame is the source plane's
    *current* frame offset along the axis, so instances step along the axis (planes are
    zero-thickness, so the step is the bare gap/pitch) and follow the source if it moves.
    Instances are grouped under the repeat op in the Elements pane, and go away with it. Count
    and spacing are the same expressions/modes as body repeats; a repeat may target bodies and
    planes at once.
  - **Repeating an operation (#220):** a repeat can target an **extrusion**
    (`RepeatOperation::extrusion_targets`) and replay its *effect* at each offset rather than
    copying a solid — a **cut** extrusion's tool is subtracted again (`occt_body_shape_from_indices`)
    to punch N holes, and an **add** extrusion's solid is fused again (`occt_fused_extrusions`) to
    grow N bumps. No output bodies; the extra ops fold into the target body's shape at build time
    (spacing is center-to-center, extent 0). Scripting: `bearcad.repeat_cut{ cuts = {ei}, axis,
    mode, count?, spacing?, length? }` (works for add or cut targets). The Repeat tool picks an
    extrusion operand by clicking it (Elements pane / selection → `extrusion_targets`, shown as an
    operation count in the context pane, #235); the op is a selectable/deletable `RepeatOp` whose
    deletion drops the replay.
  - **Repeating whole sketches (#226):** `RepeatOperation::sketch_targets` copies a
    construction-plane-hosted sketch at each offset. Each copy rides a fresh construction plane
    parallel to the source's, translated along the axis (`rebuild_repeated_sketches`), and carries
    copies of the source's lines/circles (plane-local coords unchanged, so entity world positions
    step by the offset). The source may be hosted on a construction plane **or a body face**
    (#231): the copy plane is synthesized from the source sketch's frame either way. Regenerated
    on create/edit; the copies (planes, sketches, entities) go away when the op is deleted, and
    they nest under the repeat op in the Elements pane (host planes grouped under the op, not at
    the top level). The Repeat tool picks a sketch operand the same way it picks bodies/planes
    (Elements-pane / selection click → `sketch_targets`, shown as a count in the context pane,
    #234). Scripting: `bearcad.repeat_sketches{ sketches = {si}, axis, mode, count?, spacing?,
    length? }`.
  - **2D in-sketch repeat (#222):** `SketchRepeatOperation` (`Document::sketch_repeat_ops`)
    duplicates selected sketch **lines/circles** along an in-plane direction as generated
    entities in the same sketch, reusing the shared spacing math
    (`extrude::spacing_offsets`, `extrude::sketch_repeat_offsets`). Copies are driven geometry
    (no dimensions/constraints of their own), regenerated by `rebuild_sketch_repeat` on create
    and edit. Scripting: `bearcad.repeat_sketch{ sketch, lines?, circles?, angle?|dir?, mode,
    count?, spacing?, length? }` / `bearcad.edit_sketch_repeat{ index, … }`. The operation is a
    first-class pane element (`SceneElement::SketchRepeatOp`/`HierarchyNode::SketchRepeatOp`,
    #228): its duplicated lines/circles nest under it in the Elements pane (excluded from the
    sketch's own listing), and it is selectable, nameable, and deletable (delete removes the
    copies). **Interactive tool (#232):** with a sketch open, the **Repeat** tool repeats sketch
    entities — click lines/circles to toggle them into the set, **Shift+click an edge** to set
    the repeat direction (its unit vector; default is the sketch U axis), **Enter** commits a
    `SketchRepeatOperation` (a `CreatingSketchRepeat` draft carries the same count/gap/distance
    parametrization as the 3D repeat). The context pane shows the entity count, the direction,
    and the **count/gap/distance** fields with the same picture toggles as the 3D repeat, plus a
    **Repeat** button. A live dashed **ghost preview** of the duplicated lines/circles is drawn at
    every computed offset while the draft is in progress.
  The GUI/scripting to *pick* the length face is a tracked follow-up.

- **Slice tool (#181):** cuts whole bodies with planar cutters. Two pickers — **Bodies**
  (the targets, multi-select) and **Cutters** (construction planes and/or planar body
  faces, multi-select) — with a *Picking* switch in the context pane choosing where the
  next viewport click lands. Each target is split independently: for every cutter the
  current pieces are divided by the cutter's plane, so *n* cutters through a body can yield
  up to *2ⁿ* fragments. Each fragment is an output body (`BodySource::Sliced { op, target,
  piece }`) nested under an editable **slice operation element** (`Document::slice_ops`,
  `ShapeKind::SliceOperation`); the input body becomes a **shadow body** exactly like the
  Combine tool, and fragments chain as ordinary bodies into further operations. The
  **Extend cutters to infinity** toggle (default on) treats each cutter as an infinite
  plane; turned off, a cutter only separates material within its own face footprint. The
  slicing runs through the OCCT kernel (half-space booleans); a cutter that misses a body
  leaves it whole. "Edit slice" re-opens the tool and resizes the fragment list; the whole
  slice undoes as one step. Scripting: `bearcad.slice{ bodies, cutters, extend?, name? }` /
  `bearcad.edit_slice{ index, … }`.
  - **2D in-sketch slice (#224):** `SketchSliceOperation` (`Document::sketch_slice_ops`) splits
    target sketch **lines** at their interior crossings with cutter lines. Each split original is
    flagged `shadow` (kept for editing but excluded from face detection, like a shadow body —
    `polygon::closed_line_loops` skips shadow lines), and its pieces become fresh `Line` entries
    grouped under the op (`rebuild_sketch_slice`, `segment_crossing_t`). Scripting:
    `bearcad.slice_sketch{ sketch, lines, cutters }` / `bearcad.edit_sketch_slice{ index, … }`.
    The operation is a first-class pane element (`SceneElement::SketchSliceOp`/
    `HierarchyNode::SketchSliceOp`, #229): its fragment lines nest under it (excluded from the
    sketch's own listing; shadowed originals stay listed, dimmed), and it is selectable, nameable,
    and deletable (delete un-shadows the originals and removes the fragments). **Curved** targets
    and cutters work too (#233): crossings are found by intersecting the entities' sampled
    polylines, and a curved target is split with de Casteljau so each fragment keeps its bezier
    shape. **Circle targets** (#237) split too: line↔circle crossings give the arc angles, the
    circle is shadowed, and each arc is emitted as bezier fragment lines (≤90° per cubic).
    **Face (loop) slicing** (#238): a `face_targets` entry (a closed boundary loop's line indices)
    is bisected where a cutter crosses its boundary at two points — the two crossed edges are
    split, a cut **chord** line is emitted between the crossings, and generated coincidence
    constraints (`constraint_outputs`, tombstoned/regenerated on rebuild like the fragments) stitch
    the pieces so the loop resolves into two faces. The split pieces inherit the crossed edges'
    corner coincidences, so uncrossed neighbours attach to the correct side. This works because
    `closed_line_loops` now extracts **minimal, vertex-simple** faces: it drops self-touching cycles
    (running twice through a cut point) and any loop an internal chord subdivides (the reconstructed
    outer perimeter), so exactly the two half-faces survive — a no-op for ordinary sketches, whose
    loops are already minimal and simple. Scriptable via `faces = { {l0,l1,…}, … }`.
    **Interactive in-sketch Slice tool** (#238): with a sketch open, the Slice tool picks target
    lines/circles/faces and cutter lines with **two roles**, like the Combine tool's side-A/side-B
    element pickers (`CreatingSketchSlice`, `picking_cutter` chooses which the next viewport click
    feeds). Clicking a line/circle toggles it as a target; clicking empty space inside a face
    toggles that face (`face_loop_at_world` picks the smallest containing loop); while the Cutters
    picker is active, a click toggles a cutter line. The context pane shows both pickers and a
    Slice button; Enter commits. The draft is cleared when the tool changes or the sketch is
    exited.
  Picking side-wall faces as cutters remains a tracked follow-up (#191).

### 3.4 Modifying solids
- **Fillet** and **Chamfer**, 2D sketch vertices: the tools described in §3.1 (#37/#38) —
  truncate-and-bridge on a sketch vertex where two lines meet, with the fillet arc approximated
  by a single bezier segment on the bridging `Line`.
- **Fillet** and **Chamfer**, 3D solid edges (#77): with the OCCT kernel linked (`--features
  occt`, see §10) these are **true BREP fillets/chamfers** — the extrusion builds a real OCCT
  solid and `BRepFilletAPI_MakeFillet`/`MakeChamfer` is applied to the matched edges (matched by
  their analytic world-space endpoints), producing genuine tangent-continuous rounded / flat
  beveled surfaces, then tessellated for the viewport. In the default build (no kernel) the same
  edges get a **mesh-bevel approximation** instead: it doesn't attempt a tangent-continuous
  curved surface, correct face trimming, or vertex-miter blending where 3+ edges meet; it
  directly reshapes the extrusion's own triangle mesh. If the kernel can't place a treatment (an
  edge it can't match, or an OCCT error) that extrusion falls back to the mesh-bevel path, so
  broken geometry never ships. A treatment the kernel can't build at all (e.g. a fillet radius
  larger than the solid) is **rejected at commit time** via a kernel trial-build (#103), and if
  a cut-bearing body ever does render the additive-only fallback (e.g. a pre-existing infeasible
  treatment in an old document), the status bar warns that its cuts are not shown. Both paths are scoped to bodies whose source is one or more
  `Extrusion`s with a `Polygon` profile (a rectangle being a four-line polygon), and to the two
  edge families that have a clean
  analytic definition there (see `crate::extrude::side_quad_world`/`cap_polygon_world`):
  - a **vertical side edge**, where two adjacent flat side walls of the profile meet, and
  - a **side/cap edge**, where a side wall meets the top or bottom cap.

  In the mesh-bevel fallback, **Chamfer** replaces the edge with a single flat bevel quad
  connecting the two originally adjacent faces, offset back from the edge by the chamfer distance
  on each side (the same truncate-by-`amount` math as the 2D vertex case,
  `crate::model::vertex_treatment_geometry`, generalized to arbitrary 3D corners via
  `crate::extrude::corner_bevel_3d` — any two rays from a shared point span a flat 2D subspace,
  so this is an exact, not approximated, embedding). **Fillet** replaces it with an N-segment
  faceted rounded bevel instead of a true curved surface, sampling the same cubic-bezier arc
  approximation the 2D fillet uses, faceted at `EDGE_TREATMENT_FILLET_SEGMENTS` (= `BEZIER_
  SEGMENTS`, the existing curve-faceting precedent). The `occt` build instead produces the true
  BREP fillet/chamfer surface described above.
  - **Circle cap rims (#177, kernel builds)**: a `Circle`-profile extrusion's two cap rims
    are treatable as `Cap { edge: 0, top }` — one continuous circular edge each. Circle
    extrusions build as **true BREP cylinders** (`Shape::cylinder`, not a prism over the
    sampled 48-gon), so the rim is a single circular edge the kernel chamfers/fillets
    exactly; the FFI edge matcher gained a closed-edge pass (two diametrically opposite
    request points, matched by curve projection) since a seam-vertex circle can't be matched
    by endpoints. Rim treatments on a **cut** extrusion are **countersinks**: the tool is
    built without them, subtracted, and the treatment is applied to the resulting body's
    hole rim (beveling the tool itself would leave a lip — the inverse). The kernel
    feasibility trial accordingly trials the owning *body* build when there is one. Rims are
    kernel-only: the no-kernel mesh fallback renders them untreated (its bevel builder is
    polygon-vertex-based), and no analytic rim edges are offered for picking in a lean
    build. Slanted-target (lofted) circle extrusions keep the sampled profile and stay
    untreatable.
  - **Explicitly out of scope**: `Circle`-profile *vertical* edges (a smooth wall, nothing
    to bevel); STL/STEP-imported bodies (pure triangle soup, no analytic
    profile to derive an edge from — #31's generic mesh-feature-edge extraction still works for
    *picking/hovering* those edges for plane-referencing, just not for beveling them); and a
    **vertex miter** where 3+ treated edges would meet at a shared corner — rejected at commit
    time (`crate::extrude::edge_treatment_conflicts`) rather than attempting to blend three
    bevels together, a documented limitation rather than a crash or wrong-looking result.
  - **Data model**: parametric, like everything else in this app (re-evaluated from the document
    every frame, not a one-time mesh edit). Each `Extrusion` carries `edge_treatments: Vec<
    EdgeTreatment>`, where `EdgeTreatment { edge: ExtrusionEdgeRef, kind: VertexTreatmentKind,
    amount: f32 }` and `ExtrusionEdgeRef` names the analytic edge family + index (`Vertical {
    face, edge }` or `Cap { face, edge, top }`, `face` indexing `Extrusion::faces`). `kind`
    reuses `VertexTreatmentKind` (Chamfer/Fillet) from the 2D case directly. `crate::extrude::
    extrusion_mesh` applies every treatment on a face while building its mesh.
  - **Interactive tool**: the same Chamfer/Fillet tool (`K`/`F`) as the 2D case — when a sketch
    is open it behaves exactly as §3.1 describes; when no sketch is open, clicking a body's
    analytic edge (picked directly from the edge list, not the generic mesh-feature-edge
    extraction, since the structured `ExtrusionEdgeRef` is needed) starts a parallel in-progress
    state and shows the same push/pull gizmo (anchored at the edge midpoint, pointing along the
    inward bisector of the two adjacent faces) with a live semi-transparent ghost-preview solid
    (reusing the extrude tool's `preview_extrusion`/`editing_extrusion` mechanism: a clone of the
    extrusion with the live treatment spliced in, the committed body hidden meanwhile) — drag or
    type an amount, Enter/click commits, Esc cancels.
  - **Selection picker (context pane, #157/#167)**: while the Chamfer/Fillet tool is active
    outside a sketch, the context pane shows a **selection picker** — one row per edge in
    the in-progress set (named by owning extrusion + analytic edge), each with a remove
    button, plus a clear-all; when the set is empty it shows a pick hint ("Click an edge —
    Shift+click adds more"). The picker is the first instance of the generalized per-tool
    selection input (future tools may host several, e.g. boolean A/B sets).
  - **Multi-edge sets (#157/#166)**: the in-progress treatment holds a *set* of edges sharing
    one amount/gizmo. Shift/⌘+click toggles additional treatable edges into the set (a plain
    click restarts with just the clicked edge); switching to Chamfer/Fillet with body edges
    already selected (Select mode, #156) **preloads** the selection — filtered to treatable
    edges — and shows the gizmo immediately. Commit applies every edge in one undo group;
    edges that individually fail (e.g. a vertex-miter conflict) are skipped with a status
    note while the rest apply. Each commit pushes a transient `EdgeTreatmentEdit` marker
    with a snapshot of the prior treatment list (#168, mirroring construction-plane edits),
    so **Undo reverts the whole treated set** — restoring any replaced treatments — without
    touching the extrusion itself. The ghost preview shows the gizmo-anchoring extrusion's edges
    (a set spanning several extrusions still commits everywhere, but only the primary
    extrusion gets a ghost — the preview mechanism shows one extrusion at a time).
  - Scriptable via `bearcad.chamfer_edge{ extrusion =, edge = {...}, distance = }` and
    `bearcad.fillet_edge{ extrusion =, edge = {...}, radius = }`, where `edge` is `{ kind =
    "vertical", face =, edge = }` or `{ kind = "cap", face =, edge =, top = }`.
  - **Elements-pane node + edit-after-the-fact (#192/#259):** each committed edge treatment shows
    as a display-only row (`HierarchyNode::EdgeTreatment`, chamfer/fillet icon, "Chamfer/Fillet
    (amount)" label) nested under its extrusion. It has no `SceneElement` — it isn't
    individually selectable or hideable — but double-clicking the row (or right-click → "Edit
    chamfer/fillet") reopens it with its push/pull gizmo and amount input via
    `EditEdgeTreatment`; adjusting and committing re-commits that same edge through
    `CommitEdgeTreatment` (which updates the existing treatment in place, undoably), so a
    fillet/chamfer radius can be changed after it's made without re-picking the edge.
- **Shell** — hollow a solid to a wall thickness, removing selected faces.

### 3.4.1 Tracing images (#163)
- **Import (#169):** File → Import Image…, or right-click a construction plane in the
  Elements pane → "Import image on this plane…" to target that plane directly (#175)
  (or `bearcad.import_image("p.png")` /
  `bearcad.import_image{ path =, plane = }`) embeds a PNG/JPEG in the document (base64 in
  the saved JSON, so files stay self-contained like imported meshes) and places it on a
  construction plane (default: plane 0), centered on the plane origin at an initial scale
  of **1 px = 1 mm**. The image is an Elements-pane row nested under its host plane —
  renamable, hideable, deletable, undoable.
- **Rendering (#170):** each image draws as a **textured quad** on its host plane at 85%
  opacity — depth-tested (bodies in front occlude it) but never writing depth, so sketch
  geometry and fills always read on top. Decoded pixels and GPU textures are cached by
  content, so the per-frame cost is one quad.
- **Scale calibration (#163/#171):** the guided flow starts from the image itself: select
  the tracing image and the context pane shows a **Calibrate scale** button. Clicking it
  enters a point-placing mode — click **two points** on the image over a feature of known
  size (the placed points, the span between them, and a live rubber band to the cursor are
  previewed; Esc cancels; picking another tool cancels) — then the context pane shows the
  length field: typing the feature's real length rescales the image uniformly about the
  span's midpoint so the marked span measures that length. The calibration (reference
  segment in image-UV + assigned length) is stored on the image for re-editing, and
  re-running calibration replaces it. **Marker editing (#424):** a dot under the cursor
  previews each placement click; the length field **pre-fills** with the span's current
  measured length (`context::sync_calibrate_draft`, re-syncing whenever the span
  changes); with a calibrated image selected the marker line and points stay visible,
  the context pane re-opens the editable length (Apply re-calibrates the stored span),
  and either point can be **dragged** (`Action::SetCalibrationPoint` — updates the
  stored uv, never rescales) or **clicked + Deleted**
  (`Action::RemoveCalibrationPoint` — the guided flow re-opens holding the other point,
  so the next click re-places it). Scriptable: `bearcad.calibration_point{ image, index,
  x, y }` / `bearcad.remove_calibration_point{ image, index }`.
- **Image constraints & viewport move-pick (#425):** a calibrated image's two reference
  points are first-class constraint points (`ConstraintPoint::ImageCalibrationPoint`),
  pickable/snappable in sketches hosted on the image's plane and usable in
  coincident/midpoint/distance constraints against vertices, lines, and the
  origin/axes. Solving **translates** the whole image (`set_point_uv` shifts `origin`
  and `base_origin`; scale never changes), and the solver holds the non-image side of a
  point-point coincidence so the image follows its target. The Move tool also picks an
  image by clicking its quad in the viewport (`App::pick_tracing_image`), not only from
  the Elements pane. Scriptable: `bearcad.select{ kind = "image", index, point = 0|1 }`. Alternative segment source: a **line** drawn on the
  image's plane, selected together with the image, feeds the same length field. Scriptable
  via `bearcad.calibrate_image{ image =, from = {x, y}, to = {x, y}, length = }`
  (plane-local coordinates). *Known limitation:* calibration mutates the image in place and
  is not yet individually undoable (3D edge treatments had the same gap and now undo via a
  transient snapshot marker, #168 — calibration can adopt the same mechanism).

### 3.4.3 Sketch text (#282)
- **Text tool:** with a sketch open, the **Text** tool (sketch toolbar, or the **T** shortcut
  — #311; T still means the Tangent constraint while drawing a line, with a sketch vertex
  selected, or in the Constraint tool) places a `SketchText`
  element. **Clicking** drops a textbox that grows in width to fit the text; **dragging a
  rectangle** (#282) drops one that **word-wraps** to the dragged width and grows downward (the
  drag width becomes the `wrap_width`). While the drag is held, a **dashed rubber-band
  rectangle** previews the box (#407; drawn once the drag passes `TEXT_DRAG_MIN_WIDTH_MM`,
  3 mm, the same threshold that separates a drag from a click), and the status line
  advertises the gesture. Its glyph outlines are **baked** at create/edit time
  from a system font into sketch-local mm contours (`src/text.rs`: `fontdb` selects the font by
  family+weight/italic and yields its bytes; `ttf-parser` walks each glyph's outline, flattened to
  polylines and laid out along the baseline by each glyph's advance, multi-line stacking by
  ascent/line-gap; word-wrap breaks words that overflow `wrap_width` onto new lines,
  `text::outline_text_wrapped`). The **source font bytes are embedded** in the document (base64
  in JSON) so the text renders identically on a machine that lacks the font — like a PDF; if the
  font is missing on load, the stored outlines still render.
- **Model/rendering:** `SketchText` stores the string, font family, bold/italic/underline, size
  (+ expression), baseline origin, rotation, optional wrap width, the baked `contours`, and the
  embedded `font_bytes`. The baked contours (outer loops + counters/holes, separated by winding)
  render as closed polylines on the sketch plane, transformed by the element's origin/rotation. A
  `SketchText` is a first-class element — one node nested under its sketch in the Elements pane and
  graph, selectable/renamable/deletable/undoable; selecting it selects the whole text. Persisted in
  the `.bearcad` file (`sketch_text` nodes). Editing (`EditSketchText`) re-bakes from the font,
  falling back to the stored outlines when only the transform/style changed and the font is gone.
- **Context editor (#286):** selecting exactly one text opens its editor in the context pane: a
  multi-line textarea, a font-family chooser listing the installed families (`fontdb`) with
  **each name rendered in its own face** (#384 — faces register with egui lazily as the
  chooser's virtualized rows scroll into view, so unbrowsed fonts never load),
  **B**/**I**/**U** style toggles, a **Size** field accepting length expressions (parameters
  work: `w / 2`) with **± stepper buttons** that bump the evaluated size by 1 mm (#385,
  replacing any expression with the stepped literal, floored at 1 mm), a **Rotation°** field
  in degrees, and a **Wrap width** field (mm; empty
  grows the box to fit, a value word-wraps to that width, #282). Every change re-bakes the
  glyphs immediately. A size expression is stored as typed; the evaluated size only moves once
  the expression is valid, so mid-edit states don't clobber the text.
- **Move-tool rotation (#286):** with the **Move** tool active and one text selected, the
  rotation-ring gizmo (#216's ring) appears in the sketch plane around the text's baseline
  origin, sized to the glyph outlines; dragging the ring turns the text about its origin, live.
  The ring and the context **Rotation°** field read the same model value, so they stay in sync.
- **Constrainable anchors (#408, replacing #356/#359's bespoke pin):** each of a text's nine
  bounding-box anchors (`model::TextAnchor` — four corners, four edge midpoints, centre) is a
  first-class sketch point: `ConstraintPoint::TextAnchor { text, anchor }`. Anchors are
  pickable with the Constraint tool (a **selected** text draws them as dots,
  `text::sketch_text_anchor_points`), are snap targets for dragged vertices, and plug into
  `Coincident`/`Midpoint`/distance constraints like any vertex. Solving **translates** the
  whole text (`set_point_uv` writes `origin = solved − rotated anchor offset`); rotation and
  size never change from constraints, and the solver **holds the non-text side** of a
  point-point coincidence so the text follows the target, matching the old pin semantics.
  Texts re-bake *before* the solve (`recompute_document_geometry`), so anchors are computed
  from current contours, and `EditSketchText` re-solves so a resized text keeps its anchor in
  place. Scriptable: `bearcad.select{ kind = "sketch_text", index = i, anchor = "center" }`
  then `bearcad.add_geometric_constraint("coincident")`. Legacy documents with a
  `SketchText::pin` migrate on load (`storage::migrate_text_pins`) to an equivalent
  `Coincident` constraint; the pin field is never written back.
- **Width drag handles (#409):** a **selected** wrapped text draws its box (full wrap width ×
  glyph-bbox height, `text::wrap_box_baseline`) as a dashed outline with a handle at the
  mid-height of each vertical edge. With the Select tool, dragging a handle resizes the wrap
  width live (`Action::ResizeSketchText`, re-wrapping from the **embedded** font bytes and
  re-solving anchor constraints); the right handle keeps the origin, the left handle shifts
  the origin so the right edge stays put. Width clamps at `MIN_TEXT_WRAP_MM` (2 mm).
  Scriptable as the `"text_width"` gizmo (`available_gizmos`/`set_gizmo`), exposed whenever
  the selection is exactly one wrapped text.
- **Text-on-curve groundwork (#286):** `SketchText` carries an optional `baseline_line`
  reference (default none = straight baseline). Baking currently advances a pen along a
  straight baseline (`text::outline_text`); curve support later resolves the reference into a
  baseline provider (position + tangent per pen offset) at bake time, without reshaping the
  stored model.
- **Extrude/cut (#285):** the Extrude tool treats a sketch text as an extrudable face set —
  clicking a text toggles one `ExtrudeFace::TextGlyph { text, glyph }` per glyph (grouped by
  `text::group_glyphs`: the larger loops are outer boundaries, smaller loops nest as holes of the
  tightest enclosing outer). Each glyph builds as a **face-with-holes** (reusing #268: the kernel
  cuts each counter's prism from the glyph's outer prism; the mesh fallback uses hole-aware caps),
  so counters (`o`, `a`, `e`, …) come out. The whole string extrudes or cuts as one operation.
- **Scriptable:** `bearcad.text{ text =, x =, y =, size = (expression), font =, bold =,
  italic =, underline =, rotation = (degrees), wrap =, name = }` places a text declaratively
  (beginning a ground sketch when none is open, like `rect`/`circle`); tool name `text`;
  element kind `sketch_text` (works with `select`/`set_name`/`set_visible`/`count`); extrude
  face spec `{text_glyph = {text = i, glyph = g}}`. Each text is a pane row nested under its
  sketch — `Text N ("string")` with the Text-tool icon — selectable there like any element.

### 3.4.2 Web build (wasm32)

BearCAD also compiles to **wasm32-unknown-unknown** and runs in the browser (built by
`scripts/build-web.sh`, hosted at `/app/` on the docs site, deployed by the Website CI
workflow). The web build is the lean configuration plus web-specific plumbing:

- **The OCCT kernel ships as a second wasm module** (`scripts/build-occt-wasm.sh`:
  OCCT + the same C++ shim compiled with Emscripten into `kernel.js`/`kernel.wasm`). The
  app — which is wasm32-unknown-unknown and can't link Emscripten C++ — calls its
  16-function C API through a JS bridge (`web/kernel-bridge.js`, `src/kernel/web.rs`);
  shape handles cross the boundary as heap-pointer integers, arrays are copied between
  module heaps, and STEP bytes go through the kernel module's in-memory filesystem. Full
  geometry parity: cuts, booleans, BREP fillets/chamfers/countersinks, STEP both ways.
  If the kernel module fails to load, the app still runs with the lean fallbacks, and the
  boot status line reports the kernel self-check either way.
- **No SQLite; Lua runs as a side module** — bundled C doesn't compile for
  wasm32-unknown-unknown, and mlua's bindings can't cross a module boundary, so mlua's
  REPL/CLI are compiled out and SQLite storage is JSON instead (below). Browser scripting
  mirrors the OCCT kernel: the Lua interpreter (Lua 5.4, vendored in `third_party/lua/`) ships
  as a *second* Emscripten module (`cpp/bearcad_lua.cpp`, built by
  `scripts/build-lua-wasm.sh` into `web/lua/`). A small Lua prelude in that module makes every
  `bearcad.*` call forward its name plus JSON-encoded arguments through one hook back to the
  app — `globalThis.bearcadDispatch(name, json_args) -> json` — and the Rust side
  (`src/web_lua.rs`) routes it through `src/script_json.rs`, which turns the command name +
  JSON arguments into the same `Instruction`/query the desktop mlua closures drive, executed
  against the live `AppState`. So both frontends drive the identical Instruction/Action layer.
  **File → Load Script…** exists on both platforms — desktop runs the `.lua` through mlua, web
  picks the file and feeds it to the Lua module, which routes each call back into the
  dispatcher. If the Lua module fails to load, scripting is reported unavailable and the rest
  of the app runs normally.
- **In-window menu bar** (`src/web_menu.rs`): the browser has no OS menu bar, so File/Edit/
  View/Help render as an egui menu strip emitting the same `MenuCommand`s
  (`src/menu_command.rs`, shared with the muda native menus) through one dispatch path.
- **Documents are JSON**: `storage::to_json_bytes`/`from_json_bytes` (the whole `Document`
  serde-serialized). Native `open()` sniffs file magic and accepts either format, so
  web-saved `.bearcad` files open on desktop. Nothing persists to browser storage — open
  and save go through the browser's file pickers (`rfd::AsyncFileDialog`; saving downloads
  the file), as do STL/STEP/image import and STL/STEP export (byte-level `AppState`
  helpers: `open_document_bytes`, `import_*_bytes`, `export_*_bytes`).
- **Entry point**: `eframe::WebRunner` into the `bearcad_canvas` element of
  `web/index.html`; `web-time` stands in for `std::time::Instant`; wgpu's `webgl` feature
  provides the fallback for browsers without WebGPU.

### 3.5 Advanced features
- **Sweep** — sweep a profile along a path.
- **Revolve** *(implemented)* — spin one or more coplanar closed profiles around an axis
  into a solid. The **Revolve** toolbar tool collects profile faces by clicking (same face
  picking as Extrude), then an axis: any line in the sketch (plain, construction, or
  projected) or a global X/Y/Z axis. The sweep angle defaults to **360°** and is set by
  dragging a push/pull disc handle **around an arc** — the arc sweeps from the profile to the
  current angle and the handle rides its far end (#262) — or by typing (bare numbers are
  degrees; `rad`/`deg` suffixes and parameter expressions work); **Symmetric** sweeps half the
  angle to each side of the
  profile plane. The context pane shows the picked profile faces and the axis as their own
  element pickers (each row has a ✕ to remove it; faces/axis are still added by clicking in
  the viewport) (#261). The result lands as a **new body**, **fused into touching bodies**
  (resolved at commit by mesh-bounds intersection), or **cut from picked bodies** — chosen
  with a segmented icon button group (New body / Add to touching / Cut, the same icons the
  Extrude "into" picker uses) (#261); cut targets are clicked in the viewport and listed in
  the context pane's generic selection picker. Data model: `Revolution { sketch, faces, axis, angle_deg, symmetric, mode }` in
  `Document::revolutions` with `RevolveMode::{NewBody, AddTo(bodies), Cut(bodies)}`;
  add/cut relationships live on the revolution (bodies consult `revolutions_targeting` at
  mesh/kernel build time), and a NewBody revolve gets `BodySource::Revolve`. One
  `ShapeKind::Revolution` undo marker covers the feature and its body. Kernel builds use
  `BRepPrimAPI_MakeRevol` (full revolutions via the no-angle constructor — the angle
  constructor normalizes mod 2π and would build a sliver from a float 2π) with symmetric
  sweeps pre-rotating the profile; the no-kernel fallback lathes rotated profile rings
  with sweep-end caps, oriented against the rotated profile centroid (correct for
  washer profiles that don't contain the axis). Scriptable as
  `bearcad.revolve{ polygon|circles =, axis = "x"|"y"|"z"|{line = i}, angle =,
  symmetric =, body = "new"|"add"|"cut", bodies = {..} }`, and interactive revolves
  replay to the command log as the same call. Limitation: the profile must not cross its
  axis.
- **Loft** *(implemented)* — blend a solid through two or more closed cross-section
  profiles (circles or line loops) on different planes. The **Loft** toolbar tool collects
  sections by clicking profiles in the viewport (a click on a loop's line picks the whole
  loop; clicking a picked section removes it); hovering a pickable profile highlights the
  whole closed loop under the cursor, and each picked section shows the selection highlight
  on its sketch entities, so the collected set is visible in 3D as well as in the pane. The
  picked set also shows in the context pane's
  generic selection picker (§6.4-style rows with per-row remove and clear-all), seeded from
  any profiles already selected when the tool is chosen. Once two or more sections are
  picked, a translucent **ghost preview** of the blended solid renders live and updates as
  sections are added or removed (#203), meshed exactly the way a commit would. **Enter**
  (with ≥ 2 sections)
  commits: sections are ordered along the loft's principal direction (so pick order doesn't
  tangle the blend), and a new `Loft` feature plus its body land under a single undo marker.
  The mesh is a ruled loft rebuilt parametrically from the live profiles: each section
  boundary is resampled to a common ring size, rings are aligned (consistent winding,
  twist-minimizing start offset) and stitched with wall quads, and the end sections are
  capped — a hand-rolled mesh like the no-kernel edge-treatment fallback; an OCCT
  `ThruSections` BREP loft is a documented follow-up. Scriptable as
  `bearcad.loft{ circles = {i, ...}, polygons = {{line, ...}, ...}, name = }` (singular
  `circle`/`polygon` also accepted; each face's sketch is inferred as in `bearcad.extrude`),
  and interactive lofts replay to the command log as the same call. In the Elements pane a
  loft shows as its **own operation node** (`HierarchyNode::Loft`) with its output body nested
  beneath it and its cross-section **sketches** feeding it as Graph-view dependency edges
  (#252) — previously the loft body surfaced as a bare top-level element with no sign of what
  produced it.
- **Pattern** — linear and circular patterns of features/bodies.

Each operation is exposed identically through the GUI, the action DAG, and the scripting
API (§8). Failures from the kernel (e.g. a fillet that can't be applied) must surface as a
recoverable error on the relevant feature node, not a crash.

### 3.6 Technical drawings (#180)

A **technical drawing** is a black-on-white sheet for print/PDF output. A document holds any
number of them; each references bodies but produces no solid geometry, so drawings live
outside the shape/undo DAG (undo is snapshot-based, §4.3).

- **Create & manage:** the Elements pane has a **＋ New Drawing** button (and a `Drawing`
  node, with its own icon, per drawing). Right-clicking a drawing — or clicking its row —
  **opens it** in the drawing pane, which takes over the central area. The **editor** is
  white-on-black to match the app's dark-mode aesthetic (#254); **export** inverts back to
  black ink on a white sheet.
- **Pop-out window (#254/#276):** the drawing pane's **⇱ Open in window** button moves the
  drawing into its **own OS window** (an eframe *immediate* viewport, so its render can borrow
  app state), handing the central area back to the 3D view — so the model and a drawing are
  visible at once. Closing the window (or `Esc`) dismisses it. Native only.
- **Workbenches (#254/#271/#272):** opening a drawing switches to the **Drawing workbench**,
  whose toolbar shows **Back, Select, Add view, Aligned view, Dimension, Text** (#295: no Move
  tool; the Select tool drags projections directly, #293 — and **only** the Select tool: with
  any other tool, e.g. Dimension, dragging across a card moves nothing, #374). Entering the
  workbench with any
  other tool active drops back to Select. A **Back button** (left of Select, #318) returns to
  the model; **Escape no longer exits** the workbench (it cancels in-progress tool actions).
  Clicking anywhere on a projection card selects it (not just the caption, #316), and a
  hovered card gets a highlight border. The model-only **Selection** element picker is hidden
  here (#317), since projections and annotations have their own selection state.
- **Aligned-projection tool (#296):** the workbench's **Aligned view** tool (projection icon;
  tool name `drawing_align`/`aligned_view`) derives an orthographic child from an existing
  projection. It picks a **base view** to align to: a single selected projection is used
  automatically on entering the tool, otherwise it's chosen from the tool's **Base view** element
  picker in the context pane or by clicking a projection on the page (#365). Then move the mouse —
  the direction from the base picks the child
  (down → Bottom, up → Top, right → Right, left → Left for a Front parent, by glass-box
  unfolding: `drawing::aligned_child_orientation`), previewed as a ghost card with the derived
  orientation labelled; click commits `AddAlignedDrawingView`. The child stays **lined up**
  with the parent along their shared axis — placed above/below it shares the horizontal
  position (`pos_x`), left/right shares the vertical (`pos_y`) — enforced by
  `drawing::resolved_view_pos`, which resolves an aligned child's shared coordinate from its
  parent (recursively, so chains stay consistent) in both the editor and export. Dragging a
  child only slides it along its free axis; moving the parent carries its children. Alignment lines
  up the **projected geometry**, not just the cards (#364): a child inherits the base's auto-fit
  scale (`drawing::view_autofit_scale`) and centres its geometry on the base along the shared
  projected axis (`drawing::view_render_center`), so the part's edges register across the group in
  both the editor and exports. A child **inherits the parent's scale** and can't change it
  (`drawing::resolved_view_scale`), and its
  orientation **defaults** to the base+direction derivation but can be **adjusted within the ring
  of angles that keep the shared edge** (#367): the view editor shows the same **orientation
  bear** as a normal view (#370), restricted to that ring — only its faces/edges hover-highlight
  and click; everything else is inert (`show_orientation_picker`'s `allowed` set). The ring is
  `drawing::aligned_inline_orientations` — the straight-on faces *and* the diagonal
  edge views sharing the fold axis, excluding the base's own orientation and anything using the
  perpendicular pole, so a Front base with a right child offers right/back/left and the four
  vertical-edge views, never top/bottom. Picking one rolls the projection about the shared edge:
  `resolved_view_axes` maps the chosen orientation into the parent's unfolded frame, so it renders
  the new angle while staying lined up. Crucially, an aligned child **renders with the unfolded
  basis** (`drawing::resolved_view_axes`), not a fixed canonical orientation — for a non-Front base
  the unfolded view is *rotated*, so **all four directions work from any base** (#351): a Top base
  gives Front below, Back above, and rotated Right/Left to the sides. Every projection site (editor,
  export, silhouette, dimension candidates) uses `resolved_view_axes` so the rotation is consistent;
  the child's stored `orientation` is just the nearest face for its label. All six straight-on bases
  offer all four directions; an isometric/edge/corner parent has no aligned children. Scriptable:
  `bearcad.drawing_align_view{ drawing, parent, dir = "below"/"above"/"right"/"left", pos? }`.
- **Text annotations (#312):** the **Text** tool (the same tool, `T` shortcut, brought into
  the Drawing workbench) places **free text on the page** — click for a growing single-line
  box, drag a rectangle for one that word-wraps to that width. Annotations
  (`Drawing::annotations`, `DrawingAnnotation`) store page-fraction position and a
  page-height-fraction size so they hold across page-size changes; they render as plain text
  (not glyph outlines) wrapped by egui in the editor and by `drawing::wrap_text_lines` in the
  exports. The **Select** tool clicks to select and drags to move them; the context pane shows
  a multi-line editor + Remove (`Action::AddDrawingAnnotation`/`EditDrawingAnnotationText`/
  `MoveDrawingAnnotation`/`RemoveDrawingAnnotation`; `AppState::selected_drawing_annotation`).
  **Double-clicking** a textbox on the page focuses that editor with the text selected (#379,
  `ContextPaneState::focus_annotation_field`), so typing immediately replaces it.
  Scriptable: `bearcad.drawing_text{ drawing, text, x, y, wrap? }`. While the **Text** tool is
  active, the context pane belongs to placing/editing text: a projection that happens to still be
  selected does **not** show its view editor (#329), and the **Default units** section is hidden
  (#330) — both reappear under the Select/Dimension tools.
- **Variable interpolation in text (#338):** both drawing annotations and sketch text may embed
  `{expression}` fields that resolve against the document's parameters
  (`value::interpolate_text`). A field evaluates any length/angle expression — a bare parameter
  (`{foo}`), or arithmetic (`{foo + 3in}`) — and substitutes the value formatted in the
  document's default unit; `{{`/`}}` are literal braces; an unknown variable or syntax error
  renders as `#NA`. Drawing annotations interpolate at render time (editor and exports), so the
  context-pane editor still shows the raw template. Sketch text bakes its glyph outlines from the
  interpolated string while storing the raw template, and `recompute_document_geometry`
  re-bakes every sketch text (`parameters::rebake_sketch_texts`) when a parameter changes, so the
  text follows edits like any other parametric feature. Both text editors offer **parameter-name
  tab completion scoped to `{…}` fields** (`expression_input::interp_autocomplete_*`): typing a
  name inside braces shows the parameter dropdown (Tab/Space/arrows to accept), but ordinary words
  of prose don't trigger it.
- **Add-view tool (#289):** the workbench's **Add view** tool (＋ icon; tool name
  `drawing_add`) replaces the old inline "Add view:" combo row. With it active, clicking a
  **body or sketch** in the Elements pane drops a projection of it onto the page and selects
  it; the **context pane** then shows the view editor — source label, **orientation**
  dropdown, **Scale** field, and **Remove view** — and the card can be dragged into place.
  Clicking any existing card (any tool) selects it and opens the same editor (selected card
  gets an accent border; `AppState::selected_drawing_view`).
- **Drag from the pane (#290):** with a drawing open, **dragging a body or sketch row** from
  the Elements pane onto the page places a projection at the drop point (the page shows an
  accent border while a compatible drag hovers), selected and ready to configure — the same
  result as the Add-view tool. The row's **name and its type icon** are both grab handles
  (#368). Plain clicks on those rows still select as usual.
- **Orientation bear (#315):** a selected view's orientation is chosen with an **interactive
  navigation bear** in the context pane (the same widget as the viewport's HUD bear, replacing
  the dropdown; `view_cube::show_orientation_picker`): drag it to spin, click a face for that
  straight-on view or a corner/edge for the isometric, and — when the widget has focus — the
  numpad picks views (**4** left, **5** front, **6** right, **8** top, **2** bottom, **0**
  back). It drives a local camera and maps the picked `StandardView` to a `DrawingOrientation`.
  Adding a view now selects it, so the bear appears immediately. The **currently-selected view is
  highlighted in blue** on the bear (#323/#340) — a face fill for the six straight-on views, a dot
  on the top-front-right **corner** for Isometric, or the matching **edge** for a diagonal edge
  view (`drawing_orientation_to_cube_pick` → `view_cube::CubePick`). The highlight is drawn
  **unculled** (`draw_selected_pose`), so the chosen face/edge/corner still shows even when it's on
  the far side of the bear, and a glance always tells which way the view looks while spinning.
- **Arbitrary angle — "Use this view" (#345/#366):** the view editor has a **Use this view** button
  immediately below the orientation bear. It sets the projection to whatever the 3D viewport is
  currently showing, stored as an arbitrary `(right, up)` basis (`DrawingOrientation::Free`) taken
  from the live camera (`view_cube::free_basis`, whose sign convention makes a Front camera pose
  reproduce the Front projection exactly). So to get a non-standard angle you orbit the 3D model,
  then click the button. The bear itself only ever picks presets (faces/edges/corners); there is no
  free-spin mode.
- **View styles (#301):** each view renders in one of three styles, picked in the view
  editor: **Visible edges** (hidden lines removed — every feature edge is depth-sampled
  against the body's mesh and only the unoccluded runs stroke), **Wireframe** (every feature
  edge, the default), or **Shaded** (front faces painted back-to-front, greyed by a fixed
  key light, under the visible edges). Sketch views have no solid, so they always draw
  wireframe. The projection logic is `drawing::styled_view_geometry`, shared by the editor
  pane (greys darkened for the dark sheet) and both exports (the `Canvas` trait gained a
  filled-polygon primitive). `Action::SetDrawingViewStyle`.
- **View scale (#300):** each view has a print **Scale** as `page:model` text, e.g. `1:20`
  (1 page mm represents 20 model mm) — any positive numbers work (`2:3`, `10:1`). The field
  only commits text that parses, so an erroneous entry leaves the last valid scale in
  effect; empty returns to **auto-fit** (the default). A set scale draws the projection at
  exactly that size in the editor and both exports, and shows in the card caption
  (`Body 0 — Front (1:20)`). `Action::SetDrawingViewScale`;
  `crate::model::parse_drawing_scale`. The Parameters
  pane **hides by default on entering the Drawing workbench** (#398) but can be re-shown
  from the View menu like anywhere else (#378) — so parameters can be edited (rebuilding the
  model and the open drawing's views) without leaving the drawing — and its pre-drawing
  visibility restores on returning to the model.
- **Aligned projection lines (#377):** an aligned child can draw **two dashed, lightweight
  lines** connecting its silhouette extremes to its base view's across the gap — at the far
  left/right of the pair for an above/below child, the top/bottom for a left/right one —
  toggled by a **Projection lines** checkbox in the child's view editor (stored as
  `DrawingView::align_lines`, `Action::SetDrawingViewAlignLines`; rejected for non-aligned
  views). `drawing::aligned_projection_lines` computes the endpoints in each view's own
  projected space and the editor and both exports map them through the owning view's
  transform, so the lines land exactly on the rendered silhouettes (dashed strokes:
  `stroke-dasharray` in SVG, a `d` dash pattern in PDF). Scriptable:
  `bearcad.drawing_view_align_lines{ drawing, view, show }`.
- **View labels (#372):** each view's caption label ("Body 0 — Front (1:20)") is editable from
  the Select tool's context pane: a **Label checkbox** shows/hides it, a **2×3 position grid**
  places it (top/bottom × left/center/right of the card, `DrawingLabelPos`, default top-left),
  and a **text field** overrides the caption — like any label it may embed `{expression}`
  interpolation fields (#338); clearing the field returns to the automatic caption (the
  field's hint). Stored per view (`label_hidden`/`label_pos`/`label_text`,
  `Action::SetDrawingViewLabel`), honored identically by the editor and both exports.
  Scriptable: `bearcad.drawing_view_label{ drawing, view, hidden?, pos?, text? }` (`pos` is
  `"top-left"`…`"bottom-right"`; `text = ""` resets to automatic).
- **Elements-pane filter (#254/#275):** a **Filter** button (funnel icon, #291) at the bottom
  of the Elements pane expands into per-type show/hide toggles (planes, sketches, sketch geometry, bodies,
  operations, images, drawings, **drawing components** #381). The toggles render as
  **icon-group buttons** (#382, `icons::selectable_icon_group`), stacked vertically (#389):
  each category shows the icons of the element types it covers (Operations =
  Extrude+Revolve+Combine; hover for the category name), dimmed while off. "Sketch
  components" and "Drawing components" use dedicated icons — the parent's icon beside a
  shared two-squares-two-lines child motif — and Images has its own picture icon. Hiding a type prunes those nodes but promotes their kept
  children (hiding "Operations" still shows the result bodies, un-nested). The Drawing
  workbench defaults the filter to sketches + bodies + **drawings** (#333), so the open drawing's
  **projections, text notes, and dimensions** appear in the pane. In the **Model** workbench
  those drawing components are hidden by default (#381) — the drawing rows themselves stay,
  and the "Drawing components" toggle brings the page details back. Each drawing's text notes are
  `HierarchyNode::DrawingAnnotation` children (Text icon) alongside its `DrawingProjection`
  children, and each projection's shown dimensions are `DrawingDimension` children nested under it
  (Dimension icon, labelled by their length, #341); all are display-only leaves whose row click
  opens the drawing and selects the element.
- **Page dimensions (#254/#273):** each drawing has a page size and margin (`page_width_mm`,
  `page_height_mm`, `margin_mm`), defaulting to a **landscape US-Letter** sheet (11 × 8.5 in)
  with **0.5 in** margins. The editor draws the page outline and margin at the page's aspect
  ratio; right-clicking the sheet background opens a page-dimensions editor (in inches, with
  Landscape/Portrait Letter presets), via `Action::SetDrawingPage`. Scriptable (#406):
  `bearcad.drawing_page{ drawing, width?, height?, margin? }` in millimetres — omitted keys
  keep the drawing's current value. The sheet **pans** (drag
  the empty background) and **zooms** (scroll, about the cursor) like the 3D viewport but never
  rotates; **`Z`** (or the Zoom tool) resets it fit-to-pane, and opening a drawing starts fit.
- **Placed views (#254/#274):** each view carries a page position (`pos_x`, `pos_y`, page
  fraction). Views render as cards **on the page** and are **dragged** by their caption strip
  (`Action::MoveDrawingView`, non-undoable per-frame). Right-clicking a card picks its
  **projection orientation** (`Action::SetDrawingViewOrientation`) or removes it. New views
  cascade from the page centre so they don't fully stack. A **body or sketch** can be added
  from the Elements pane's right-click **Add to drawing** while the drawing is open; a
  **sketch** view (`DrawingView::sketch`, #278) projects that sketch's line/circle geometry
  instead of a body's mesh edges (both editor and export share `drawing_view_world_edges`).
  Scriptable too (#403): `bearcad.drawing_view{ drawing, sketch = i, orientation? }` — the
  call takes exactly one of `body` or `sketch`.
- **Projection elements (#254/#281):** each placed view shows in the Elements pane as a
  **projection** node (`HierarchyNode::DrawingProjection`, its own icon) nested **under its
  drawing**. In the Graph view it also draws a dashed **dependency edge** to its source body —
  a second input beyond its drawing parent (the full multi-parent relationship lands with the
  element graph, #252). It's a display-only leaf (no `SceneElement`).
- **Views:** a drawing collects **views**, each a chosen body shown in one orientation — the
  six straight-on directions (Front/Back/Left/Right/Top/Bottom), an **Isometric** three-quarter
  view, one of the twelve **diagonal edge views** (`DrawingOrientation::Edge(EdgeView)`, #339)
  that look square at a cube edge (Front-Right, Front-Top, …), or one of the eight **corner
  views** (`DrawingOrientation::Corner(CornerView)`, #344) that look at a cube corner. Clicking an
  edge or corner on the orientation bear picks that specific view (#344) — not a fixed isometric.
  An edge/corner view's basis is derived from its two/three faces: the camera looks along their
  averaged into-page direction with world +Z up (`drawing::view_axes`, orthonormal via
  Gram-Schmidt). Each view renders as a black wireframe of the body's feature edges,
  orthographically/isometrically projected and auto-fit into its cell; views sit wherever
  they were placed on the page and are added/removed from the drawing pane.
- **Curves (#313/#319):** tessellated circles (a cylinder rim, an extruded-circle boundary)
  are **detected in world space** (`drawing::classify_world_circles`: clean degree-2 cycles
  that fit a planar circle) and **projected per view** (`project_world_circle`): **round** when
  the circle faces the viewer (a real SVG `<circle>` / PDF Bézier-arc, not a polygon), or a
  **foreshortened diameter line** when edge-on. Edge-on is decided from the true projected
  ellipse — minor semi-axis `r·|normal·view|` — so a diagonal edge view (e.g. Front-Right) of a
  cylinder correctly draws its caps as lines, not floating circles (#369). Either way it carries a **single diameter
  dimension** (`Ø…`, using the WinAnsi-safe Ø glyph, #320), and its segments are excluded from
  the straight-edge strokes and the length-dimension set. A **face-on** circle gets a **horizontal**
  diameter line across it (#397) with the value on it — the label is **draggable up/down**
  (Select or Dimension tool), stored as a per-circle `circle_dim_offsets` override
  (`Action::SetDrawingCircleDimOffset`, keyed by the quantized world centre like
  `dimensioned_circles`); an **edge-on** circle (which looks like a plain line) gets a
  **normal linear dimension** — extension lines, an offset dimension line with arrowheads, the
  value running along it (#320) — since it reads as a length, and its label drag slides the
  whole dimension line nearer/further like an edge dimension's does. **Silhouette edges (#319):** a body view also strokes the
  view-dependent silhouette (`solid_mesh_silhouette_edges`: edges where the two adjacent faces
  face opposite ways), so a cylinder's straight sides show. They're kept out of **circle
  detection** so the rims stay clean circles, but they **are dimensionable** (#334): the
  dimensioning candidate set is `drawing::drawing_view_dimensionable_edges` (crease edges plus
  silhouette edges, deduped), so the **length** of a smooth extrusion — which has no crease edge
  down its side — can be dimensioned like any straight edge, in the editor and both exports.
- **Dimensions:** a newly added projection starts with **no dimensions shown** (#331). The
  projection's context pane has **Show all dimensions** and **Hide all dimensions** buttons
  (`Action::SetAllDrawingDimensions`, `DrawingViewEdit::SetAllDimensions`): *Show all* populates
  the deduped, staggered default set (every edge's length dimension — except edges pointing
  straight into the page, which project to a point and carry no meaningful in-view length (#294),
  and except tessellated-circle segments, which get a single diameter dimension instead (#313));
  *Hide all* clears them. A detected circle's **diameter dimension is toggleable too** (#342),
  tracked per view in `dimensioned_circles` (keyed by the circle's quantized world centre): it
  starts hidden like the rest, *Show all* reveals every circle's Ø and *Hide all* clears them —
  the circle **outline** always draws, only its Ø dimension is gated. User-added angle dimensions
  are left untouched by both. Individual edges
  are still toggled with the Dimension tool (or `bearcad.drawing_dimension`), and so are
  individual circles (#373): with the Dimension tool, hovering a detected circle's outline —
  the round outline face-on, or the foreshortened line of a side-viewed circle — highlights
  it, and a click toggles its Ø (`Action::ToggleDrawingCircleDimension`, scriptable as
  `bearcad.drawing_circle_dimension{ drawing, view, center = {x,y,z} }`; circle-tessellation
  segments are excluded from the edge pick so the circle itself is the target). Length dimensions
  render as proper **architectural
  dimension lines** (#294): two extension lines off the edge, a dimension line offset outward
  (on the side away from the geometry centroid) with **arrowheads** at each end, and the
  measurement centred on it — in the editor and both exports, from one shared
  `drawing::dimension_line_geometry`. Dimension lines, their extension lines, and diameter lines
  are stroked **thinner than the model outline** (#327): the projected model edges and detected
  circles use `drawing::MODEL_STROKE` and the annotations use the lighter `drawing::DIM_STROKE`,
  so the part reads as the primary geometry and the dimensions sit visually beneath it (editor
  and exports share both constants). The default dimension set is **deduped by projected
  segment** so coincident front/back edges (a box's bottom edge seen from the front) get one
  dimension, not two stacked on the same line; the surviving representative is chosen
  deterministically (smallest world key), so reopening a drawing dimensions the same edge every
  time. To keep the initial set legible, parallel dimensions whose lines would land at the same
  distance and whose spans overlap are pushed out onto successive **tiers**, the way CAD stacks
  parallel dimensions, so no number label overlaps another dimension line or label (#321;
  `drawing::plan_dimension_tiers`, applied as `dimension_offsets` when the projection is
  created). With the **Dimension tool** active (#277), the edge
  nearest the cursor **hovers** (highlighted) to show a click will toggle it; clicking toggles
  its dimension. The hit-test also covers a shown dimension's **own line/label** (#324), so an
  existing dimension can be toggled off by hovering its dimension line, not just the model edge
  (`dim_line_screen` mirrors the render geometry in the hover pass). **Shift+click** two edges
  toggles the **angle** between them (drawn at their corner). A dimension **label is draggable** (Select or Dimension tool) to slide the whole
  dimension line further from or closer to the edge; the offset is stored per view as a
  `dimension_offsets` override (`Action::SetDrawingDimensionOffset`), cleared when the
  dimension is hidden. Hovering a dimension **highlights** it — the
  dimension line is accented and its label outlined (#326) — so it's obvious which dimension a
  drag will move: with the **Select tool** via its label, and with the **Dimension tool** also
  when hovering its line or its model edge (#375, where a click toggles it). With the Select
  tool, **clicking** a dimension selects it (`AppState::selected_drawing_dimension`,
  staying highlighted). **Delete/Backspace** removes the selected drawing element (#336): a
  projection (`RemoveDrawingView`), a text note (`RemoveDrawingAnnotation`), or a dimension
  (hidden via `ToggleDrawingDimension`); the handler skips when a text field wants keyboard input
  so Backspace still edits note text. The open drawing's **projections, text notes, and
  dimensions are listed in the Elements pane** (#328/#341), nested like a sketch's geometry —
  projections and text under the drawing, each projection's dimensions under it
  (`HierarchyNode::DrawingProjection`/`DrawingAnnotation`/`DrawingDimension`). Clicking a row
  opens the drawing and **selects** that element (its row shows the selected style and its context
  editor opens); hovering a row **highlights** the element on the page
  (`AppState::hovered_drawing_element`). Clicking **blank page space** with the Select tool
  **deselects everything** (#346) — the page-background interact reports the click only when no
  card/note/dimension consumed it. The label **runs along its dimension line**, always reading
  **left-to-right or bottom-to-top** (#322; `drawing::readable_text_angle` normalizes the angle
  into `[-90°, 90°)`, so a downward vertical reads upward and a down-to-the-right slope reads
  top-left → bottom-right); when the line is too short for the text, the label is placed just
  past the line's end horizontally instead (#314; `drawing::dimension_label_layout`, rendered
  with rotated text via egui `TextShape` in the editor and SVG `rotate()` / a PDF text matrix
  in the exports). All dimensions are keyed to the edges' quantized world endpoints (a geometry
  identity that survives rebuilds), stored per view.
- **Title (#335):** a new drawing arrives with its **title as a normal text annotation**
  (defaulting to the drawing's name, or `Drawing N`), placed in the top-left margin. It is an
  ordinary note — draggable, editable, and deletable like any other — so it appears identically
  in the WYSIWYG editor and both exports. The exporter no longer stamps its own title into the
  top margin (that never showed in the editor).
- **Export:** a drawing exports to a self-contained black-on-white vector document (title
  annotation, view captions, projected edges, dimensions) as either a single-page **PDF** or an **SVG**
  (which also prints to PDF through any browser/OS print dialog). Exports show only the
  projection and its caption — **no grey card border** (#337); that rectangle is an editor-only
  affordance for selecting and dragging a view. Exports are **WYSIWYG**
  (#297): each view lands at its placed page position, and the exported page **is the
  drawing's configured page** (#298) — the PDF MediaBox is `page_width_mm × page_height_mm`
  in points, landscape US-Letter (792 × 612 pt) by default. The editor lays out
  **proportionally to the export** (#376): cards are the exact page fraction (no pixel
  clamp), card padding and text sizes scale with the on-screen page (11 pt dimension/caption
  text mapped through the page's px-per-point), and the same width estimate drives the
  "does the label fit along its line" decision — so a dimension label that runs along its
  line in the editor does in the PDF too. A detected circle's plane normal is
  **sign-canonicalized** in `classify_world_circles`, since an arbitrary sign flipped which
  end of an edge-on diameter line the label hung past between the editor's and the export's
  own classification passes. Both backends share the same
  layout through a `Canvas` trait in `src/drawing.rs`; the PDF is hand-rolled (no dependency),
  so it works identically on native and web (download in the browser). Export is a single
  **Export icon** in the drawing workbench toolbar (#348) whose popup picks **SVG** or **PDF**.
- **Scripting:** `bearcad.drawing{ name? }` creates a drawing (returning its index),
  `bearcad.drawing_view{ drawing, body, orientation? }` adds a view (`orientation` is
  `"front"`/`"top"`/`"iso"`/…, default front),
  `bearcad.drawing_dimension{ drawing, view, a = {x,y,z}, b = {x,y,z} }` toggles an edge's
  length dimension, `bearcad.drawing_circle_dimension{ drawing, view, center = {x,y,z} }`
  toggles a detected circle's diameter dimension (#373),
  `bearcad.drawing_view_label{ drawing, view, hidden?, pos?, text? }` edits a view's caption
  label (#372),
  `bearcad.drawing_view_align_lines{ drawing, view, show }` toggles an aligned child's dashed
  projection lines (#377),
  `bearcad.drawing_angle{ drawing, view, edge1 = { a, b }, edge2 = { a, b } }`
  toggles the angle between two edges, and `bearcad.export_drawing_pdf{ drawing, path }` /
  `bearcad.export_drawing_svg{ drawing, path }` write the PDF/SVG. `bearcad.count("drawing")`
  counts drawings.

---

## 4. Action DAG (history & non-linear undo)

BearCAD replaces Fusion's linear timeline with a **directed acyclic graph of actions**. This
is the source of truth for the model; geometry is derived from it (see §4.4).

### 4.1 Nodes and edges
- A **node** is an action: creating/editing a feature, creating/editing a parameter,
  creating a component, defining a joint, etc. **Parameter creation and every parameter
  change are nodes**, exactly like geometric features.
- A **directed edge** `A → B` means *B depends on A* — i.e. B consumes an output of A
  (a body, a face/edge reference, a parameter value, a sketch, etc.). Dependencies are
  derived from real data references, not from authoring order.
- The graph is acyclic. Attempting an edit that would create a cycle is rejected.

### 4.2 Per-component subgraphs
- Each component has its own connected subgraph. Two independent components show two
  independent graphs. When component C references components A and B, C's subgraph shows
  dependency edges into A's and B's outputs.

### 4.3 Undo / redo / time travel
- Undo is **infinite and persistent** — it survives closing and reopening the file
  (the full history lives in the `.bearcad`; see §7).
- *Implemented today* (pre-DAG): undo is **checkpoint-based** (#194). `AppState::apply`
  snapshots the whole document *before* each mutating user action; **Undo last** restores
  the most recent snapshot and **Redo** (#193) re-applies it. Because a snapshot reinstates
  the exact prior document, a whole gesture (a rectangle's four lines plus constraints, or a
  fillet's truncate-and-bridge) reverts in one correct step — no per-entry reversal to get
  wrong. New/Open/Clear reset the history (undo never crosses into a different document); a
  fresh action clears the redo stack. This history is **session-only** so far (the snapshots
  aren't persisted), unlike the persistent DAG this section targets.
- The history is a **commit graph**: each user-visible change creates a new state. Undo
  moves to the parent state; redo moves forward. Because history is a graph (branches
  allowed) rather than a line, redo may present multiple forward branches; the UI MUST
  let the user choose among them.
- Editing the *value* of an existing feature/parameter does **not** destroy downstream
  work — it re-evaluates dependents (§4.4). This is the key difference from a linear
  timeline: rolling "back" to edit a node does not discard later, independent nodes.

### 4.4 Evaluation, caching & recompute
- The **action DAG is the source of truth**; evaluated geometry is **derived and cached**.
  Evaluated geometry **is persisted in the `.bearcad`** so files open fast — open should
  display cached geometry without a full rebuild. Speed is a priority for this app.
- Each DAG node caches its evaluated output (per-node BREP and/or tessellation; granularity
  **TBD**, but at least per-feature). Editing a node invalidates only that node and its
  transitive dependents (dirty-propagation); unaffected branches keep their cache and are
  not recomputed. The same in-memory cache is used during a session.
- **Cache validity** is tracked per node by a fingerprint of (the node's inputs/payload +
  its upstream dependencies' fingerprints + the **OCCT version**). On open, any node whose
  fingerprint no longer matches its cached entry is recomputed; everything else loads from
  cache. This keeps cached geometry correct across edits and across OCCT upgrades.
- Because the DAG fully determines geometry, the cache is always reconstructible: a
  "force rebuild" command (and CLI flag, §9) discards the cache and replays the DAG.
- Evaluation must be **deterministic** given the same DAG and the same OCCT version, so
  that a rebuild, a headless CLI run, and the GUI all agree. Record the OCCT version in
  the file (§7).

### 4.5 Topological references (naming)
- Feature inputs that reference faces/edges (e.g. "fillet this edge") must use **stable
  topological identifiers**, not raw OCCT indices, so that upstream edits don't silently
  re-target downstream features. Define a persistent-naming scheme that maps user/feature
  references to topology across recomputes. (Algorithm: **TBD** — candidate: hash of
  generating feature + geometric signature. This is a known-hard CAD problem and must be
  designed explicitly.)

---

## 5. Parameters, expressions & units

### 5.1 Parameters
- Parameters are a first-class feature with their own pane in the GUI.
- Parameters exist at **document** and **component** scope; component parameters may
  shadow document ones.
- A parameter has: name, expression (text), evaluated value, unit, and optional
  description.
- Parameter changes are DAG nodes (§4.1).
- When a parameter's name or value field is focused in the Parameters pane, the Elements
  pane highlights every element that uses that parameter (the dimensions referencing it and
  the geometry they drive), dimming the rest.
- Each parameter row has a muted-red **✕** delete button (`Action::DeleteParameter`, #270).

#### 5.1.1 Inline parameter creation
- In **any value input** (GUI field or scripting), prefixing the entry with
  `name=` creates a new parameter on the spot and uses it for that input. For example,
  typing `width=20mm` in an extrude-distance field creates a parameter `width = 20mm` and
  binds the field to it (the field now holds the expression `width`). This mirrors
  Autodesk Fusion's inline-parameter behavior.
- The assignment target follows the normal scoping rules (§5.1); creation is a DAG node
  like any other parameter creation.
- If `name` already exists, the input must either **reuse** it (binding the field to the
  existing parameter) or, if a value is also supplied, treat `name=value` as redefining
  that parameter — the UI must make which one is happening unambiguous (e.g. reuse on
  bare `name=`, redefine on `name=value`, with a clear indicator). Reject names that
  collide with reserved words or that would create an expression cycle (§4.1).

#### 5.1.2 Derived parameters (#432)
- A parameter may be **driven by a measurement** (`Parameter::source`,
  `model::ParameterSource`): a line's length (`LineLength`, the original #measured flow),
  the world-space distance between two points (`PointDistance`, any two
  `ConstraintPoint`s — 2D or 3D), the distance between two **parallel** lines
  (`LineDistance`), or the angle between two non-parallel **same-sketch** lines
  (`LineAngle`, stored in degrees).
- The Parameters pane classifies the current selection
  (`parameters::derived_source_from_selection`) and, when it measures something, shows
  the value it would capture beside a **Derive from selection** button
  (`Action::CreateDerivedParameter` → `parameters::add_derived_parameter`; duplicate
  measurements are refused).
- Derived expressions are **read-only** (names stay editable) and re-sync from geometry
  on every rebuild (`sync_computed_parameters` → `derived_source_value`; lengths format
  in the document length unit, angles in the document angle unit).
- Focusing a derived parameter's row highlights its defining elements
  (`derived_source_elements` feeds `elements_using_parameter`).
- Scriptable: `bearcad.derive_parameter{ kind = "line_length"|"point_distance"|
  "line_distance"|"line_angle", a =, b =, name = }`.

### 5.2 Expressions
- **Any input that accepts a value accepts an expression**, e.g. `1 + 2 + lengthOfThing / 2`.
- Expressions may reference parameters and other values by name.
- Expressions support `+ - * /`, parentheses, and a standard math function library
  (trig, sqrt, min/max, etc. — full list **TBD**). **Implemented today (#431/#445):** `max`,
  `min`, `abs`, `floor`, `ceil` (alias `ceiling`), and `round` in both the length and
  angle parsers (`value::apply_builtin_function`).
  `max`/`min` take one or more arguments or a square-bracket array (`max([a, b, c])`,
  which flattens into the argument list — mixing works, `max([1, 2], 10)`); `abs` takes
  exactly one. Arguments are full expressions (units, parameters, nesting compose);
  malformed calls fail the whole expression rather than half-parsing.
- The **raw expression text is stored verbatim** so the user sees and can edit exactly
  what they typed (e.g. `3mm + 2in`), alongside the evaluated value (§7).
- **Variable-name autocomplete**: while typing an identifier in an expression field, a
  dropdown offers matching parameter names (best match on top). Arrow keys move the
  highlight; **Space** or **Tab** completes the highlighted name and keeps editing;
  **Enter** completes the highlighted name *and* commits the field in a single keystroke.

### 5.3 Units
- Strong unit support with mixed units. `3mm + 2in` is valid and evaluates correctly.
- Every component has **default units**; a bare number inherits the contextually relevant
  default unit.
- Units are dimension-checked: adding a length to an angle is an error.
- Supported unit families for v1: length (mm, cm, m, in, ft), angle (deg, rad). Extend as
  needed.
- Internal canonical storage units: **TBD** (recommend millimeters for length, radians for
  angle), but the stored expression text is always preserved.
- **Default-unit picker (#52):** the Context pane lets the user choose default length/angle
  units. With nothing selected, it edits the document-wide defaults
  (`bearcad.set_units{ length = "mm", angle = "deg" }`). With exactly one **sketch** selected,
  it edits that sketch's own override instead, offering a "Follow document" entry per axis
  (length and angle can be overridden independently) that clears back to inheriting the
  document default (`bearcad.set_units{ sketch = N, length = "in" }`; omitting an axis on a
  sketch call means "follow document" for that axis, since Lua can't distinguish an omitted
  table field from an explicit `nil`). Any other selection hides the picker. **Scope note
  (#85):** dimension labels and the Elements pane now format geometry in the effective unit
  (document default, or the owning sketch's override) instead of always showing mm/degrees.
  This does **not** change the bare-number parsing fallback, which is still hardcoded to
  mm/degrees (per above) — internal storage stays mm/radians regardless of display unit.

---

## 6. Constraints

BearCAD has a geometric **constraint solver** supporting both 2D (sketch) and 3D constraints,
modeled on SolveSpace (https://solvespace.com).

### 6.0 Constraint tool (implemented subset)

- **Tool:** Constraint, shortcut **`C`**. Distance/dimensional constraints remain on the
  **Dimension** tool (`D`).
- **Angle dimensions — placement phase:** pressing `D` with two non-parallel lines selected
  (and no existing angle constraint between them) does not commit a value immediately.
  Instead the angle preview follows the mouse: two lines crossing have two distinct angle
  magnitudes (supplementary, one on each pair of opposite wedges), and whichever wedge
  encloses the cursor is the one previewed. Clicking commits that choice and moves to typing
  the value, the same as other dimensions (#40).
- **Selection:** Sketch points (line endpoints — including a rectangle's corners — and circle
  centres), lines (a rectangle's four edges are plain lines), and circles are selectable in the
  viewport. Point picks take precedence near vertices within the point pick tolerance.
- **Elements-pane hover → viewport highlight (#161):** hovering any row in the Elements
  pane (List or Graph view) highlights that element in the 3D viewport using the
  standard hover color: sketch entities get their usual pick highlight, a hovered sketch
  row highlights all of its entities, a construction plane its fill, and a body or
  extrusion its **aura** tinted in the hover color. Drawn depth-test-disabled like other
  pick highlights (#153).
- **3D body sub-element selection (#156):** outside sketch mode, the Select tool can select
  a body's **edges and vertices** (the same feature edges/corners the hover highlight shows,
  #144), not just sketch entities. Shift/⌘-click multi-selects them like any other element.
  Their selection identity is the quantized geometry (not a stable topological name): if a
  rebuild moves the edge, the selection simply drops — acceptable for ephemeral, never-
  persisted selection state. Selected body edges/vertices draw depth-test-disabled like
  their hover highlights (#153).
- **Element picker for the Select tool (#202/#213):** while the Select tool is active the
  context pane shows the unified **element picker** — a focusable, combo-box-style input that
  is the single, consistent way every tool gathers the elements it operates on. Collapsed it
  reads like a text input: a **generic empty state** (#388) — the count (`0`, or `0/1` for a
  single-select picker) beside dimmed icons of the element kinds this picker can take (no
  per-tool placeholder prose) — otherwise a compact
  `N ⟨icon⟩` summary per element kind (e.g. `2 ⟨line⟩ · 1 ⟨body⟩`; a single-select picker
  reads `1/1`). Clicking it opens a popup
  listing each picked element (kind icon + name) with a per-row remove button and a clear-all.
  The Select tool's instance is configured to accept **every** element kind and is
  **always shown and always focused** (it never blurs). Suppressed only while a draw
  construction owns the pane. Each picker instance is configured with: the subset of element
  kinds it accepts (planes, lines, circles, vertices, edges, bodies, constraints, operations —
  and, for operations, which sub-kinds), a pick limit (a whole number or unlimited), and an
  optional override of the selected-element highlight color (defaulting to the theme selection
  color). The Select and Constraint tools mirror the live selection; the construction tools
  (Combine, Move, Repeat, Slice, Revolve-cut, Loft, Chamfer/Fillet) each present their own
  in-progress picked set through the same control — with the currently-active picker focused
  (a tool with several, e.g. Combine's A/B sides or Slice's bodies/cutters, switches which is
  focused when you click it). Whatever a picker holds is **styled as selected in the viewport**
  while the tool is active (folded into the scene's highlight set, not the persistent
  selection). While a body-set tool (Combine/Move/Repeat/Slice) is active, the **body under
  the cursor hover-highlights** as selectable — the same whole-body resolution the click uses
  (#227).
- **Whole-body vs. sub-element picking (#218):** a viewport click picks a **whole body** only
  when the focused picker's accepted types exclude edges, faces, and vertices — so the
  body-set tools (Move/Repeat/Slice/Combine, Revolve cut), whose pickers accept only bodies,
  select a whole body by clicking anywhere on it (edge, corner, or flat face); the Select tool,
  which accepts sub-elements, picks the edge/vertex/face instead. Regardless of that, a body
  **clicked in the Elements pane** (or otherwise selected) always feeds the active body-set
  tool's picker — so you can gather bodies from the pane even for tools where the viewport is
  picking sub-elements.
- **Fade descendants while editing (#260):** while an operation is being edited (an extrusion,
  a Move/Combine/Repeat/Slice op, or a revolve), the bodies **downstream** of its outputs
  (`extrude::descendant_bodies`, walked forward through consuming operations) render dimmed and
  translucent, so the edit's ripple effects are de-emphasized. For the spatial gizmo edits —
  extrude distance/faces, a Move transform, a revolve angle — those descendants are **live-updated
  as the gizmo drags**: each frame a scratch clone of the document is meshed with the in-progress
  edit applied (`body_solid_mesh_uncached_pub`, off the main mesh cache so the rest of the scene
  stays warm), and every faded descendant renders that recomputed geometry in the preview style
  instead of its stale committed solid. Edits without a scratch replay (e.g. boolean/slice input
  re-picks) keep the plain fade.
- **No picking through bodies (#155/#265):** while selecting (Select/Constraint tools, picks
  made for a tool such as construction-plane references or dimension targets, and the
  body-set tools Combine/Move/Repeat/Slice/Revolve), geometry hidden **behind** a visible
  body under the cursor is not a pick candidate — clicking a body never selects a line buried
  inside or behind it, and a body-set tool can't pick a body through one in front of it. The
  probe point is the spot on the candidate nearest the cursor, so a partially hidden edge
  stays pickable along its visible stretch; hiding a body (Elements pane) removes it as an
  occluder, restoring the old X-ray behavior deliberately.
- **3D body sub-element hover (#144):** with the Select tool, hovering a 3D body highlights the
  **vertex, edge, or face** under the cursor — in that priority order (a corner beats an edge on
  it, which beats the face they lie on), so it is always clear what a pick would grab. Edges are
  the solid mesh's feature edges (`solid_mesh_unique_edges`, the same crease/boundary edges the
  wireframe draws, so this works for any body — extrusion-sourced, boolean-cut, or imported);
  vertices are the mesh corners; a face is the maximal edge-connected group of coplanar triangles
  (`solid_mesh_coplanar_faces`), so a whole box side or cylinder cap highlights as one face, with
  the nearer face winning when two project onto the cursor. The Chamfer/Fillet tool likewise
  hover-highlights the treatable analytic edge under the cursor before it is clicked.
- **Selected-body fill (#174):** a selected body's solid also fills in a **more saturated
  blue** than the neutral body grey (in every shading mode), so selection reads on the body
  itself, not just its aura outline.
- **Selected-body highlight / aura (#145/#148):** selecting one or more bodies
  — e.g. in the Elements pane — draws a blue **aura** around them: a purely **2D
  screen-space effect**. Selecting an **Extrude** element auras only the solid that
  extrusion created (#154), with the rest of its body treated as non-selected geometry (it
  occludes the outline where it stands in front) — so picking a feature highlights the part,
  not the whole merged body. All selected bodies are rasterized into one projected footprint, and
  the aura is that footprint's outline pushed a few pixels outward (the iso-contour of the
  footprint's screen-space distance field, traced by marching squares, smoothed, and drawn as
  a single solid-color mitered stroke). Consequences of the 2D design, all intentional:
  - The aura is one continuous non-overlapping outline around the union silhouette — no line
    ever crosses a selected body (e.g. behind a boss standing on a selected cube).
  - Multiple selected bodies whose footprints overlap on screen share one outline, and bodies
    **closer than twice the offset join** into a single merged aura.
  - A **non-selected body occludes the aura** where it stands in front of the selected
    silhouette being outlined (depth-compared per contour stretch); a body behind the
    selection does not.
- **Context pane:** While the constraint tool is active, the context pane lists geometric
  constraint types as buttons (text labels for now; icons later), and below them shows the
  unified **element picker** (§7, #213) for the geometry being constrained. The constraint
  picker is configured to accept only constrainable geometry — points, lines, circles, and
  body/face edges — so it rejects bodies, planes, and operations; it mirrors the live
  selection, and removing a row (or Clear-all) deselects that geometry.
  - **Always all types:** every constraint type is **always listed**, in fixed order.
    Types the current selection cannot satisfy (including when nothing is selected) appear
    **disabled/faded**, with a hint beside the button describing what must be selected
    (e.g. `line, line` for Parallel). Buttons are **enabled** only when the selection
    satisfies that constraint.
  - **Shortcuts (#401):** each type has a fixed **digit** shown left of its button, in pane
    order — Parallel `1`, Perpendicular `2`, Equal `3`, Coincident `4`, Midpoint `5`,
    Vertical `6`, Horizontal `7`. Pressing the digit **while the Constraint tool is active**
    applies that constraint if it is currently enabled; the digits do nothing on other tools,
    so they can't collide with global tool keys.
- **Geometric types (v1):**
  - **Parallel** — `line`, `line`
  - **Perpendicular** — `line`, `line`
  - **Equal** — `line`, `line` (the two edges are constrained to equal length; a rectangle's
    edges are plain lines). See #47.
  - **Coincident** — `point`, `point`; `point`, `line`; `point`, `circle` (point on the
    circle's perimeter); `point`, `origin` (pins the point to the origin); or `line`, `line`
    (the two lines are made **collinear** — each endpoint of one is held on the other's carrier).
    A `point`/`line` operand may be the sketch's own face's vertex/edge (#26/#27, see §3.1) — or
    the origin/origin axes — picked the same way as any other sketch point/line.
  - **Midpoint** — `point`, `line`
  - **Vertical** — `line`
  - **Horizontal** — `line`
- **Redundant-constraint cleanup:** when a point already constrained coincident with a line
  is then constrained to a *specific* point on that same line (one of its endpoints, or its
  midpoint), the earlier generic point-on-line coincidence is removed in favor of the more
  specific constraint.
- **Scripting:** `tool constraint`; `select point line 0 start`; `add_geometric_constraint
  parallel` (uses current selection). Circle tool shortcut is **`O`** (`C` is constraint).

### 6.1 2D sketch constraints (full set)
Coincident, point-on-entity, parallel, perpendicular, horizontal, vertical, tangent,
equal, concentric, symmetric, midpoint, and dimensional constraints (distance, length,
radius/diameter, angle). Dimensional constraints may be driven by parameters/expressions
(§5), so parameters can drive sketch geometry.

### 6.2 3D constraints
SolveSpace-style 3D constraints between 3D entities (points, lines, planes, faces):
coincident, parallel, perpendicular, distance, angle, point-on-plane/line, etc. These
back the assembly joints/mates (§2.3).

### 6.3 Solver
- Sketch constraint systems are solved by **SolveSpace's solver (libslvs)** — the only solver,
  on every target. It is vendored as the `third_party/solvespace` submodule; native builds
  (including `--no-default-features`) compile and statically link it via build.rs, and the web
  build reaches it inside the emscripten kernel module via the same JS bridge as OCCT (a web
  session whose kernel module failed to load gets a hard solve error, not a different solver).
  The mapping (`sketch_solver/slvs.rs`) is one slvs constraint per document constraint
  (handles = document indices, so slvs's failure report *is* the conflict list); pins and
  reference-hold semantics ride libslvs's `dragged`-parameter mechanism. libslvs is not
  thread-safe, so solves are serialized behind a mutex.
- The native equation system (`system.rs`/`residuals.rs`) exists for **analysis only** —
  DOF/rank (`sketch_degrees_of_freedom()`), drag-movability, fully-constrained styling. It
  has no residual evaluation and no numeric solver.
- Rectangles are four constrained lines (eight endpoint variables, closed by coincident
  constraints); circles use centre point + radius variable.
- Interactive drag adds high-weight pin residuals; reference geometry uses softer holds that
  are skipped during drag so the solver can rebalance.
- The UI must report **under-** and **over-constrained** states and indicate conflicting
  constraints. `sketch_degrees_of_freedom()` exposes remaining DOF from Jacobian rank analysis.
- The solver is deterministic for headless/script use (fixed iteration order, fixed LM damping;
  stalled descents retry from deterministically seeded jittered starts).
- Residuals must be commensurately scaled: direction constraints (parallel/perpendicular)
  normalize their cross/dot products by the product of the line lengths so mm-scale point
  equations aren't drowned out, and a length dimension biases its line's start point only at
  weak gauge weight — a dimension must never pin geometry against real constraints.

---

## 7. File format (`.bearcad` / SQLite)

A `.bearcad` is a SQLite database. The schema below is the starting point; refine during
implementation but keep the migration mechanism.

### 7.1 Versioning & migrations
- A `schema_migrations` table records every patch applied, so older files can be upgraded:
  ```sql
  CREATE TABLE schema_migrations (
    id          INTEGER PRIMARY KEY,   -- ordered migration id
    name        TEXT NOT NULL,         -- human-readable migration name
    applied_at  TEXT NOT NULL          -- ISO-8601 timestamp
  );
  ```
- On open, BearCAD applies any migrations whose id is newer than the file's latest applied
  migration. A file from a newer BearCAD than the running binary must be detected and refused
  (or opened read-only) rather than corrupted.
- A `meta` key/value table records app version, **OCCT version used** (for deterministic
  recompute, §4.4), document units defaults, etc.

### 7.2 What is persisted
- **Full action DAG / undo history** — every node and edge, enough to reconstruct all
  states and support infinite persistent undo.
- **Parameters** — name, raw expression text, evaluated value, unit, scope.
- **UI/view state** — pane layout, camera position(s), active theme, and per-document
  custom shortcuts.
- **Cached evaluated geometry** — per-node BREP and/or tessellation blobs plus their
  validity fingerprint (§4.4), so files open fast without a full rebuild. The cache is
  derived data: it can always be regenerated from the DAG and may be discarded
  (force-rebuild) or stripped to shrink a file.

### 7.3 Indicative schema (refine as needed)
```sql
CREATE TABLE meta            (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE components      (id INTEGER PRIMARY KEY, name TEXT, parent_id INTEGER, default_units TEXT);
CREATE TABLE parameters      (id INTEGER PRIMARY KEY, scope_component_id INTEGER, name TEXT,
                              expression TEXT, value REAL, unit TEXT, description TEXT);
CREATE TABLE dag_nodes       (id INTEGER PRIMARY KEY, component_id INTEGER, kind TEXT,
                              payload JSON);          -- feature/param/joint definition
CREATE TABLE dag_edges       (from_node INTEGER, to_node INTEGER,
                              PRIMARY KEY (from_node, to_node));
CREATE TABLE history_commits (id INTEGER PRIMARY KEY, parent_id INTEGER,
                              node_id INTEGER, created_at TEXT);  -- commit graph for undo/redo
CREATE TABLE ui_state        (key TEXT PRIMARY KEY, value JSON);
CREATE TABLE geometry_cache  (node_id INTEGER PRIMARY KEY, fingerprint TEXT NOT NULL,
                              brep BLOB, mesh BLOB, occt_version TEXT);  -- derived; rebuildable
```
The exact `payload`/`kind` encoding for each feature type is **TBD** but must round-trip
losslessly.

---

## 8. Scripting (Lua API)

Everything achievable in the GUI must be achievable by programming, and vice versa.

- The Lua API exposes the full document model: create/edit components, parameters,
  sketches, constraints, features; run booleans; export; etc.
- Scripted actions create DAG nodes identical to GUI actions — there is one model, two
  front ends.
- The interpreter is **sandboxed** (no arbitrary filesystem/network access by default;
  explicit, opt-in capabilities only).
- The API surface is versioned and documented. Exact module layout and function signatures
  are **TBD**, but must be designed so that the GUI's command set maps 1:1 onto API calls
  (this also powers the CLI, §9, and the command palette, §11).
- **Namespace split.** The primary API is *declarative modeling*, in the spirit of OpenSCAD:
  geometry/document operations live at the top level (`bearcad.new`, `bearcad.rect`,
  `bearcad.extrude`, `bearcad.add_constraint`, `bearcad.parameter`, `bearcad.select`, …).
  All **GUI/UI manipulation** — simulated mouse/keyboard, camera, tools, panes, the command
  palette, and viewport drags — lives under the `bearcad.ui.*` sub-namespace
  (`bearcad.ui.move`, `bearcad.ui.click`, `bearcad.ui.key`, `bearcad.ui.type`,
  `bearcad.ui.orbit`, `bearcad.ui.pan`, `bearcad.ui.wheel`, `bearcad.ui.view`,
  `bearcad.ui.tool`, `bearcad.ui.pane`, `bearcad.ui.palette`,
  `bearcad.ui.wait`, `bearcad.ui.screenshot`, …). Examples and documentation should model
  with the top-level API and avoid `bearcad.ui.*` except where a UI interaction is the point.
- **Semantic gizmo manipulation (#114).** `bearcad.drag_vertex` and `bearcad.drag_line` take
  sketch-local (not viewport) coordinates, so they are top-level modeling calls (with
  back-compat aliases under `bearcad.ui.*`). Besides the positional absolute forms, each has
  a table delta form that moves things like a mouse drag would without knowing coordinates:
  `bearcad.drag_vertex{ point = <point>, du?, dv? }` nudges a vertex from wherever it
  currently is, and `bearcad.drag_line{ line = <line>, du?, dv? }` translates a line. Both
  respect constraints — attempting to drag a fully constrained vertex/line raises a
  catchable Lua error, like the GUI refusing the drag.
- **Scriptable gizmos (#214).** Viewport gizmos — a tool's drag handle for its live value,
  each a single scalar — are enumerable and drivable from a script, so gizmo-driven tools are
  automatable/testable without a mouse. `bearcad.gizmos()` returns the gizmos available in the
  current tool/creation state (`{ kind, name, value }` per handle; `kind` is `"push_pull"`,
  `"rotate"`, or `"offset"`; push/pull and offset in mm, rotate in radians). `bearcad.set_gizmo{
  name, value }` sets the scalar; `bearcad.drag_gizmo{ name, by }` nudges it by a delta. The
  value is applied the same way a drag does (the semantic path). Current coverage: the extrude
  push/pull depth (`"extrude"`), the chamfer/fillet amount (2D sketch-vertex and 3D body-edge,
  named `"chamfer"`/`"fillet"` by kind), the revolve sweep angle (`"revolve"`, radians), the
  construction-plane offset (`"offset"`), and the Move tool's translation
  (`"move_x"`/`"move_y"`/`"move_z"`, mm) and rotation (`"move_angle"`, radians, present only once
  a rotation axis is picked). The Move values are exposed ahead of the viewport drag handles
  (#185/#215/#216).
- `bearcad.ui.screenshot([path], [whole_window])` captures the 3D viewport only by default (the
  view bear (the view-cube HUD) is suppressed for that frame); passing `whole_window = true` captures the
  entire window. With no `path`, the image is written to `screenshot-bearcad.png`.
- Geometry-creation helpers are single calls that create the thing directly (no simulated
  mouse/keyboard) and enter a ground-plane sketch if none is open: `bearcad.rect{ width, height,
  x?, y?, name? }`, `bearcad.line{ length, angle?, x?, y?, name? }` (or explicit endpoints
  `bearcad.line{ x, y, x1, y1 }`), and `bearcad.circle{ r|radius|diameter, x?, y?, name? }`.
  A scripted line lands **unconstrained**, exactly like a click-drawn one; passing
  `dimension = "<expr>"` (or a number, or `true` for the as-drawn length) locks its length,
  the scripted equivalent of typing a length while drawing. Session-command export carries
  the typed expression through (`Export Session Commands…` replays typed-length lines
  dimensioned and click-drawn lines free).
- **Sizes accept parameter expressions (#402)** anywhere the GUI's dimension fields do: rect
  `width`/`height`, circle `r`/`radius`/`diameter`, and `extrude`/`edit_extrusion` `distance`
  each take a string expression (`"w"`, `"w / 3"`, `"1in + 2mm"`) in place of a number. The
  expression is stored the way typed input stores it — rect/circle sizes as locked dimension
  constraints, extrude distances in the extrusion's `expression` — so the scripted model
  rebuilds when the parameter changes. A radius expression is stored doubled
  (`"(<expr>) * 2"`) on the diameter constraint. An expression that doesn't evaluate raises a
  Lua error naming it. The JSON dispatcher accepts the same string-for-number forms, and
  session-command export round-trips the expressions (rect/circle commits carry their typed
  dimension expressions; extrudes replay `distance = "<expr>"`).
- `bearcad.plane{ offset?, from?, name? }` (#116) declaratively adds a new construction plane
  offset along the normal of an existing one (`from`, a construction-plane index — defaults to
  plane 0 / Ground), the scripted equivalent of picking a plane in the viewport and typing an
  offset. There is no scripted way yet to anchor a new plane on an axis (which also takes an
  angle) — only `edit_plane`/`commit_plane`/`set_dim("offset"|"angle")` reach that, and only for
  an already-existing plane.
- **Invalid input fails loudly (#104/#109/#110/#112):** when a declarative modeling call's
  underlying action is rejected — degenerate input (zero-size rect/circle/line, zero-distance
  extrude), an extrude face that doesn't exist or isn't a closed loop, a chamfer/fillet vertex
  that doesn't join exactly two lines or whose corner is within ~1° of straight (§3.1), an
  out-of-range 3D edge, … — the call raises a Lua error (catchable with `pcall`) instead of
  silently succeeding with nothing created. The GUI surfaces the same rejection message
  through the status bar. Options tables also **reject unrecognized keys (#403)** — a typo
  like `combine{ kind = … }` (the key is `op`) or `repeat_bodies{ gap = … }` errors
  immediately, naming the accepted keys, instead of being ignored and failing confusingly
  downstream. `gap` is in fact accepted everywhere `spacing` is (it's the Repeat pane's name
  for the field; passing both errors).
- **Read-back / introspection (#107):** the API is not write-only — pure read getters (never
  recorded as instructions) let scripts assert what they built: `bearcad.count(kind)` /
  `bearcad.get{ kind, index }` over lines, circles, sketches, constraints, construction
  planes, extrusions, bodies, and parameters (`count` also takes `drawing`, `sketch_text`,
  and `image`); `bearcad.body_stats(i)` (mesh
  volume/triangles/bbox); `bearcad.status()`; `bearcad.selection()`; and
  `bearcad.parameter("get"|"get_expression", name)`.
- **Absolute camera control (#108):** `bearcad.ui.camera{}` reads the pose
  (yaw/pitch/distance/target/projection); `bearcad.ui.camera{ … }` sets any subset instantly
  (no transition animation — deterministic screenshots); `bearcad.ui.zoom_fit()` frames the
  whole document (bodies + sketch geometry); `bearcad.ui.elements_view("list"|"tree"|"graph")`
  drives the Elements pane's layout (#34/#94).
- `bearcad.begin_sketch{ … }` starts a sketch on any face. Besides `kind = "circle"|"plane"`
  with `index`, it accepts **3D body faces**: `kind = "extrude_cap", extrusion, profile =
  "circle"|"polygon"|"boolean" (with `profile_lines = {..}` for polygons, or `boolean =
  {op, a, b}` — the same descriptor `extrude` takes — for a boolean-combined profile's cap,
  #406), profile_index, top?` and `kind = "extrude_side", extrusion, profile, profile_index,
  edge?`. (This makes sketching on a solid's face scriptable, e.g. for testing.)
- **Point-level selection (#68):** `bearcad.select{ kind = "line", index, ["end"] = "start"|"end" }`
  selects an individual vertex (a `ConstraintPoint`) rather than the whole element, so e.g.
  `bearcad.select{...}` + `bearcad.select({...}, true)` + `bearcad.add_geometric_constraint("coincident")`
  can join two line endpoints (closing a polygon loop — including a rectangle's four corners)
  purely from a script — a line's two points are `start`/`end`, i.e. `(x0,y0)`/`(x1,y1)`.
  A table with no `end` still resolves to the whole element as before; pass an explicit
  `point = true` to target a point that has no such field (e.g. a circle's center).
- **Face vertex/edge selection (#26/#27):** `bearcad.select{ kind = "face", face = { … }, index }`
  selects a corner of the *sketched-on* face's own boundary loop (a `ConstraintPoint::FaceVertex`);
  add `edge = true` to select the edge from that corner to the next instead
  (`ConstraintLine::FaceEdge`). `face` is a nested table in the same shape `begin_sketch` takes
  for a 3D body face (`kind = "extrude_cap"|"extrude_side", extrusion, profile, profile_index,
  top?/edge?`). Combine with the point-level selection above to build the constraint purely from
  a script, e.g. pinning a sketch point coincident to the face's corner 2.

---

## 9. Command-line interface

**Guiding principle:** the CLI can do *anything the GUI can do except operations that
inherently require mouse interaction* (e.g. free dragging in the viewport). The CLI and
GUI share the same model and the same action set; most CLI subcommands are thin wrappers
over scripting (§8).

Instruction scripts (§9.3) are the deliberate exception to the "no mouse interaction" rule;
they exist specifically so that interactive flows can be driven programmatically for testing
and automation (including screenshot capture of the live UI).

### 9.1 v1 subcommands
- `export` — export a `.bearcad` to `.3mf`, `.stl`, `.obj`, `.amf`, or `.step`/`.stp`.
- `run` — execute a Lua script headless against a new or existing `.bearcad`.
- `render` — render the model to an image (e.g. PNG) from a specified camera.
- `set` / parameter override + re-export — override named parameters from the command line
  and export, enabling part families from one file.
- `import` / `convert` — import STEP/STL/etc. into a `.bearcad`, or convert between formats.
- `install-cli` / `uninstall-cli` — symlink the running executable onto PATH as `bearcad`
  (default `/usr/local/bin/bearcad`), and remove it. Because macOS drag-to-Applications
  installs run no code, this is how the bundled binary becomes usable from a terminal; it is
  also exposed as **Help → Install "bearcad" Command in PATH**. Refuses to clobber a
  non-symlink at the target, and reports a sudo hint on permission errors.

The command set is expected to **grow over time** toward full GUI parity. New GUI actions
should be added to the shared action layer so they become available headlessly by default.

- `--timeout <seconds>` — force-exit (non-zero) if the app hasn't closed on its own within
  the given duration, so an unattended/CI launch can't hang forever (#61).

### 9.2 Export formats (required)
`.3mf`, `.stl`, `.obj`, `.amf`, `.step`/`.stp`. STEP via OCCT; mesh formats via OCCT
tessellation + writers (or dedicated libraries — license-audited per §1).
- **Whole-document export unions intersecting bodies (#146):** a whole-document export fuses
  the kernel-representable bodies into one real union before writing, so where two or more
  bodies **intersect** the overlap merges into a single watertight surface instead of exporting
  as interpenetrating shells with internal walls. Disjoint bodies are unaffected (they co-exist
  in the fused result). Imported (STL) mesh bodies have no kernel solid, so they're appended as
  their own triangles; if any non-imported body isn't kernel-representable, or the kernel is
  absent, the export falls back to plain per-body concatenation. Single-body and explicit
  per-body exports are never unioned.

### 9.3 Instruction scripts (for automation & testing)

**Directive:** The app should be fully scriptable. One must be able to run the app with a set of instructions (from a file) and the app must open and run each of the instructions. One must be able to export a screenshot of how the app looks as one of the instructions. This can then be leveraged for testing.

The application must be fully scriptable via a file containing a sequence of instructions.

- Invocation: `bearcad <script-file>` or `bearcad --script <script-file>` (or equivalent).
- When a script is provided the app shall open, sequentially execute every instruction in order,
  and apply the effects exactly as a user would (updating document, tools, camera, in-progress
  interactions, UI state, etc.).
- **Interactive REPL** (`bearcad --repl`): the same Lua API, driven line-by-line from stdin
  against the live app while the GUI stays fully interactive. One persistent Lua state for the
  session (globals survive between entries), bare expressions echo their value (`tostring`),
  errors report and the session continues, syntactically incomplete entries (unclosed
  `function`/`do`) buffer under a continuation prompt, and EOF (Ctrl-D) ends the session
  (combined with `--exit`, it also closes the app). Yielding instructions (waits, screenshots)
  work from the REPL exactly as from scripts. `--repl` and `--script` are mutually exclusive.
- One supported instruction must be screenshot/export of the app's current visual appearance:
  `screenshot <output-path>` (PNG or other common image format). The captured image must be a
  faithful rendering of the full window (or primary viewport + overlays) at the moment the
  instruction is executed, suitable for visual regression testing.
- Scripts shall support at minimum:
  - Core actions (new, open, save, clear, tool selection, rectangle creation flow including
    the click-to-place, mouse-move preview, dimension typing, tab, enter steps, etc.).
  - Camera/view control.
  - File I/O and export.
  - The screenshot instruction above.
  - Simple sequencing / waits if needed for UI settling or animations.
- This mechanism exists primarily to enable automated testing. Test scripts can drive the exact
  interactive flows (e.g. the rectangle tool's click → move → type → enter sequence) and emit
  screenshots that can be compared against golden images in CI.
- Execution must be deterministic (fixed random seeds, consistent layout, theme, DPI, camera,
  font rasterization, etc.) so that screenshots are reproducible.
- The precise syntax and full instruction vocabulary are **TBD** but must be simple,
  human-readable, versioned, and documented. The implementation must keep the set of
  instructions in sync with GUI actions.

The guiding principle in §9 still applies for normal CLI; instruction scripts are the
explicit exception that lets us drive "mouse/keyboard" flows for testing purposes.

**Documentation screenshots.** The screenshots in the docs site (§below / `docs-site/`) are
auto-generated by this mechanism rather than captured by hand, so they stay in sync with the app.
Screenshot scenes are stored as Lua scripts in `docs-site/screenshots/*.lua`; each builds a small
deterministic scene, sets a fixed camera, and calls `bearcad.ui.screenshot(...)` writing to the
directory named by `$BEARCAD_SCREENSHOT_OUT`. `scripts/gen-doc-screenshots.sh` runs them all into
`docs-site/static/img/screenshots/` (git-ignored build artifacts), failing if any expected PNG is
missing. The Website CI job (`.github/workflows/docs.yml`) regenerates them on Linux under
`xvfb` + a software Vulkan driver, uploads them as a downloadable artifact, and includes them in
the deployed site. This reuses §9.3's determinism guarantees (fixed view, no animation waits).

**Style swatches (#160/#173).** The docs "Viewport styles" page documents every geometry
style (line kinds × normal/hovered/selected states, points, faces, body auras, and linear/
angle dimensions in their normal and hover-accent colors). Hover states can't
be captured by scripted screenshots (scripted pointer moves don't reach egui, #130), so the
swatches are **drawn directly into PNGs** by `src/style_swatches.rs` using the renderer's own
color constants — regenerated by `cargo test generate_style_swatches -- --ignored`, which
`gen-doc-screenshots.sh` runs alongside the screenshot scripts (no GPU/display needed).

---

## 10. Geometry kernel integration (OCCT)

- Integrate OCCT via Rust FFI through a **hand-written thin C++ shim** exposing only the
  operations BearCAD needs (sketch profiles, prism/revol, boolean, fillet/chamfer, shell,
  sweep/loft, STEP/mesh I/O, tessellation). All `unsafe`/FFI is isolated behind a safe Rust
  `kernel` module (`src/kernel/`, shim in `cpp/`). The shim presents a flat `extern "C"` C
  ABI (no C++ types cross the boundary), so no `bindgen` is required.
- OCCT is **statically linked**, gated behind the **`occt` Cargo feature** (on by default) so
  a `--no-default-features` build needs no built OCCT (a C++ compiler is still required — the
  vendored libslvs sketch solver links into every native build). Static linking is permitted under
  OCCT's LGPL 2.1 because BearCAD ships the means to relink against a different OCCT: the
  pinned OCCT source (the `third_party/OCCT` git submodule), a build script
  (`scripts/build-occt.sh`), and an `OCCT_DIR` env override that repoints the link at any
  OCCT install prefix. See `README.md` ("Building with the OCCT kernel") and
  `THIRD_PARTY_LICENSES.md`. (This supersedes the earlier dynamic-linking plan in §1; the
  LGPL obligation is met by relink-ability rather than by dynamic linking.)
- A **Help ▸ Licenses** menu item links to `THIRD_PARTY_LICENSES.md`, which reproduces/points
  to the LGPL 2.1 + OCCT exception text and every other dependency's license, satisfying the
  attribution/notice obligations.
- Record the OCCT version in the file (§7.1) to support deterministic recompute (§4.4).
- Kernel errors must be converted into typed Rust errors attached to the failing DAG node —
  the shim catches OCCT C++ exceptions at the boundary and returns error sentinels rather than
  unwinding across FFI.
- **Default feature + CI/release wiring** (#89): `occt` is **on by default**, so the normal
  `cargo build`/`cargo run` ships the kernel (the default `cargo build` therefore needs a C++
  toolchain + a built OCCT). The lean no-kernel fallback builds with `--no-default-features`.
  A dedicated CI job builds OCCT once (cached on the pinned submodule + build-script hash) and
  runs the default (kernel) test suite; the `ci` job separately tests `--no-default-features`,
  so both paths stay green. **macOS and Linux release binaries ship with the kernel** (default
  build); **Windows lags** — a static OCCT/MSVC build is being stood up via an experimental,
  non-blocking `windows-occt` CI job (#96); the Windows release still ships the
  `--no-default-features` fallback build until that build is proven.
- **Migration status**: extrusions (prism/loft), multi-body union, solid booleans (incl.
  extrude cut), 3D edge fillet/chamfer, and STEP I/O are switched onto OCCT in `occt` builds,
  each with a hand-rolled fallback retained for the no-kernel build and for cases OCCT doesn't
  yet cover (multi-face profiles, imported meshes). The fallbacks are **not** removed until the
  kernel is the shipping default on all platforms (blocked on Windows, above).

---

## 11. GUI

### 11.1 Layout
- **Tiled panes only** — avoid floating windows and modals. Use docking/splitting.
- Core panes: 3D viewport, action-DAG/history graph, parameters, feature/constraint
  properties, component/assembly browser.
- **Context pane:** shows the **union** of editable properties for everything currently
  selected (or for the active draw tool — including before the first click — and for
  in-progress draw operations). If selected items disagree on a property, the control
  shows a mixed/indeterminate state; applying a new value sets that property on all
  applicable targets. Draw-tool mode takes precedence over selection when both apply.
  Fields render as **two aligned columns** (#371, `context::labeled_row`): the label in a
  fixed-width left column, the input/value (including element pickers) in the right column,
  so inputs line up down the whole pane.
- A standard **application menu bar** (File / Edit / View / Help) sits above the
  workspace. Menu items dispatch the shared action layer (§8) so menu, toolbar,
  shortcuts, and scripting stay in sync. The **View** menu contains a **Panes**
  submenu that shows/hides each available pane via a checkbox. (The menu bar is
  drawn in-window rather than as a native OS menu so it appears in screenshot
  regression tests, §9.3, and stays consistent across platforms.)
- **Import/Export menus & toolbar (#352):** the File menu groups the model interchange under an
  **Import** submenu (STL/STEP/Image) and an **Export** submenu (STL/STEP), and the model
  workbench toolbar has matching **Import** and **Export** icon buttons whose popups offer the
  same formats — so import/export is reachable from either the menu or the toolbar. (The drawing
  workbench's own Export icon, #348, exports SVG/PDF instead.)
- **STL export from the GUI:** **File → Export → STL…** exports all bodies (via a save
  dialog); right-clicking a **body** row in the Elements pane exports just that body. Both
  mirror the scriptable `bearcad.export_stl` (§8, §9.2).
- **STL import (#70):** **File → Import STL…** (open dialog) reads an STL file — ASCII or
  binary, auto-detected by exact byte-length match against the binary format's
  header+triangle-count framing — and adds it as a new **Body** with no source feature (no
  sketch/extrusion to nest under, so it nests directly under the Elements pane's Document
  root (#87), named after the file). Scriptable via `bearcad.import_stl(path)`. The mesh is
  stored and rendered as-is (no auto-centering/scaling); it participates in STL/STEP export,
  visibility, renaming, and deletion exactly like any other body, but — since it has no
  sketch/distance parameters — can't be edited or merged into by a further extrude the way
  an extrusion-backed body can (#32).
- **STEP export/import (#65/#71):** **File → Export STEP…** / **Import STEP…** (and the
  per-body Elements-pane export). With the OCCT kernel compiled in (`--features occt`, §10),
  a single-body STEP export — including the whole-document export when the document holds
  exactly one live body (#106) — writes **real BREP** (planar *and* curved surfaces) straight
  from the body's OCCT solid via `STEPControl_Writer`, and import reads **real BREP incl.
  curved/NURBS surfaces** via `STEPControl_Reader`, tessellating the result into a new **Body**
  (nests under the Document root, named after the file). Scriptable via `bearcad.import_step`
  / `bearcad.export_step`; import/export/open/save failures raise catchable Lua errors (#106).
  - **No-kernel fallback:** builds without OCCT (and the multi-body export path, plus any body
    whose geometry isn't kernel-representable) use the hand-rolled `step.rs` path — export
    writes a conformant AP203 `FACETED_BREP` with full product scaffolding (parenthesized
    complex context entity, `SHAPE_DEFINITION_REPRESENTATION` anchoring; OCCT and third-party
    readers can parse *and transfer* it, #106), and import reads only that same
    `POLY_LOOP`-bounded planar `FACE_SURFACE` subset. In this mode, STEP files using full BREP
    geometry (`ADVANCED_FACE` with curved/NURBS surfaces, as most CAD tools export) are
    rejected with a clear error rather than approximated. Imported bodies behave like STL
    imports (no analytic face/edge structure to sketch or edit against).
- **Export session commands:** **Help → Export Session Commands…** (also a command-palette
  entry, "Export Session Commands…") writes everything done since the app opened as a
  timestamped, replayable Lua script (the same instructions as `--show-commands`, §9). Useful
  for reproducing a bug by pasting the steps, or for turning an interactively-modeled part into
  a script. The session is always recorded interactively, including the interactive draw/extrude
  tools (#59): committing a rectangle/line/circle/extrusion logs the equivalent declarative
  `bearcad.rect{}`/`line{}`/`circle{}`/`extrude{}` call built from the as-committed geometry (not
  the in-progress drag), so a script-recorded session and a hand-written script produce the same
  document when replayed. Editing an already-committed extrusion isn't yet representable by a
  declarative call, so re-commits from the Edit flow aren't re-logged (a known gap, not a second,
  wrong instruction).
- **Document JSON dialog:** **File → Document JSON…** (also a command-palette entry) opens a
  dialog holding the whole document serialized with the web build's JSON codec
  (`storage::to_json_bytes`). Copy the text into a bug report to share exact document state;
  paste a reported document in and **Load into document** to reproduce it. Works identically
  on desktop and web (no file dialogs involved).
- **Elements pane view modes (#34/#252):** two icon-toggle buttons next to the pane heading
  switch between **List** (the default flat, topologically-sorted view) and **Graph**. The
  former **Tree** view is retired (#252): a strict tree can't represent an element with
  multiple inputs (a body that is both one op's output and another's input), which is the whole
  point of the graph model — so its button is gone and a script-set `Tree` mode renders as
  List. **Graph** is a 2D node-link diagram
  laid out by a **force-directed simulation (#94)**: nodes are pulled into depth-ordered
  horizontal layers so the graph flows top-to-bottom — "somewhat vertical" — while pairwise
  repulsion and weak, capped parent↔child springs spread siblings sideways; repulsion is
  deliberately sized to beat the springs at dot-diameter range so nodes never rest on top of
  each other (#151). The layout animates each frame ("bounces") until its kinetic energy
  decays and it settles, then stops repainting; the pane-edge clamp kills the velocity
  component into the wall so a crowded row settles instead of pumping forever. x is
  contained to the pane width so it never scrolls horizontally, only vertically. A depth band
  too wide to fit the pane **wraps into stacked sub-rows** (#350): each band is laid out to fit
  the width and the bands stack top-to-bottom by their wrapped height, so the graph grows
  **taller** rather than overflowing sideways (`declutter_label_bands` returns each node's x and
  sub-row). The seed layout is deterministic (reproducible across runs, no RNG). Each node draws as its
  element's icon — the same icon its List row uses, tinted by selection/health state
  (#152); only the synthetic Document root keeps a plain dot. Clicking a node in Graph view selects it like any
  other row; selecting a node highlights its ancestor and descendant nodes/edges with a distinct
  accent color/stroke. This is a per-session UI preference, not saved with the document.
  Beyond the single tree-parent edges, the Graph view also draws dashed **dependency edges**
  from an element's **inputs** to it (`graph_dependency_edges`), covering **every
  operation** (#448/#449): boolean/move/slice input bodies (#266), a repeat's input
  bodies/planes/sketches/replayed cut extrusions, a move's planes and images, a slice's
  construction-plane cutters, a revolution's profile sketch and axis line, the in-sketch
  repeat/slice ops' source lines/circles (+ the slice's cutter lines), a loft's section
  sketches, and a drawing projection's source body/sketch (#281) — the input edges of the
  eventual full element graph (#252). Nodes are **draggable** (#451): a per-node offset
  (`GraphLayout::drag_offsets`, UI state) adds on top of the physics/declutter layout, so
  the user can rearrange without fighting the sim.

### 11.2 Command palette
- VS Code-style palette listing **context-pertinent** commands. Commands come from the
  shared action layer (§8) so palette, shortcuts, GUI buttons, and scripting stay in sync.

### 11.3 Shortcuts
- Sensible defaults for the most common actions.
- **Every action is rebindable**; custom bindings persist (per §7.2, in-document; global
  defaults in app settings).

### 11.4 Theming
- Light and dark modes, ideally a general theme system.
- **Icons are always independent SVG assets** (#325): every toolbar/pane/button icon is a
  bundled SVG in `src/assets/icons/`, referenced through `icons::IconId`, and rasterized with
  `currentColor` so it inherits the theme's tint. **Never render an icon as a font glyph** (a
  Unicode arrow, box-drawing char, emoji, etc.) — those fall back to an empty box wherever the
  UI font lacks the codepoint, which is exactly the bug this rule prevents. A button that pairs
  an icon with a text label uses `egui::Button::image_and_text` with the SVG texture, not a
  glyph baked into the label string.

### 11.5 3D interaction
- Orbit/pan/zoom the 3D rendering; select faces/edges/vertices; manipulate sketches and
  features directly in the viewport.
- **Default viewport bindings** (all rebindable per §11.3):

  | Input | Action |
  |---|---|
  | Right-drag | Orbit the camera |
  | **Middle-drag**, or **Shift + right-drag** | Pan the camera (slide the view target in the view plane). Middle-drag is the browser-safe pan: Firefox forces its native context menu on Shift+right-click regardless of `preventDefault`, so the web build relies on middle-drag (#195). |
  | Mouse wheel | Zoom (dolly in/out) |

- **Zoom to Fit (#164/#279):** available from the toolbar **Zoom** button (magnifying-glass
  icon, in both the Model and Drawing workbenches), the **`Z`** shortcut (plain `Z`; `Cmd/Ctrl+Z`
  stays Undo), the command palette ("Zoom to Fit"), and the View menu. Frames the **current
  selection** (union of the selected elements' world bounds) so it nearly fills the viewport;
  with nothing selected it frames all **non-construction** geometry (bodies plus solid sketch
  lines/circles — construction scaffolding and datum planes are ignored). Scriptable via the
  existing `bearcad.ui.zoom_fit()` (whole-document form).
  | Left-drag (with an active draw tool) | Use the tool, e.g. draw a rectangle on the active plane |
  | **X** | Toggle construction/substantial on the in-progress draw op, or on each constructable selected item |
  | Escape | Cancel the in-progress operation; if none, deactivate the current tool (back to *Select*) |

- **Tooling model:** the viewport has an active **tool** (e.g. *Select*, *Rectangle*).
  *Select* is the default and only orbits/pans/zooms — geometry is created only when a
  drawing tool is active, so navigation never creates geometry by accident. Tools are part
  of the shared action layer (§8) so they appear in the palette and are rebindable.
- **Sketch-mode border (#74):** while a sketch is open, the 3D viewport is outlined in a
  bright orange border — a mode indicator distinct from every other viewport accent color, so
  sketch mode is never mistaken for ordinary 3D navigation at a glance.
- **Selectable hover feedback:** in any tool mode where the user can click to select
  geometry (e.g. picking a reference face or axis for a construction plane), every
  pickable target under the cursor is highlighted before click. The highlight uses a
  distinct accent colour and follows the shape of the target (line stroke, face outline,
  ground crosshair, etc.).
- **Proximity picking:** thin or point-like geometry (lines, endpoints, vertices) must
  be pickable within a screen-space tolerance — the pointer need not land exactly on the
  stroke. Lines use a pixel-radius threshold around the segment and its endpoints; faces
  use a margin around their projected edges. Hover resolution and click picking share the
  same resolver so feedback matches what a click would select.
- **Shape edges:** when a tool accepts a line or axis reference (e.g. construction-plane
  creation), standalone sketch lines and individual edges of shapes (rectangle sides, etc.)
  are all valid picks. Shape edges take precedence over the shape's face when the cursor is
  near the edge. Construction planes are the one exception (#124): they extend infinitely,
  so their rendered border is a display artifact, not real geometry — it isn't pickable as
  an edge/axis reference, only the plane's face is.
- **3D body edges (#31):** any edge of any 3D body — not just 2D sketch geometry — is a valid
  axis reference for a construction plane, including STL/STEP-imported bodies. An edge here is
  a *feature* edge of the body's triangle mesh (a mesh boundary, or a crease where adjacent
  triangles' normals differ by more than ~15°, so flat-face triangulation diagonals *and* the
  small seams between facets approximating a smooth curved surface are both excluded, #82/#101)
  — the same extraction `ShadingMode::Wireframe` uses to draw a body's edges — so it works
  uniformly for any body regardless of how it was created, without needing an analytic profile.
- **Global axes:** the origin X/Y/Z triad is pickable as an axis reference when creating
  construction planes. Axis gizmo handles show a hover affordance (bright ring and thicker
  stroke) so the user can see which handle will be grabbed on click.
- **Gizmos draw through bodies:** manipulation gizmos and their grab handles (plane-making,
  extrusion offset/angle, and any future gizmo) render with depth testing disabled, so they
  stay visible and clickable even when a body would otherwise occlude them.
- **Gizmo direction arrows:** every gizmo grab handle (plane/extrude/treatment offset
  handles and the axis-plane angle handle) shows flat line-drawn arrowheads — one per
  direction the handle can be dragged (both ways along the offset normal; both tangent
  directions on the angle circle), pointing away from the handle and stood off from its
  disc. Arrows are sized in screen pixels (constant on-screen size, like the disc
  handles) and drawn screen-facing; the non-GPU 2D painter fallback draws the same
  line-V arrows. (They were briefly solid 3D cones, which flared with perspective when
  orbiting/zooming — flat screen-facing arrows stay visually stable.)
- **View bear (view-cube HUD) settings popup (#33):** where the projection (orthographic/perspective)
  toggle button used to sit (bottom-left of the view bear), a gear icon instead opens a
  popup with two icon-button rows (words are avoided in favour of icons + tooltips):
  - **Projection** — the same orthographic/perspective choice the old button toggled
    directly; the active one is highlighted, click the other to switch.
  - **Ground** — how the ground plane renders (#159), one of two icon options:
    - *Ground grid*: the classic line grid (the default).
    - *Solid ground*: one filled plane in the grid's grey, slightly darkened, drawn with
      the same depth bias as the grid so bodies resting on z = 0 never z-fight it; the
      X/Y/Z axis lines still draw on top for orientation.
    Scriptable via `bearcad.ui.ground("grid" | "solid")`.
  - **Fill depth-biasing:** coplanar decals (sketch-shape fills, hover fills, stroke
    overlays) combine small world-space millimetre lifts with **slope-scaled pipeline
    depth bias** toward the camera (`wgpu::DepthBiasState`, sketch-fill and overlay
    pipelines): constant offsets alone collapse under glancing-angle depth interpolation
    on long thin faces (stippled z-fighting on e.g. an 8 ft board); the slope term grows
    the bias exactly where the depth gradient does. Construction-plane fills keep their
    away-from-camera bias so faces win overlaps deterministically.
  - **Sketch-mode dimming (#433):** while a sketch is open, every body's fill color is
    scaled down (`gpu_viewport::SKETCH_MODE_BODY_DIM`, 45%) in all shading modes, so
    sketch lines and dimension labels drawn over a face read clearly instead of fighting
    the face shading.
  - **Shading** — how committed bodies render, one of:
    - *Wireframe*: edges only, no fill. Draws *feature* edges only — mesh boundaries and
      creases sharper than ~15° — so the internal triangulation of flat faces (#82) and the
      facet seams of tessellated smooth surfaces like cylinder walls and fillets (#101) are
      not drawn. Smooth surfaces additionally draw their **view-dependent silhouette
      edges** (#158): the seams where the surface turns away from the camera (one adjacent
      facet front-facing, the other back-facing), so a cylinder shows its two tangent
      sides from any angle; these move with the camera and are rebuilt per frame.
    - *Transparent solid*: translucent fill with edges visible through it.
    - *Solid*: opaque fill, no edge overlay (the default — today's existing look).
    - *Solid + wireframe*: opaque fill plus an edge overlay that stays visible through the
      body, using the same depth-test-disabled technique as gizmos drawing through bodies
      (above) so the far-side edges aren't occluded by the near faces.
    - *Realistic (#83)*: ambient + diffuse + specular (Blinn-Phong-ish) lighting instead of
      `Solid`'s flat/Lambert-ish term, giving bodies a matte/satin "painted object" look with a
      camera-dependent specular highlight. The diffuse term is the stronger of a fixed scene
      light (above-ish, dominant, so form still reads) and a camera "headlight" (#102), so a
      face square to the camera is always clearly lit — roughly as bright as `Solid` — instead
      of dropping to the ambient floor when the fixed light misses it. Still flat-shaded per
      triangle (no shared vertex normals exist on the mesh), so it reads as faceted rather than
      smoothly lit. No materials/textures yet — every body renders with the same fixed gloss;
      per-body/per-face materials are future work.

  Both rows are backed by `Camera` state (a viewport display preference, alongside
  projection mode — not saved model geometry) and are fully scriptable:
  `bearcad.ui.toggle_projection()` / `bearcad.ui.view("orthographic" | "natural")` for
  projection, and `bearcad.ui.shading("wireframe" | "transparent" | "solid" |
  "solid_wireframe" | "realistic")` for shading.

### 11.6 First-person (FPS) mode (#91)

A completely different control scheme for walking around (and inside) models like a
first-person game, toggled via the command palette ("Toggle FPS Mode"), the View menu
("FPS Mode", checked while active), `Action::ToggleFpsMode`, or `bearcad.ui.fps()`. The
document is millimeters, so the player is person-scale: eye height
1700&nbsp;mm, walking ~4.3&nbsp;m/s.

- **Seamless entry (#135):** toggling FPS mode on never moves the view — the player's eye
  starts at the orbit camera's exact position and look direction, so the frame before and
  after the switch is identical (in perspective projection). Above standing eye height the
  player enters **flying** (gravity would otherwise yank the view to the ground); below it
  the player is auto-shrunk (see Scale, #120) so their standing eye height matches the
  camera and the first walking tick doesn't pop the view up (floored at minimum scale — a
  camera at/below the ground still pops up to the 17&nbsp;mm minimum standing height).
  Leaving FPS mode likewise keeps the camera where the player last stood; the player
  *scale* (but not position) carries over to the next FPS entry in the same session.
- **Movement:** WASD walks/strafes on the ground plane (heading follows the view yaw, but
  walking never leaves the ground); the mouse looks (raw pointer motion; the OS cursor is
  locked and hidden). On macOS the cursor stays **visible and un-grabbed**, warped back to the
  crosshair each frame; mouse-look reads the pointer's offset from the crosshair rather than a
  grabbed motion delta (#121). This is because a hidden cursor on macOS decodes a GIF through
  ImageIO on first use, which has been observed to crash (#119), and `CursorGrab::Locked` there
  freezes the pointer so egui reports no motion at all — warping a visible cursor sidesteps both.
  The offset only applies on frames where a pointer event actually arrived (#436): the warp
  emits no egui event, so `latest_pos` goes stale off-centre after the mouse stops, and
  re-applying that stale offset kept turning the camera slowly. **Web (#435):** the browser
  can't be cursor-grabbed via viewport commands, so entering FPS mode requests the real
  **Pointer Lock** and **fullscreen** on the canvas (best-effort — the browser may deny
  either outside a user gesture) and both release on exit.
  **Space** jumps (ballistic, gravity 9.81&nbsp;m/s²); **double-tap
  Space** toggles Minecraft-style flying (no gravity; Space ascends, Shift descends; flying
  into the ground lands and resumes walking). **Esc** leaves FPS mode.
- **Weapon-style tool switching:** number keys **1–9** pick tool slots (Select, Sketch,
  Rectangle, Line, Circle, Extrude, Dimension, Constraint, Plane) and the **mouse wheel
  cycles** through all tools (including Chamfer/Fillet) — the wheel does not zoom and
  right-drag does not orbit while in FPS mode.
- **Everything still works:** the controller owns the player's eye/look and *writes* the
  ordinary orbit camera every frame (`target = eye + look`), so rendering, picking, hover
  highlighting, and every gizmo behave exactly as in normal mode. The locked cursor sits at
  the viewport center (marked by a crosshair), so clicking interacts with whatever the
  crosshair points at. Panes, the palette, and modifier shortcuts stay available; while a
  text field has focus (e.g. typing a dimension) movement keys stand down, like an FPS with
  a menu open. Bare-letter shortcuts are suspended (WASD would collide), but Delete still
  removes the selection.
- **Scale (#120):** **`[`**/**`]`** shrink/grow the player by 2× per press (clamped to
  1/100×–100× human scale, i.e. eye height 17&nbsp;mm–170&nbsp;m), so mm-detail work and
  building/meter-scale walkthroughs are both comfortable without leaving FPS mode. Eye
  height, walk/fly speed, jump speed, and gravity all scale together (an intentionally
  smaller/larger person, not a world zoom); look sensitivity and `fps_move`'s explicit mm
  offsets are unaffected.
- **Scripting:** `bearcad.ui.fps(on?)`, `fps_look(dx, dy)` (degrees; positive dx looks
  right, dy up), `fps_move{ forward?, strafe? }` (mm along the ground), `fps_jump()`,
  `fps_fly(on?)`, `fps_advance(seconds)` (integrates physics with no keys held, e.g. to
  land a jump), and `fps_scale(value)` (sets the player scale directly, clamped as above).
  Outside FPS mode these raise catchable errors.

---

### 11.x Auto-update (#427)

Native builds check GitHub's latest release once at startup in a background thread
(`updater::spawn_check`, system `curl` against the releases API — no TLS dependency; the
check is best-effort and silent on failure; `BEARCAD_NO_UPDATE_CHECK` disables it, and
the doc-screenshot harness sets it). When a strictly newer version exists
(`updater::is_newer`, dotted numeric compare), a **bright green badge** appears in the
status bar's bottom-right corner — no popup, no interruption. Clicking it stages the
update **in place on every desktop OS**: **Windows** (bare exe artifact) and **Linux**
(tar.gz) download to a temp dir and swap the running executable via the rename trick
(old binary moves to `.old`); **macOS** (.dmg) uses the Squirrel.Mac trick — a running
`.app` bundle can be renamed — so it mounts the dmg (`hdiutil attach`), `ditto`-copies
the new bundle beside the installed one (same volume, so the final rename is atomic),
and rename-swaps (`BearCAD-old.app` aside; roll back on failure; dmg detached either
way). Once staged the badge becomes a **⟳ Restart BearCAD** button
(`updater::restart_into`: `open -n` for a bundle, plain spawn otherwise, then exit).
Leftover `.old` binaries/bundles are cleaned on the next startup. Fallbacks: a
non-bundle macOS run (dev build) auto-downloads the artifact in the browser; a failed
stage rolls back and opens the releases page.

### 11.x2 Auto-zoom (#438)

A toolbar **toggle** beside Zoom-to-fit (`AppState::auto_zoom`, off by default;
`bearcad.ui.auto_zoom(bool)`). While on and a rectangle or extrusion is in progress,
each frame checks the **live bounds** (document ∪ in-progress rect corners ∪ extrusion
profile swept by its live distance): if any corner projects off-screen/behind the camera,
or the whole thing occupies < ⅓ of the viewport (the extrusion was dragged back down),
the camera **glides** to frame the bounds (`Camera::frame_bounds_animated`, 0.22 s, same
destination math as the instant zoom-to-fit; orientation untouched). Triggers only
between animations (never fights an in-flight glide) and stands down in FPS mode and the
Drawing workbench. Decision logic is the pure `auto_zoom_should_frame` (unit-tested).

### 11.y Keyboard Shortcuts window (#434)

**View → Keyboard Shortcuts** / **Help → Keyboard Shortcuts** (and the palette entry
"Keyboard Shortcuts") opens a closable window listing **every** binding in the app,
grouped by scope: Everywhere, Tools (3D modeling workbench), Sketch mode, Constraints
(Constraint tool), Expression fields, First-person mode, and Technical drawings —
sections whose shortcuts only apply in a certain state carry a scope note.

The single source is **`shortcuts::all_shortcuts()`**. Maintenance contract: any new or
changed key binding MUST be reflected there in the same change. Two sections are derived
so they cannot go stale — Tools from `tool_shortcut()` (the same table the toolbar
labels use) and Constraints from `GeometricConstraintType::ALL` — with tests
(`shortcut_list_covers_every_tool_shortcut`,
`shortcut_list_covers_every_constraint_mnemonic`) enforcing the coverage; everything
else is listed explicitly in that function.

## 12. Technical drawings & printable schematics

BearCAD supports **2D technical drawings** derived from 3D models — dimensioned, annotated
sheets suitable for printing/manufacturing.

### 12.1 Model
- A **drawing** is a first-class document object (alongside components/assemblies),
  consisting of one or more **sheets** at standard paper sizes (ISO A-series, ANSI A–E)
  with a title block.
- A sheet contains **views** placed on it: orthographic projections (front/top/side/
  iso), section views, detail views, and a configurable projection convention (first- vs
  third-angle).
- Views are **associative**: each view references a component/assembly and recomputes
  when the source model changes (the reference is a DAG dependency edge, §4). Views have
  a scale (e.g. `1:2`), independent of model units.

### 12.2 Annotations
- Dimensions (linear, aligned, angular, radial/diameter), driven from real geometry and
  shown with the document's units; tolerances; leaders/notes; centerlines/center marks;
  surface-finish and datum/GD&T symbols (GD&T depth: **TBD**); a bill of materials /
  parts list for assemblies.

### 12.3 Output
- **Print** and **export to PDF** (vector) and **SVG/DXF** for the 2D content. PDF/SVG/DXF
  drawing export must be available from the CLI as well (§9), consistent with the
  GUI-parity principle.
- Drawing definitions (sheets, views, annotations, placements) are persisted in the
  `.bearcad` (§7); like geometry, computed view projections (HLR vector output) are **cached**
  in the file and invalidated when the source model changes, so drawings open fast (cache
  strategy mirrors §4.4). HLR is expensive, so caching it is especially important here.

### 12.4 Library notes
- Hidden-line removal / projected-edge generation comes from OCCT (e.g. its HLR
  facilities). DXF/SVG/PDF writers must be license-audited per §1.

---

## 13. Out of scope for v1 (record for later)
- Variable-radius fillets, simulation/FEA, rendering beyond basic shaded/snapshot,
  collaboration/multi-user, cloud sync, plugin marketplace. (Adjust as priorities change.)
- Technical drawings are **in scope** (§12). If schedule pressure arises, the minimum
  drawing v1 is: orthographic + iso views, linear/angular/radial dimensions, a title
  block, and PDF export.

---

## 14. Open items (TBD) — must be resolved before building the relevant area
1. Topological persistent-naming algorithm (§4.5).
2. ~~Constraint solver implementation choice (§6.3).~~ **Resolved:** native Rust LM solver.
3. Canonical internal units & full math function library (§5.2–5.3).
4. Full assembly joint catalog (§2.3).
5. OCCT binding strategy and the exact C++ shim surface (§10).
6. Lua API module layout and function signatures (§8).
7. Per-feature `payload` encoding in the SQLite schema (§7.3).
8. GD&T symbol coverage and standard for technical drawings (§12.2).
9. DXF/SVG/PDF writer library selection and licensing for drawing export (§12.3–12.4).
10. Geometry cache granularity — per-feature (floor) vs. per-body and/or tessellation-LOD
    entries, and the BREP/mesh blob encoding (§4.4, §7.3).
