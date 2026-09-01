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
Release builds (CI) sign the assembled app and the `.dmg` with a **Developer ID
Application** identity, hardened runtime (`codesign --options runtime`), then **notarize**
both with `notarytool` and **staple** the tickets (`stapler staple`). Local packaging
without a Developer ID certificate **ad-hoc signs** the app so a quarantined download is
not rejected by Gatekeeper as *"'BearCAD' is damaged and can't be opened"* (the message
macOS shows for an unsigned or signature-invalidated bundle on Apple Silicon). Sign inside
out (QuickLook `.appex` first, then the app, then the `.dmg`) after the executable, icons,
`Info.plist`, and PlugIns are in place, and verify with `codesign --verify --deep --strict`.
The `.dmg` volume must also contain an `Applications` symlink (→ `/Applications`)
alongside the app so the user can drag `BearCAD.app` straight into Applications from the
mounted volume. The Finder window uses a honey-toned drag-to-Applications background
(`macos/dmg-background.png`: app icon left, Applications right, arrow and hint in the
backdrop) rather than a blank folder (#1451). The `Info.plist`
must declare `.bearcad` as a document type (`CFBundleDocumentTypes` +
`UTExportedTypeDeclarations` for UTI `com.bearcad.document`) so double-click *launches*
BearCAD. The path is not argv: AppKit delivers `application:openURLs:` after
`applicationWillFinishLaunching:` (winit's delegate does not implement that method; we
steal the launch `odoc` Apple Event there and add `application:openURLs:` after launch)
(#1285, #1326). The bundle
ships a QuickLook Preview Extension at `Contents/PlugIns/BearCADQuickLook.appex`
(`com.bearcad.app.quicklook`) that claims the same UTI and renders the `preview_stl` mesh
snapshot embedded on save, with SceneKit camera control (rotate/pan/zoom like system STL)
(#1290).

**Linux packaging:** the tarball ships FreeDesktop `com.bearcad.app.desktop` and
`com.bearcad.app.xml` (MIME `application/x-bearcad`). First GUI launch and
`bearcad install-cli` install copies under `~/.local/share/…` with `Exec=` pointing at
the running binary (#1285).

**Windows packaging:** the portable `.exe` registers a per-user
`HKCU\Software\Classes` ProgID (`BearCAD.document`) for `.bearcad` on first GUI launch
and via `bearcad install-cli` (#1285).

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

The Elements pane draws a component as **two bodies fused into one solid** (#922) — the
union silhouette the extrude tool's "Join body" output icon uses, shaded like the body
element — and an imported unit instance as the **assembly** icon (#923): two of those
components meshed together, the back one in the accent blue, the front in the theme color.

**Implemented today (#423) — components as organizational groups:** `model::Component`
(`name`, `parent`, per-component `length_unit`/`angle_unit` overrides) plus
`Document::component_members` mapping top-level elements
(`model::ComponentMember`: planes, extrusions, bodies, lofts, boolean/move/mirror/repeat/
slice/edge-treatment ops, revolutions, sweeps, drawings) to a component; the document acts
as the root component. Grouping never changes geometry. Each `ComponentMember` variant
carries the identity its own collection uses — an index for a `Vec`-backed one, a key for
an arena-backed one (#1055) — so the kind and the reference are one value that cannot
disagree.

**Component identity (#1055):** `Document::components` is an `arena::Arena`, and a
component is named by a `ComponentKey` — `SceneElement::Component`,
`HierarchyNode::Component`, `JointRef::Component`, a component's own `parent`, the
membership entries, and `AppState::active_component`. Deleting a component removes it and
re-homes its members and child components to its parent (grouping is organizational, so
nothing inside is deleted). A script names a component by its **ordinal** among the live
ones, resolved to a key at the script boundary.

- **Active component (#429):** `AppState::active_component` (UI-only) is set when a
  component is created (the new component is also selected) or clicked; while set, the
  outermost `apply` files every newly created top-level element (planes, extrusions,
  lofts, ops, revolutions — bodies derive via their source) into it
  (`assignable_members` before and after, by difference rather than by count, since an
  arena's length is not a high-water mark; `assign_new_members`, inside the same undo
  step). The active
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
  to the parent, `document_lifecycle::delete_element`).
- **Visibility:** hiding a component hides everything inside it — members resolve
  through `hierarchy::owning_component` (a body via its producing op/extrusion, an
  extrusion or image via its sketch's host plane) and every ancestor component must be
  visible (`ElementVisibility::effective_visible`).
- **Hiding a construction plane keeps its sketches (#667):** a plane's hidden flag puts its
  own display quad away and nothing else — geometry sketched on it isn't part of the plane, so
  it stays. Only the plane's *own* flag is skipped: a plane anchored to a hidden sketch is
  still gone, and so is anything drawn on it (`plane_inherited_visible`). Sketches on a **body
  face** still follow that body — hide the body and the face isn't there to draw on. This is
  what lets the doc screenshots capture a sketch against a clean background.
- **Units:** each component may override length/angle units; contents inherit sketch
  override → component chain → document default (`effective_length_unit`,
  `effective_component_length_unit`). The context pane shows **Component units** pickers
  (with an **Inherit** entry) for a selected component.
- **Graph view:** a component is a row like any other, and its contents string down the lane
  it opens beneath itself (#1670).
- **Persistence:** `components`/`component_members` meta JSON in the SQLite format, serde
  fields in the JSON format.
- **Scripting:** `bearcad.component{ name =, parent = }` (returns the index),
  `bearcad.move_to_component{ kind =, index =, component = i|false }`,
  `bearcad.set_units{ component = i, … }`, `bearcad.select{ kind = "component" }`,
  `bearcad.count("component")`.

The full referenced-component/assembly model above remains future work.

### 2.3 Assembly

Parts — bodies, components, and imported unit instances — are related by **joints**
(#891, §3.3's Joint tool): parametric kinematic relationships that pose the driven side
in place at recompute. The catalog covers rigid (including rigid groups of 2+ parts),
slider, revolute, cylindrical, planar, ball, pin-slot, and screw. Path/cam joints, gear
ratios, and belt couplings remain future work.

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
- **Adaptive ground grid & origin axes (#353/#464):** the grid and the origin X/Y/Z axis
  triad scale with the camera so they stay a usable on-screen size for parts of any
  magnitude — the axes are kept at least ~90 px long. Grid spacing follows the
  **document's default length unit** (`gpu_viewport::grid_steps_for_unit`): two levels at
  once, heavy lines at a coarse step with lighter subdividing lines between them —
  powers of ten of a millimetre for metric documents; quarter inches under inches,
  inches under feet, then tens of feet for imperial ones. As the camera zooms in the
  next-finer rung **fades in** continuously (8→32 px screen spacing), and every line
  sits on a fixed world multiple of its step, so zooming never pops or slides lines.
- **A datum plane is part of the scene (#1087).** Committed construction-plane fills draw in a
  translucent layer **after** the opaque scene, so a plane shows through the body it bisects —
  a datum plane is a *reference* the user is pointing at, and it reads over the geometry it
  crosses rather than being hidden by it. Translucent **solids** (shadow bodies, previews,
  ghosts, faded bodies) share that layer, because they too are meant to tint what they cover.
  Body faces that **lie on** a construction plane (or an extrusion target plane) are then
  re-drawn once more **after** that wash (#1215/#29): coplanar solid/plane pairs would otherwise
  z-fight under floating-point depth, and neither world-space nor frag-depth bias is used
  (those mis-place planes — #1088/#1121).
- **The origin axes are widened on screen (#1072).** Each axis quad's corners carry both of
  the segment's world endpoints and a signed half-width in **pixels**; `vs_axis` projects
  both, then steps the corner sideways in screen space. A fixed-world-width quad is only the
  right thickness at one depth, so under perspective an axis used to swell at the near end
  and thin away at the far one.
- **The grid is a fragment shader (#1073).** The ground is one quad covering the visible
  footprint, and every line is worked out per pixel from the screen-space derivative of the
  world position (`fs_grid` in `shader.wgsl`, fed by `ViewportGrid`). Widths are therefore
  in **pixels**, not world units: a grid line is razor thin whether it runs under the camera
  or off to the horizon, and stays that way at a grazing angle, where the world-space
  quads it replaces used to foreshorten into wedges. The one-pixel coverage ramp
  anti-aliases it as a side effect. The quad is depth-tested but never depth-writing, so
  bodies occlude the grid while the gaps between its lines hide nothing.
- **Datum planes & plane size (#833):** a new document opens with three construction
  planes — **XY**, **XZ** and **YZ** — each 100 mm square and placed in the positive
  quadrant of its own space, standing the same 5 mm clear of the origin in both of its
  axes so the three frame the origin with a gap rather than boxing it in (#838). Every
  construction plane carries a `PlaneExtent { u_min, u_max, v_min, v_max }` giving its drawn
  rectangle in its own u/v axes; planes made by the Plane tool, and those in documents saved
  before extents existed, use the symmetric ±50 mm square. Selecting a plane (Elements pane
  or a viewport click) puts a square grip on its **low (`u_min`, `v_min`) and high
  (`u_max`, `v_max`) corners**; with the **Select** tool, dragging a grip moves that corner
  and leaves the opposite one where it is (`Action::SetPlaneExtent`, one undo step per drag,
  minimum 5 mm a side). The extent is display only — a plane still extends infinitely for
  sketching, picking and extrude-to targets. Readable from scripts as the `extent` field of
  `bearcad.get{ kind = "plane", index = i }`.

---

## 3. Geometry & modeling operations (v1 scope)

All geometry is B-rep via OCCT. The following operations are **in scope for v1**:

### 3.1 Sketching (2D)
- Sketches are created on a datum plane or a planar face.
- **Construction plane identity (#1055):** `Document::construction_planes` is an
  `arena::Arena`, and a plane is named by a `ConstructionPlaneKey` — `FaceId::ConstructionPlane`
  (so every sketch hosted on a datum plane names it by key), `SceneElement`/
  `HierarchyNode::ConstructionPlane`, `ComponentMember::ConstructionPlane`,
  `ExtrudeTarget::Plane`, `MateRef::Plane`, `ProjectionSource::Plane`, a tracing image's host
  plane, a plane's own `ConstructionPlaneParent`, and the repeat ops' plane targets and
  outputs. `Document::ground_plane()` is the XY datum every document opens with — what "plane
  0" used to mean. A script names a plane by its **ordinal** among the live ones, resolved to
  a key at the boundary. Load used to pad the plane list until every sketch's plane index
  existed; a key cannot be conjured that way, so a sketch whose host no longer resolves is
  simply reported unhealthy.
- **Circle identity (#1055):** `Document::circles` is an `arena::Arena`, and a circle is
  named by a `CircleKey` — `SceneElement`/`HierarchyNode::Circle`, `FaceId::Circle`,
  `ExtrudeFace::Circle`, `ConstraintEntity::Circle`, `ConstraintPoint::CircleCenter`,
  `DistanceTarget::CircleDiameter`, a repeat op's path circle, and the in-sketch operations'
  circle targets and outputs. Deleting a circle removes it, so a diameter dimension or an
  extruded profile on it reads as gone rather than moving onto the next circle. A script
  names a circle by its **ordinal** among the live ones, resolved to a key at the boundary —
  which is also why `FaceId::from_script` now takes the document.
- **Line identity (#1055):** `Document::lines` is an `arena::Arena`, and a line is named by a
  `LineKey` — `SceneElement`/`HierarchyNode::Line`, `FaceId::Polygon`, `ExtrudeFace::Polygon`,
  `ConstraintLine::Line`, `ConstraintPoint::LineEndpoint`, `DistanceTarget::LineLength`,
  `RevolveAxis::Line`, `ParameterSource::LineLength`/`LineDistance`/`LineAngle`, a sweep's
  path, a bridging line's `chamfer_fillet_parent`, and the in-sketch operations' line targets,
  cutters, and outputs. Deleting a line removes it, so a length dimension, a mirror target, or
  a polygon profile that named it reads as gone rather than sliding onto the next line. A
  script names a line by its **ordinal** among the live ones, resolved to a key at the
  boundary.
- **Sketch identity (#1055):** `Document::sketches` is an `arena::Arena` and `model::SketchId`
  *is* its key — so every line, circle, constraint, sketch text, extrusion, in-sketch
  operation, drawing view, and open-sketch session names its sketch by key. Deleting a sketch
  removes it (along with its geometry, as before), so anything left pointing at it reads as
  gone rather than adopting whichever sketch used to follow. A script names a sketch by its
  **ordinal** among the live ones, resolved to a key at the script boundary.
- **Draw tools begin sketches:** with no sketch open, the Rectangle, Line, Circle, and Text
  (#383) tools hover-highlight sketchable faces and a click begins a sketch on the clicked
  face — the tool then draws there immediately, no separate Sketch-tool step needed. An
  imported tracing image is the same kind of surface (#1588): a click on its quad starts a
  sketch on the image's host plane, including the part of the image that sits off the
  plane's display rectangle. Scriptable: `bearcad.begin_sketch{ kind = "image", index = 0 }`.
- **Rectangle anchor mode (#532):** the Rectangle tool's context pane has a two-icon radio
  — **corner-anchored** (`RectAnchor::Corner`, the classic behavior: the first click is one
  corner, drag to the opposite) or **centre-anchored** (`RectAnchor::Center`: the first click
  is the centre and the rectangle grows symmetrically as the cursor picks a corner). It is a
  persisted tool setting (`AppState::rect_anchor`, `Action::SetRectAnchor`). Pressing **R**
  while already on the Rectangle tool (and not mid-draw) **toggles** the anchor mode (the same
  key that selects the tool). The width/height dimension
  inputs always read the **full** extents in both modes (centre mode's cursor sits half a side
  from the centre). Live width/height (and line length) ValueInputs sit **outside** their edge
  and clear every corner/endpoint so a click can still land there (#1184).
  `CreatingRect::corners` resolves the two opposite corners for the preview,
  the live dimensions, and the commit, honoring the anchor and any locked width/height.
- **Circle anchor mode:** the Circle tool's context pane has the same two-icon radio —
  **centre + radius** (`CircleAnchor::Center`, the classic behavior: the first click is the
  centre, drag out the radius) or **edge to opposite edge** (`CircleAnchor::Edge`: the first
  click pins one point on the rim and the cursor drags to the diametrically opposite rim point,
  the two clicks spanning a diameter). It is a persisted tool setting
  (`AppState::circle_anchor`, `Action::SetCircleAnchor`); pressing **O** while already on the
  Circle tool (and not mid-draw) **toggles** it (the same key that selects the tool); the radio
  shows in 3D as well as in a sketch (#635). The
  diameter input constrains the circle in both modes. `CreatingCircle::center_local`/`radius`
  resolve the centre and radius for the preview, the live diameter, and the commit — in edge
  mode the centre is the midpoint of the two rim points, so a snapped first edge only positions
  the rim (there's no centre to pin).
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
  **Revolved bodies' flat faces are sketchable too (#621):** a partial (< 360°) revolve's two
  profile caps (`FaceId::RevolveCap`, the profile rotated to the sweep's start/end angle, its
  frame's normal facing out of the solid along the sweep tangent), and the flat washer/annular
  faces swept by any polygon-profile edge whose endpoints share an axis coordinate
  (`FaceId::RevolveSide`, one candidate per profile edge like an extrusion's side walls; full
  360° sweeps keep these even though they have no caps). A revolve side's frame sits **on the
  axis** (origin at the axis point, normal along the axis pointing away from the profile);
  edges not perpendicular to the axis sweep curved surfaces and are not offered. Sketches on
  either kind depend on the revolution that produced the face, and both faces hover-highlight
  and pick like extrusion caps (a full washer's pick is hole-blind, matching extrusion caps).
  Scripts address them as `{ kind = "revolve_cap", revolution = i, profile = …, endpoint =
  bool }` / `{ kind = "revolve_side", revolution = i, profile = …, edge = i }`.
  When several faces project onto the cursor (e.g. the near and far faces of a solid), face
  picking resolves to the one nearest the camera, so a hover/click never selects a face hidden
  behind the body. Entering a sketch reorients the camera head-on to the face and keeps the plane's
  own axes **screen-aligned** (each u/v axis lands on a screen axis, the two perpendicular), but
  takes the **shortest roll** from the current view rather than forcing a fixed u-right/v-up
  convention (#577): on the ground plane the old convention spun the camera all the way around, and
  with the sketch axes now drawn and selectable, orientation no longer has to encode which way is
  "horizontal" (`sketch_view_up_score` minimises the on-screen rotation of the u/v axes, with a tiny
  nudge toward the convention only to break near-ties). For a near-vertical face (such as a side
  wall) the view instead orients with world up (+Z) toward the top of the screen so the ground stays
  at the bottom and orbit behaves normally, rather than rolling sideways.
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
- **Sketch selection isolation (#742):** while a sketch is open, the selection-family
  tools (Select, Constraint, Dimension) hover and pick **only that sketch's own
  geometry** — its lines/circles/points/text, the origin and its axes, and the
  sketched-on face's own edges/corners (#26/#27). Outside bodies, other sketches, and 3D
  body sub-elements neither glow nor select; clicking them clears the selection like
  empty space. One filter (`element_in_sketch`) gates both the hover-highlight and the
  click path, and the Select picker the context pane hands out carries it as a
  `PickRule::InSketch` while a sketch is open (#982) — so the Selection Exploder's fan,
  which prunes to what the focused picker takes, can never offer a datum plane, world
  axis, or body that a click would refuse. Outside geometry enters a sketch only through
  the Projection tool (#140/#1193).
- **Projections (#140/#1193/#1197/#1199/#1351):** **`Tool::Project`** (shown as **Projection**
  since #753 — the verb read as a noun; Lua `"project"`; shortcut **P** in sketch mode) is
  a toolbar tool that appears only while a sketch is open. The declarative form is
  `bearcad.project{ body = i }` / `{ plane = i }` / `{ entities = { … } }` (empty call =
  current selection, including un-project). With it active, outside body
  edges/faces and crossing planes hover-glow and **clicks select** them (Shift multi-
  selects); **Enter** or the Context pane's blue commit button projects the selection onto
  the sketch plane along the plane normal through `Action::ProjectSelection` →
  `Action::ProjectSources`. A face or vertex projects the whole body's edges; a body does
  the same. Selecting external 3D geometry before opening the sketch, then activating
  Project (**P**) and pressing Enter, works the same — the tool's picker keeps projectable
  items across the handoff. Palette "Project Selection into Sketch" still runs
  `Action::ProjectSelection` directly. Each projected edge becomes a construction-style
  line drawn **solid cyan** (#1186; distinct from dashed construction) and usable like
  construction geometry (snapping, constraints). While the host sketch is open, **all of
  that sketch's lines** — solid, construction, and projected — **show through bodies**
  (depth-disabled; #1192/#1200) so a body between the camera and the sketch plane does not
  hide the profile. Closed sketches keep depth-tested strokes. In the Elements pane,
  projected lines wear the **Projection** (projector) icon, not the plain line glyph (#1193).
  **Un-project (#1193):** when the selection is **only** already-projected lines of the
  open sketch, Enter (or Project Selection) **un-projects** them (removes the references).
  A mixed selection of projected lines and outside sources still projects the sources.
  Projections are **associative**: each geometry recompute re-resolves the source edge and
  rewrites the projected line, so it follows its source body. Sources are geometry-keyed
  (mesh edges have no stable topological name), so if a rebuild moves/removes the source
  edge the projection keeps its last resolved shape rather than dangling; projected lines
  are fixed (not draggable). Edges edge-on to the sketch plane (zero projected length) are
  skipped. Standalone vertex projection is not yet supported (a projected edge's endpoints
  already serve as snap targets).
  **Construction planes project too (#983)** (`ProjectionSource::Plane`, identified by
  index — stable, unlike a mesh edge): the projected line runs along the two planes'
  intersection, spanning the source plane's drawn rectangle's shadow on that line
  (`plane_sketch_intersection`) — so the reference sits where the planes visibly cross even
  though the datum planes' quadrant rectangles float a gap clear of the origin — and
  follows the plane through moves and resizes. A plane parallel to the sketch resolves to
  nothing and is refused up front.
  **The tool picks outside geometry only (#983)**: its picker carries
  `PickRule::ProjectableInto(sketch)` — bodies, their edges/corners, crossing planes, and
  this sketch's **already-projected lines** (selected so Enter can un-project them) —
  never the sketch's own drawn geometry. The hover path, the click path, and the Selection
  Exploder's fan all consult that one rule, so none can offer what another would refuse.
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
  **The analysis excludes the solver's gauge-hold `Pin` equations (#459)** — those are
  solve-time stabilisers, not constraints; counting them once made every *dimensioned*
  shape read as fully constrained, so dimensioned-but-unpinned geometry refused to drag.
  A dimensioned shape drags as a whole (translation preserves its dimensions) and only
  locks once it's also located (e.g. pinned to the origin). Invariant, enforced in
  `dof::build_jacobian`: anything that must truly lock a variable uses `System::fixed`,
  never a `Pin`. Guarded by unit tests and by **interaction regression tests**
  (`tests/interaction/*.lua`, run in CI): scripted real pointer input — synthetic events
  delivered through eframe's `raw_input_hook`, so they build genuine egui pointer state —
  driving click-select and drags end to end, asserting on geometry via
  `bearcad.line_endpoints`.
  **A pointer call aimed at a world point waits out a camera glide (#1814):** where that point
  lands on screen is not yet where it will land, so the runner answers `StepResult::Wait` and
  the Lua wrapper yields and calls again (bounded, then errors). `exec` used to drop that
  answer, turning "not yet" into "never" — the call vanished and the script went on to assert
  against a click that never happened.
  Construction (dashed grey) and projected (solid cyan, #140/#1186) styling take precedence.
  **Construction geometry draws only inside its own sketch (#994):** it is scaffolding — a guide
  to dimension and constrain against, never model geometry — so a construction line or circle is
  hidden whenever its sketch isn't the open one (`construction_geometry_visible`). Left visible
  it stood dashed on the face of the finished part, telling every later view about a decision
  that belongs to one sketch. Solid geometry is unaffected: it still shows, dimmed, while another
  sketch is active.
- **Snapping:** while drawing or dragging sketch geometry, the cursor snaps to nearby
  vertices, line midpoints, lines, the sketch **origin**, and the sketch's two in-plane
  **origin axes** (the X axis `v = 0` and Y axis `u = 0`, #189) — vertices/origin take
  priority, then midpoints, then anywhere on a line or axis. Leaving a point on a snap adds
  the implied constraint (coincident for a vertex/origin/on-line/on-axis snap, midpoint for a
  midpoint snap), deduped against existing constraints. A point-on-axis snap is a point-on-line
  coincidence against the origin axis, pinning that coordinate to 0. A ring marks the active
  snap. Snapping is toggleable from the context pane and the toggle only appears for tools that
  snap (Select, Line, Rectangle, Circle); the Select tool shows it only while a sketch is open,
  while the three drawing tools show it in 3D too (#636) so their context sections read the same
  either way — the setting is sticky and carries into the sketch their first click opens. The
  origin (`SceneElement::Origin`,
  drawn as a small marker where the axes cross) and the origin axes (`ConstraintLine::OriginAxis`)
  are also directly viewport-selectable in the constraint tool — not just reachable by snapping —
  so a point can be constrained coincident with the origin, or onto an axis, by clicking them. A
  selected origin brightens to the selection color and a selected axis highlights along its full
  length so the pick is visible. The **world** axes carry their own scene element
  (`SceneElement::GlobalAxis`, #952) — fixed geometry with no owning entity, like the origin, and
  no Elements-pane row — so an axis pick has an identity an element picker can hold, which is what
  the axis inputs (a Repeat path, a Revolve axis) select into. Scriptable as
  `kind = "axis"` with index 0/1/2 for X/Y/Z.
- **Analytic faces are selectable (#952):** `SceneElement::SketchFace(FaceId)` names a sketch
  profile, a body cap/side wall, or a revolve's flat face — the *parametric* face an Extrude
  profile, a Revolve/Sweep profile, a Slice cutter, or a Mirror plane is defined against.
  Distinct from `SceneElement::BodyFace`, which is a **mesh** face keyed by quantized
  centroid+normal. Build one with `SceneElement::from_face_id`, never directly: a
  `FaceId::ConstructionPlane` normalizes to `SceneElement::ConstructionPlane`, so a plane has
  exactly one identity however it was reached (which is also what collapses the Selection
  Exploder's duplicate plane loupes into one).
- **Snap points are selectable (#952):** `SceneElement::MovePoint(MovePointRef)` names a point
  the Move/Joint tools snap from or onto — an edge midpoint, a point along an edge, a planar
  face's middle — so their twelve point pickers hold elements like every other picker. Built with
  `SceneElement::from_move_point`, which normalizes the two cases that already have an element: a
  body corner is `BodyVertex`, the origin point is `Origin`. `as_move_point` is the inverse, for
  handing a picker's contents back to the geometry code.
- **Repeat-instance faces are selectable (#955):** `SceneElement::RepeatedFace { face, op,
  instance }` names a repeat copy's translated plane (#452) — the source face's plane moved by
  that instance's offset. It is not the source face (a different plane) and has no independent
  existence in the document, so it needs an identity of its own; without one, the three
  "extrude up to"-style pickers couldn't hold what the user had actually snapped to.
  `SceneElement::from_extrude_target` maps every `ExtrudeTarget` onto an element, which is what
  the Extrude **Up to**, Repeat **Distance to**, and Joint **Min/Max stop** pickers hold. It
  highlights as the source face's boundary run through the instance transform.
- **Analytic extrusion edges are selectable (#952):** `SceneElement::ExtrusionEdge { extrusion,
  edge }` names what the 3D Chamfer/Fillet tool treats — distinct from `BodyEdge`, the quantized
  mesh edge. It highlights the **whole** analytic edge, so a hole's rim (one `Cap` reference,
  many mesh chords) reads as one circle rather than a row of facets (#807).
  A **loft section** needs no element of its own: it is a profile plus its sketch, and the
  profile is a `FaceId`, so the analytic face already names it
  (`extrude::extrude_face_scene_element`). The **sketch's two axes are always drawn** while a sketch is open
  (#577): the X (u) axis in the red axis color and Y (v) axis in green, through the origin, faint
  normally and brighter when hovered — the "floating origin" that makes the sketch frame's
  orientation unambiguous now that the camera takes the shortest roll instead of forcing u-right.
  Selecting a line **and** an axis and applying **Parallel** (or Perpendicular) constrains the line
  parallel/perpendicular to that axis — the general, axis-based replacement for the old separate
  Horizontal/Vertical constraints.
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
- **Regions of a hosted face (#993):** a sketch drawn *on a face* has a boundary it never drew —
  the face's own outline — so lines ruled across a box's cap read as separate faces even though
  neither line closes a loop with anything. `polygon::sketch_plane_regions` builds the **planar
  arrangement** of that outline together with the sketch's own solid lines (curves sampled,
  construction geometry excluded — it is scaffolding and bounds nothing): every segment is split
  where another crosses it, and the minimal faces of the resulting graph are the regions, wound
  counter-clockwise. It reports nothing when the lines divide nothing, since a single region is
  just the face over again. Clicking one gives `ExtrudeFace::SketchRegion { sketch, seed_u,
  seed_v }`, which names the region by a **seed point** (thousandths of a sketch unit, so the
  profile stays `Eq`/`Hash`) rather than by its boundary — the boundary is derived, running
  partly along the host's own outline, which has no line indices to point at. The region is
  recomputed from the live sketch on every resolve, so it follows edits like any other profile;
  if the cuts move out from under the seed it simply stops resolving, which `document_health`
  already reports as a face gone missing. Like `Boolean`, it has no `FaceId` of its own.
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
      same smoothing used by "Convert to bezier curve" below, and the joint records an explicit
      **`ConstraintKind::Tangent`** between the two line-ends (#473) — listed in the Elements
      pane like any constraint — so the tangency is durable state, not a coincidence of handle
      positions. Off: the previous segment's handle is left alone and the new segment gets an
      independent "corner" handle a third of the way along its own chord — a barely-curved
      starting shape meant to be reshaped by hand via the draggable handles below.
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
    active sketch) as small discs (sketch blue; gold when hovered, dragged, or selected, #472)
    with dashed guides back to their endpoint; dragging one reshapes the curve live — and when
    the handle's joint carries a Tangent constraint (#473), the partner handle on the other
    line rotates to stay collinear (keeping its own length), so a tangent joint stays tangent
    through any handle drag. **Clicking** (rather than dragging) a handle toggles the joint's
    tangent constraint on/off (status bar reports "Tangent: on/off"); clicking an
    already-selected vertex whose joint has a curve does the same. A click also selects the
    handle; pressing Delete/Backspace, or right-clicking it and choosing "Delete handle",
    straightens the line
    (#75) — a curve is either both handles or neither, so there's no independent per-handle
    state to remove, only the whole curve.
  - **Right-click a vertex:** right-clicking a vertex where exactly two plain lines meet offers
    "Convert to bezier curve", which smooths the joint into a tangent-continuous pair of curves
    (Catmull-Rom-style, using the two lines' far endpoints to set the tangent direction through
    the shared vertex). The reverse, "Straighten curve", is offered when right-clicking an
    existing curved line.
  - **Right-click already-selected geometry (#1224):** right-clicking something that is already
    selected (a body face/edge/vertex of a selected body, a selected line, plane, joint, …)
    opens the **same** context menu as that element's Elements-pane row — the shared
    `element_context_menu` (edit, Create drawing / Add to drawing, Make shadow body, exports,
    Move to, Rollback, Delete). Bezier-handle right-clicks still win when the pointer is on a
    handle.
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
- **Chamfer and fillet (#37/#38/#538/#1504), dual-mode — a parametric operation:** both
  are tools ("push/pull" gizmo + text-entry input, mirroring the extrude tool) that operate on a
  sketch vertex where exactly two plain lines meet, or on a solid edge when no sketch is open.
  Click toggles a treatable vertex or edge (Offset's rule). A commit creates (or extends) a
  `SketchVertexTreatmentOperation` — a first-class parametric node in the Elements pane, alongside
  in-sketch offset/mirror/slice — using a **shadow + replace** model (#538, standard CAD):
  - The two source edges of a treated corner become **shadow** lines (`Line.shadow = true`): they
    are kept and still solved with all their own constraints (dimensions included), but excluded
    from face detection. Because a source keeps its full length, its length dimension is
    **untouched** and references the **virtual sharp corner** where the untrimmed edges meet.
  - On every geometry recompute (`rebuild_sketch_vertex_treatment`), the op reads the shadow
    sources' solved endpoints and regenerates the visible geometry: one **trimmed copy** per source
    edge (a `Line`, trimmed at each treated end) plus one **bridge** per corner, tied together with
    fresh **stitch `Coincident` constraints** so the visible profile is a closed loop (still a
    fillable, extrudable face; loop detection walks the constraint graph, keyed by coincidence
    group across shadow endpoints). A **chamfer** bridges with a **straight** line; a **fillet**
    bridges with a line whose `bezier` field is a **single-cubic-bezier approximation of the
    circular arc** — reusing the bezier-curve machinery (rendering, hit-testing, extrusion
    tessellation) for free.
  - The chamfer distance / fillet radius is a **parametric expression string** (like a sketch
    offset's distance), evaluated each rebuild, so the bevel follows dimension and parameter edits.
  - One op owns a whole **connected treated region** (many corners), mirroring the 3D
    `EdgeTreatmentOperation`. Because adjacent corners share an edge, treating a corner whose
    edge(s) already belong to a live op **merges** the new corner into that op (merging two ops
    into one when the two edges belong to different ops) — so no two ops ever share a source edge
    and each op's rebuild is self-contained.
  - The tangent length is clamped so it never cuts back past either adjacent line's own far
    endpoint; a corner within ~1° of straight (0°/180°, i.e. parallel/anti-parallel edges) is
    rejected as degenerate. The source edges' virtual-corner `Coincident` is **kept** (the shadow
    sources still meet at the sharp vertex). Deleting the op un-shadows its source edges (sharp
    corner restored) and removes the generated copies/bridges; undo restores the pre-commit
    document. Other constraints that referenced the old vertex position are **not** automatically
    fixed up (a known, documented limitation; the sketch may need manual re-constraining).
  This is specifically the **2D sketch-vertex** case;
  the same Chamfer/Fillet tool also does a **3D solid-edge** mesh-bevel approximation on an
  extrusion's analytic side/cap edges when no sketch is open — see §3.4, which is *not* a true
  kernel-backed BREP fillet (BearCAD has no BREP/NURBS kernel — see §10). Scriptable via
  `bearcad.chamfer_vertex{ point = {...}, distance = }`
  and `bearcad.fillet_vertex{ point = {...}, radius = }`, where `point` is the usual
  `ConstraintPoint` table (e.g. `{ kind = "line", index = 0, endpoint = "end" }`).
  `points = { ... }` treats several corners in one operation (mirroring `edges` on
  the solid verbs).
  - **Live geometry preview (#76):** while the gizmo is being placed or dragged (before commit),
    the actual treated-corner shape is drawn as a preview overlay — the two truncated points and
    the bridge between them (straight for a chamfer, sampled from the fillet's bezier arc) — not
    just the gizmo arrow. It's recomputed every frame from the live drag amount, so pulling the
    handle further visibly grows the cut/round before you commit.
  - **Elements pane nesting (#76/#538):** the op is its own row (default label "Chamfer N" /
    "Fillet N", derived from its first corner's kind), with its generated trimmed copies and bridge
    lines nested beneath it and its shadowed source edges dimmed under the sketch — exactly like the
    in-sketch offset/mirror/slice operation nodes. (The legacy `Line.chamfer_fillet_parent` field is
    retained for backward-compatible loading and the solve-time fillet-arc re-fit.)
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
    `bearcad.extrude{ profiles, distance?, name?, body?, to?, taper?,
    taper_mode? }` (`profiles` is one circle, one line loop, a text, or a list of those;
    `bearcad.sketch_faces(sketch?)` lists them. `taper`/`taper_mode` #1243).
    **Extrude to object (#114):** instead of a fixed distance, `to = { plane = i }` /
    `{ face = <face spec> }` / `{ vertex = <point> }` snaps the extrusion to that object's
    extended plane, and the link is parametric — the snapped extrusion follows when the
    target moves. `face` accepts either a flat sketch profile (`{circle=i}`/`{polygon={..}}`/
    `{boolean={..}}`) or, for a 3D body's cap/side wall (#126) — including another body
    entirely, not just the extrusion's own sketch — the same `{kind = "extrude_cap" |
    "extrude_side", extrusion, profile, top?/edge?}` shape `begin_sketch` uses.
    **Semantic push/pull (#114):** `bearcad.edit_extrusion{ index|extrusion, distance?,
    by?, to? }` edits a committed extrusion like dragging its gizmo — `by` nudges from the
    current effective depth, `distance` sets an absolute depth (clearing any snap target),
    `to` (re)snaps. `index` is an alias of `extrusion`.
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
    kept. The preview **flips a cut that currently points out of the body inward**, exactly as
    the commit does (`resolve_cut_direction`), and — when the target body isn't
    extrusion-sourced (a fillet's output, a boolean's, a move's) — subtracts the tool from the
    body's own solid rather than rebuilding it from extrusions, so drilling into a finished
    part previews like anything else (#805). **Preview performance (#386):** both live previews are cached per
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
    directly on an existing body's own cap or side wall (an `ExtrudeCap`/`ExtrudeSide`), or a
    **revolved body's flat face** (`RevolveCap`/`RevolveSide`, #625), no separate sketch
    needed — "drag a face straight off a solid," like many CAD tools. This
    creates an implicit sketch hosted on that exact face and mirrors its boundary into it (a
    circular cap gets a real circle, not a tessellated approximation; a full-sweep revolve
    side — a complete washer — gets two real circles combined by difference, since a
    boundary line loop can't carry its hole), then starts a fresh
    single-face extrusion from it — a body face is never grouped with other faces into one
    multi-face extrusion, unlike coplanar sketch profiles. Sketching on an existing body's
    face merges into that body by default (§3.2's `body?` choice, #32), so pushing/pulling a
    face this way naturally extends the solid rather than creating a disjoint one.
  - The distance popup is the same field the line tool's distance uses (#881) — amber frame,
    monospace expression, computed value underneath — with the **Flip** button beside it.
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
  - **Faces picker (#268/#955):** the picked profile faces show in a real element picker over
    analytic faces (`SceneElement::SketchFace`), always focused — Extrude has only the one
    picker. Dropping a row goes through `Action::ToggleExtrudeFace`, so it does the same sketch
    bookkeeping and lands as the same single undo step as clicking the face off in the viewport.
  - **Taper (#1243):** an end-face size change vs the start face — `Extrusion::taper` plus
    `taper_mode` (`distance` | `angle`). Distance mode is millimetres **per side** (a 10×10
    square with taper 5 ends 20×20; circles add to the radius). Angle mode is a draft against
    the extrusion normal (−90°…89°; values outside that, or that would make a huge solid,
    clamp and warn); a negative draft that collapses the profile **cuts the
    height** (10×10 at −45° over distance 10 ends as a point at height 5). Past collapse the
    end stays a point (never inverted). With **Symmetric** (#1268) the sketch mid-plane keeps
    the profile size and both free ends take the taper (two half-frustums), not a single
    bottom-to-top taper. The context pane's **Taper** row toggles distance/angle via a picture
    icon like Repeat's measure toggles. Scriptable:
    `bearcad.extrude{ …, taper = 5 }` or `taper = -45, taper_mode = "angle"`.
  - **In-context distance / target / commit (#584):** the Extrude tool's context section carries a
    full alternative to the 3D gizmo — a **Distance** value input that mirrors the floating 3D field,
    an **"Up to"** element picker (single-select, planes/faces) for the extrude-to target, and an
    **Extrude** button. Typing in the Distance field drives the depth and clears any target; focusing
    the "Up to" picker arms **target-pick mode** so the next viewport click on a plane/face sets the
    target; dragging the gizmo onto a plane/face **sets that target in the picker and nulls the
    Distance field** (the depth then comes from the target). On **touch** the gizmo is a plain
    press-drag-release instead (#829): a finger has no hover, so "grab, follow the cursor,
    click to drop" leaves the handle stranded the moment the finger lifts — lifting is what
    locks it in there. Crucially, a gizmo release-click no
    longer *commits* — it just locks in the current distance/target; the extrusion is **completed
    only on Enter or the Extrude button** (`context::ExtrudeControl`/`ExtrudeEdit`, wired through
    `App::extrude_target_pick`). Picking a face leaves the distance field holding the
    keyboard with its value selected, so a depth can be typed straight away (#437/#880),
    and **Enter commits whichever field has it** (#880) — **except while target-pick mode is
    armed** (#988), when that field **surrenders the keyboard**: the next thing to happen is a
    click on geometry, not a typed depth, and the target is about to decide the depth anyway.
    Holding it swallowed the **Space** that opens the Selection Exploder (via
    `egui_wants_keyboard_input`, #794), and the fan is the only way to name a face **buried
    behind the solid** — "up to the bottom of this box" being exactly that. Two more things had
    to stand down for that pick to land: the **pull-handle gizmo** takes no pointer while the fan
    is open (#986's rule — the redirected anchor lands on the handle in a top view, where
    everything on the extrude axis projects to one spot), and the hovered leaf's **own target**
    is applied rather than re-resolving a pick at its anchor, which `pick_sketch_face` would
    answer with the near face again. `focus_tool_picker` arms it (and Repeat's "Distance to"),
    which it previously did not — so neither pick mode was reachable from a script at all.
  - **Body target (#32/#35)**: a `Body`'s source is one or more extrusions (`BodySource::Extrusion`
    for one, `BodySource::Extrusions` for several; `BodySource::Solid { add, cut }` once some of
    its extrusions are subtracted rather than added — see §3.3). Extruding a profile that sits on an
    existing body's face (a cap or side face) defaults to joining that body; a floating
    coplanar profile on the same sketch defaults to a **New body** (#1204); the context pane shows three (icon-labelled) choices whenever the **Extrude tool** is
    active — including *before* a face is picked (#587), defaulting to **New body** with Add/Cut
    disabled until a host body is known — **New body**, **Add to `<body>`**, and **Cut `<body>`** — to
    override the choice
    (editing can also split a merged/cut extrusion back out into its own body). A **Symmetric**
    toggle (extrude half the distance to each side of the sketch plane) sits alongside; like the
    body-mode choice it shows for the whole Extrude tool and is **sticky** — set before a face is
    picked, it's remembered (`AppState::pending_extrude_symmetric`) and seeds the next extrusion
    (#587). The **Cut** option
    is only offered when the OCCT kernel is compiled in, since a non-kernel build can't perform
    the subtraction (see §3.3). **A cut must bite (#380):** committing a cut first checks
    (kernel builds, `extrude::cut_tool_bites`) that the tool solid actually overlaps the
    target body — a positive distance on a side face points *out* of the solid, which used to
    commit a silent no-op cut. An outward cut whose flipped direction would bite is
    **auto-flipped inward** (the commit-time analogue of the backward-drag auto-cut) with a
    status note; one that can't remove material in either direction commits as given with a
    **status warning**. Target-driven or expression-bound depths are never flipped, only
    warned. **Auto-cut on backward drag (#141/#1204):** only when a profile **actually sits on**
    the host body face (nonzero UV overlap) — not merely coplanar on a sketch started from a
    body face. That body lies on the negative-normal side, so dragging the extrude gizmo
    *backward* (negative distance) drives an on-face profile into it — the mode auto-switches
    to **Cut** of that body; pulling forward again reverts to **Add to**. A floating coplanar
    profile defaults to **New body** and is never auto-cut (nothing to cut into). This only
    flips the cut toggle (an explicit **New body** choice is left alone on forward drags) and,
    like the manual Cut option, only engages when the OCCT kernel is present. **Combine/merge shadows the host
    (#1106/#1107):** committing **Add to** leaves the host as a shadow body (like Move/Boolean)
    and creates a new live solid whose source is the host fused with the extrusion; that solid
    nests under the extrusion as its graph output, and the host feeds the extrusion as a
    graph input. Repeated merges chain this way (each prior body shadows, each extrusion
    outputs the next combined solid). **Cut** still mutates the host in place. Deleting one
    extrusion of a multi-extrusion body only drops that extrusion's contribution — the body
    survives as long as it still has at least one added extrusion. Scriptable via
    `bearcad.extrude{ ..., body = "add" | "cut" | "join" }` (`"add"` joins the host body,
    `"cut"` subtracts from it, `"join"` puts every profile in one body). An explicit
    `"add"`/`"cut"` requires the sketch to sit on a body face: with no such body it is a
    hard error (#178), never a silent fall-through to a new body. Omitted / `"new"` creates
    a new body. Unknown values error and list the accepted names (#1860).
  - **One extrude, several solids (#837):** an extrude's picked profiles are grouped into the
    solids they actually make (`extrude::disjoint_face_groups`) — profiles that touch (nested,
    like a hole in its own wall, or overlapping) stay in one solid; profiles sharing nothing
    are separate ones, and every glyph of one sketch text counts as one label. Under **New
    body** each group becomes its own extrusion and body; **Join** (`ExtrudeBodyMode::JoinNew`,
    `body = "join"`) keeps them in a single extrusion and body. The Output picker's second
    button is *Join <host body>* when the sketch sits on a body face and *Join the profiles
    into one body* when it doesn't (enabled only when there is more than one group).
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
    `crate::polygon_boolean::face_boolean` (#88): it **delegates to OCCT** (planar
    faces on z=0, `BRepAlgoAPI_Cut`/`Common`, result accepted only as exactly one hole-free
    face whose outer wire is walked in loop order); the hand-rolled two-simple-polygon
    Weiler-Atherton clip (`Difference` reverses the clip polygon's winding — the standard
    trick that turns the same intersection-walk into a subtraction) is retained as the
    reference implementation for a parity test matrix holding both to the same strictness
    contract. The resolved loop feeds mesh generation, fill rendering, and
    hover-highlighting the same way a `Polygon` face's loop already does. Scriptable via `bearcad.extrude{ boolean = { op = "intersect" |
    "difference", a = <face spec>, b = <face spec> }, distance }`, where a face spec is
    `{circle=i}`/`{polygon={...}}` (a rectangle is a four-line polygon)/a nested `{boolean={...}}`.
    - **Rings / faces-with-holes (#268/#263)**: a `Difference` whose subtrahend lies strictly
      inside the minuend (concentric circles, a shape fully inside another) is an **annulus** —
      no longer rejected. `extrude::extrude_face_uv_region` resolves such a face into a
      **`UvRegion`** (an outer loop + interior **hole** loops), and both solid builders honor it:
      the **kernel** extrudes/revolves a `Boolean` face by building each operand's solid and
      applying the same boolean to the *solids* (`Difference`→cut, `Intersection`→common), so a
      concentric ring becomes a true **tube** (outer cylinder minus inner cylinder — exact walls,
      single circular rims); the **mesh fallback** (previews, kernel failures) builds hole-aware caps
      (`polygon::triangulate_planar_with_holes`, hole loops bridged into the outer loop and
      ear-clipped) plus inner side walls. This works for extrude (`ExtrudeFace`) and revolve alike.
      **Hover highlighting is hole-aware too** (#942): the region's highlight
      (`ViewportHoverHighlight::ClosedLoop`, which carries `holes`) fills only the ring and
      outlines every rim, so hovering a wall shows the wall — not the whole outer shape.
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
- **Operation identity (#1055):** every operation lives in an `arena::Arena` — the five
  in-sketch ones (`sketch_repeat_ops`, `sketch_offset_ops`, `sketch_mirror_ops`,
  `sketch_vertex_treatment_ops`, `sketch_slice_ops`, keyed by their own `…OpKey`) as well as
  the body ones —
  `boolean_ops`, `move_ops`, `mirror_ops`, `repeat_ops`, `slice_ops`,
  `edge_treatment_ops` — keyed by `BooleanOpKey`, `MoveOpKey`, `MirrorOpKey`,
  `RepeatOpKey`, `SliceOpKey`, `EdgeTreatmentOpKey`. Those are what the matching
  `BodySource` variant, `SceneElement`, `HierarchyNode` and `ComponentMember` hold, plus
  `RepeatPlaneInstance::op` and `ExtrudeTarget::RepeatedFace`. Deleting an operation removes
  it. The rule that an operation may not consume its own result used to be spelled as an
  index comparison ("a later op"); it is now the descendant walk it always meant — the input
  must not be one of the op's outputs, nor anything built from them.
- **Body identity (#1055):** `Document::bodies` is an `arena::Arena`, and a body is named by a
  `BodyKey` everywhere it is referred to — every operation's `targets`/`a`/`b`/`outputs`,
  `SceneElement::Body` and the mesh-keyed sub-elements (`BodyEdge`, `BodyVertex`, `BodyFace`,
  `BodyCylinder`, `BodyAxis`), `MovePointRef`, `MateRef`, `JointRef::Body`,
  `DrawingView::body`, `ComponentMember::Body`, `ParameterSource::BodyEdgeLength`, and the
  per-body mesh caches. Deleting a body **removes** it, so a reference to it reads as gone
  rather than resolving to whichever body used to sit after it. `shape_order` and
  `undo_groups` stay plain creation-order counts — a history tape, not an identity table.
  A script still names a body by its **ordinal** among the live ones, resolved to a key at
  the script boundary.
- **Extrusion identity (#1055):** `Document::extrusions` is an `arena::Arena`, and an
  extrusion is named by an `ExtrusionKey` — `BodySource::Extrusion`/`Extrusions`/`Solid`'s
  add and cut lists/`UnitCut`'s cuts, `FaceId::ExtrudeCap`/`ExtrudeSide` (so every sketch
  hosted on a cap or side wall names its host by key), `SceneElement::Extrusion` and
  `ExtrusionEdge`, `HierarchyNode::Extrusion`/`EdgeTreatment`, `ComponentMember::Extrusion`,
  a repeat op's `extrusion_targets`, and an edge-treatment op's treated edges. Deleting an
  extrusion removes it; a body built solely from it goes too, and a body that merged it with
  others just drops it. A script names an extrusion by its **ordinal** among the live ones,
  resolved to a key at the script boundary.
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
  is the default (#89). The cut list round-trips through save/load regardless of build. The
  context pane's **Output** row (Extrude, Revolve, Sweep, Loft, Mirror) shows **Y** next to
  the field name (#1397 / #1592), the same parenthetical form as Combine's Mode (Y); the
  New/Join/Cut icons match Combine Mode size.

- **Combine tool (whole-body booleans):** operates on committed bodies rather than
  extrusions. Four operations: **Combine** (union of the picked set), **Cut** (A − B),
  **Intersect** (only what's common), **Difference** (symmetric difference — only what's
  *not* common). Combine uses a single picker; the two-sided operations have A and B
  pickers (multi-select each, clicking a body in the viewport toggles it into the active
  side) plus a leftovers toggle that names what each op keeps: **Keep cutting shape**
  (Cut leaves the B-side inputs live), **Keep trimmed parts** / **Keep hole** (Intersect
  and Difference add the leftover pieces as extra result solids — the same three-part
  split of A and B). Switching
  from Combine to a two-sided mode after side A is filled focuses side B. Bodies already
  selected when the tool is picked **seed side A** (#943), like Repeat and Joint. The context
  pane reads as one contiguous block — no dividers between the body pickers and the controls
  below — with the four operations as a segmented icon group in the right column of a
  two-column **Mode** row (#606). **Y** cycles the kind and is shown next to the field
  name (#1534 / #1579), the same parenthetical form as Name (N).
  - Committing creates a **boolean operation element** (`Document::boolean_ops`,
    `ShapeKind::BooleanOperation`) and one **output body per result solid**
    (`BodySource::Boolean { op, solid }`) — a cut or difference that severs a body into
    pieces yields one body per piece. The output count is fixed at commit; a parametric
    rebuild that produces *more* solids folds the extras into the last output body, fewer
    leaves trailing outputs empty, so the Elements pane stays stable. **New** commits run
    the kernel **off the UI thread** (#1031): the Create checkmark becomes a spinner until
    the solids are ready, then the op is written and the first paint uses pre-warmed meshes.
    (Re-editing an existing op stays synchronous.)
  - The input bodies become **shadow bodies** (`Body::shadow`): still listed in the pane
    with their own dashed-cube icon, but hidden in the viewport (and excluded from picking
    and occlusion) except while hovered or selected in the pane, where they render as a
    translucent ghost with a wireframe. Hovering the operation row ghosts all of its
    inputs at once. The same flag is available by hand (#1218): right-click any body in
    the Elements pane → **Make shadow body** / **Make live body** (scriptable as
    `bearcad.set_body_shadow{ body, shadow }`). Shadow bodies are omitted from whole-document
    and component export.
  - Dependencies: outputs nest under the operation in the pane; the operation depends on
    its inputs (enforced: an operation may only consume bodies that exist before it —
    outputs of *earlier* operations are fine, so booleans chain acyclically; shadow
    bodies can't be re-picked unless the operation being edited already owns them).
  - The operation element is **editable**: selecting it offers "Edit operation", which
    re-opens the pickers (kind, sides, leftovers) and applies in place, re-shadowing inputs
    accordingly. Deleting the operation removes its outputs and releases its inputs
    from shadow (unless another live operation still consumes them). Undo of a commit
    restores inputs and removes the operation and its outputs as one step.
  - **Live result preview (#1033):** as soon as every side the operation needs is picked
    (two bodies for Combine, both sides otherwise), the side-A bodies are hidden and the
    solids a commit would build render in their place, translucent in the shared preview
    style — so the hole a cut takes out is visible before committing it. The B side keeps
    its red cut highlight, standing in for the tool, exactly as an extrude cut previews.
    The kernel result is cached per (document, kind, sides), so it is computed once per
    pick rather than once per frame.
  - Scripting: `bearcad.combine{ op = "union"|"cut"|"intersect"|"xor", a = {…},
    b = {…}, keep_b?, name? }` (`difference` means cut, same as 2D) and
    `bearcad.edit_combine{ index, … }`;
    session-command export replays both. `bearcad.ui.begin_combine{ op, a, b, keep_b? }` arms
    the tool with
    picked sides **without** committing, so a script can drive the result preview — the
    counterpart `begin_move` gives Move. The result geometry is kernel-computed (`xor`
    is (A∪B) − (A∩B); multi-solid results split via `Shape::solids`), on desktop and web
    alike via the kernel module.

- **Move tool (#176/#183):** shortcut **M** (#665) — pressing it again while the tool is
  already active **cycles the translate mode** (Snap ⇄ Free), the same way `R`/`O` cycle the
  rectangle/circle anchors. A rigid **translation** of whole bodies — rotation was pulled
  back out for now (#663), so the tool translates only. One multi-select body picker (viewport
  clicks toggle); translation X/Y/Z are **expressions** (parameters work — moves rebuild
  parametrically).
  **Move mode (#648/#1076, `model::MoveTranslateMode`):** a pane dropdown picks one of four.
  **Point Snap** (the Move tool's default) derives the placement from picked points; **Face
  Snap** (the Joint tool's default) puts a face on a face; **Free** takes typed/dragged X/Y/Z
  amounts *and* X/Y/Z turns; **In place** is the identity, and is offered by the **Joint tool
  only** — a joint often just needs saying "leave the part where it is", while a Move that
  moves nothing is a no-op rather than a mode (`MoveTranslateMode::for_tool`). In place takes
  no picks and no values, and the pane says so by offering no rows, not by a sentence.

  Free's turns (`rx`/`ry`/`rz`, degree expressions) act about the **moving part's own
  centre**, X then Y then Z, before the translation carries it away: typing "45° about Z"
  means "turn it where it stands", not "swing it around the world origin".

  **The third pair can be an angle (#1078).** Point Snap's C pair pins the spin the B pair
  leaves free. It can be set by picking an end point C, or — the same thing said differently —
  by typing a `roll` **angle**: the spin about the `endA → endB` axis is simply that many
  degrees, and no third point is needed at all. The pane offers whichever the user reaches for
  first, showing the Roll field while no end point C is picked. A picked end point C wins if
  both somehow end up set: it says where the part should *face*, where a number only says how
  far to turn it, so the more specific answer is the one honoured.

  **Face Snap (#1077).** Two staged pickers (#1075) — a face on the moving part and a point
  within it, then a face on something fixed and a point within that. The picked points *are*
  the mate points: a `MovePointRef::OnFace` already carries the face it lies on, so nothing
  else has to be stored to remember which face was chosen, and they land in the same
  `start_point_a`/`end_point_a` slots Point Snap uses. A viewport click takes the **exact
  spot** under the cursor rather than the nearest corner — snapping it to a corner would
  quietly change which mate you asked for.

  The placement is then: land start A on end A, turn the moving face's normal to meet the
  target's, and spin about the target's normal through the mate point. `face_flip` chooses
  the side — the default opposes the two normals so the surfaces touch, which is what "put
  this face on that face" nearly always means; flipped, they point the same way and the part
  sits behind the face. `face_spin` is the turn, a degree expression, set either in the pane
  or by dragging a **yellow ring gizmo** centred on the mate point and normal to the target
  face, with its 0° handle on a world axis in that plane — the same control, two ways in.
  The A→A connector is a **smooth bezier** that leaves each face along its outward normal
  (even with no extra turn). The ghost preview works as it does for Point Snap. Scripted as `flip? = true` and `spin? =` on `move_bodies`; a move naming
  two `on_face` points and no B pair *is* a Face Snap, while a B pair says the turn comes from
  a second point pair, so a script written before Face Snap existed still means Point Snap.

  Point Snap derives the offset from two picked points:
  - A **Start point A** picker (#649/#668) takes a corner, the midpoint of a feature edge,
    or a point on a planar face (#738/#1074) on one of the **moving** bodies
    (`model::MovePointRef`, keyed like `SceneElement::BodyVertex`/`BodyEdge`/`BodyFace` and
    resolved against the live mesh — a point on a face re-finds its coplanar group by
    quantized centroid+normal, `MovePointRef::OnFace`). While one is set the moving bodies render
    **translucent** (they join `faded_bodies`) so the gizmos and points stay visible through
    the solid.
  - An **End point A** picker (#650/#668) takes the same kinds of point on a body that
    **isn't** moving; the translation is then `end - start`, and the X/Y/Z fields and drag
    arrows are hidden. Destination picks ignore the moving bodies (#1336), so a click
    goes through them as if they were not there. Every **end** picker (A, B, C) also takes the **world origin**
    (`MovePointRef::Origin`, #946) — a fixed point of the document that belongs to no body, so
    it counts as stationary and always resolves. It wins over the geometry under it, the way a
    sketch's origin beats a nearby edge, and reads as "Origin" in the picker. Scripted as
    `{ origin = true }`. With both picked a **yellow connector** (#740, `MOVE_CONNECTOR`) is
    drawn between them (`move_snap_connector` → `ViewportSceneInput::colored_segments`), so
    the translation reads as a vector apart from its green/red endpoint marks.

  **Esc** drops the in-progress move (#749) — picked bodies, snap points, and with them the
  destination ghost — leaving the tool armed fresh; a second Esc returns to Select through
  the ordinary tool-switch cleanup (`Action::SetTool`), so no tool's cancelled picks survive
  to resurface on its next visit.

  **Live preview and point marks (#660):** an in-progress move **ghosts** each picked body at
  its destination through the same translucent preview-solid path the Mirror and Repeat
  previews use (`repeat_ghosts`), so a snap translation shows where it lands before commit; a
  move that resolves to the identity draws nothing. Every ghost's **feature edges** also draw
  into the always-on-top wireframe overlay (#743): a destination flush against — or embedded
  in — stationary geometry would otherwise be swallowed by the depth test, and the visible
  remainder read as landing offset from the snap target. The B pair carries through to the
  ghost (#748), so a completed rotation previews too. The picked points are marked in the
  viewport in colors of their own (`ViewportSceneInput::colored_pick_highlights`): **start
  point A green** and **end point A red** — go and stop, so the direction of the snap reads at
  a glance — and **both B points and both C points in candidate blue** (#748). A complete pair draws the
  point's **path**: a dashed curve from start B to end B, in the same candidate blue as
  its endpoint marks, tracing where the point travels
  with the slide and the turn advancing **together** — half way through the translation it
  is half way through its rotation (`move_b_path_points`:
  `p(t) = startA + t·T + R(axis, t·angle)·(startB − startA)`, sampled into
  `colored_segments`). The committed operation itself still translates then rotates; only
  the drawn path blends them.

  **One focused picker, stepping through (#656/#658/#659):** exactly one Move picker is armed
  at a time — it's the one the pane rings and the one the viewport hover-highlights, so what
  lights up is always what a click takes (`MoveFocus`, `move_focus_for`). The tool **advances
  on its own**: Bodies until one is picked, then Source point, then the Target point when
  snapping — and with the A pair set the chain walks straight into **Start point B** (#741),
  the rotation opt-in being the likeliest next click (Bodies stays a hand-focus away), and on
  into **Start point C** once B completes.
  Clicking a picker overrides the chain until that picker is satisfied
  (`move_focus_satisfied`), then it resumes. While a point picker is armed the hover marks
  the exact candidate **point** (#739) — the corner, the edge's midpoint, or the face's
  middle (#738) — never the whole edge or face it sits on.

  **A point within a face (#1074).** `MovePointRef::OnFace { body, centroid, normal, uv }`
  names a point lying **on** one of a body's planar faces — the vocabulary a Face Snap mate
  needs, and the general case of the face **centre** that used to be the only face point
  (#738), which is simply `uv == [0, 0]`. The face is keyed like every other face reference;
  the point is an offset from the face's centre **in the face's own plane**
  (`construction::plane_basis` of its normal, so it is the frame a sketch on that face would
  get). Holding it in the face's axes rather than in world space is what lets the same offset
  keep naming the same spot after a rebuild moves or resizes the face — a world position
  would be left behind. Scripted as
  `{ body =, on_face = {x,y,z}, normal = {x,y,z}, uv? = {du, dv} }`, with no `uv` — and the
  older `face_center =` spelling — meaning the middle.

  **The point pickers are real element pickers (#955).** Each takes a single point — a body
  corner, an edge midpoint, or a planar face's middle (`SceneElement::MovePoint`) — and carries
  the side rule as a `PickRule` (#953): a Move **start** point must land on one of the *moving*
  bodies and an **end** point on anything else (`OnBodies` / `OffBodies`); a Joint **start**
  point must land on the *driven* part and an **end** point on the *base*. The rule lives in the
  picker, so the pane, the viewport hover, and the click path can't disagree about what a valid
  pick is.

  **A picker can take two elements in sequence (#1075).** `ElementPicker::face_then_point`
  takes a **face** first, and from then on takes only **points lying on that face** — its
  selectable set changes as it is used, which no other picker's does. The second stage is a
  filter plus a `PickScope` saying how the first pick narrows it; the picker injects
  `PickRule::OnFace(face)` itself, since nothing outside can know the face in advance.
  Everything asking what a picker takes goes through `ElementPicker::accepts` /
  `active_filter`, so the pane's icons, the viewport hover, the click path, and the
  **Selection Exploder**'s crowd fan all follow the stage without knowing it exists — which
  is exactly why this lives in the picker rather than in a bespoke two-click interaction.
  "On the face" is geometric: any point resolving into the face's plane and inside its
  outline counts, so a corner or an edge midpoint on that face passes and one on the far side
  of the body does not. Dropping the face drops the point with it, and picking a second face
  starts the sequence over rather than reading as full.

  **The chain is generic (#954/#962).** A tool declares its pickers in order with whether each
  is filled (`FocusChain`); focus is the first unfilled one (`focus_chain_step`), which is how a
  single-pick picker hands focus to the next input the moment it's filled. A hand-picked focus
  pins the chain until that picker is satisfied (`focus_chain_satisfied`) — except on the
  tool's **primary** picker (the chain's first entry), where focusing by hand means "I want to
  keep adding to it", so it is never auto-released. The Move and Joint tools were the same
  algorithm written out twice, seven states and nine; they now declare a chain each
  (`move_focus_chain` / `joint_focus_chain`) and share the walk. A Free move has no point
  pairs, so its chain is just the bodies and its **Reference Point** (#1235; same picker
  target as Start point A, different label — Free has no A/B/C pairing); the Joint tool's chain runs the parts, the
  mate's two faces (#1021/#1079), and then — only when the placement named no plane to seed
  it from — the joint's own axis; its slide stops (#896) are hand-focused from the pane rather
  than stepped into, so they sit outside it.

  - An optional **second pair (#669)**, **Start point B** on the moving bodies and **End point
    B** on stationary geometry, adds the **rotation** — the pane labels the B and C rows
    **Rotation** (#915), since those four points turn the part rather than move it, and
    heads them with an **Angle snap** row (#917): a 0–90° slider beside a value field, both
    clamped to that range, defaulting to 90° and sticky across moves
    (`AppState::move_angle_snap_deg`, `bearcad.ui.angle_snap(degrees)`). It sets how far apart
    the rotation's candidate dots sit (#918): on the End-B sphere, one for every direction
    that many degrees apart about the world axes (`snap_angle_sphere_candidates` — 90° gives
    the six axis directions, 45° gives 26: two poles and three rings of eight), and on the
    End-C circle one every that many degrees around it (`SpinCircle::spots`, starting at the
    no-spin position). They're offered alongside the geometry-derived spots, through the same
    blue/gold marks and dashed pivot guides. **Hovering one draws the sweep that reaches it**
    (#919), in the candidate gold with the angle in degrees at each arc's middle (painted after
    the viewport's GPU callback, #947, or the scene buries the label): for end
    point B the **azimuth** turned in the ground plane and the **elevation** lifted out of it
    (`move_direction_sweeps`), for end point C the signed **spin** about the A→B axis
    (`SpinCircle::sweep_to`). Once the snap is fine enough that the dots would be a cloud the grid
    stands down and the **surface itself** is shown instead (#920) — **under 30°** for end point
    B (`ANGLE_SNAP_SPHERE_DEG`, #950: a sphere carries ~`(180/step)×(360/step)` dots, so even 15°
    is hundreds) and **5° or finer** for end point C (`ANGLE_SNAP_CIRCLE_DEG`, whose one ring of
    `360/step` stays readable much further down): end
    point B draws the constraint sphere as a translucent ghost solid, end point C its circle
    as a ring, and the cursor's own point on that surface — its ray hit rounded to the angle
    step (`ray_sphere_point`, `snap_direction_to_angle`) — is the candidate, so the sweep
    arcs read the angle out live as the cursor moves and a click anywhere takes it: after the A translation lands start A on
    end A, the bodies turn **about end point A** by the shortest rotation taking start B's
    direction onto end B's (`extrude::move_snap_rotation`). End B is confined to the
    **constraint sphere** centred on end point A with radius `|startA - startB|`
    (`snap_rotation_radius`) — a turn about end A can only swing start B around that sphere, so
    an off-sphere pick is refused (`snap_rotation_reachable`, ±0.05 mm for the quantisation).
    Re-picking start B resizes the sphere and clears end B; clearing start B clears end B too.
    The pair stays optional — committing without it translates only — but the focus chain
    arms Start point B as soon as the A pair completes (#741). While that picker is armed, **every spot a stationary body's feature edge crosses
    the sphere** is offered as a **blue** candidate mark (`snap_rotation_candidates` — the roots
    of the edge/sphere quadratic), and **edges whose line passes through end point A extend
    straight out to the sphere** (#745, `snap_rotation_axis_candidates`): mid-air landing
    spots along those directions, each with a **dashed guide** drawn from the pivot
    (gold when its spot is hovered). Every unhovered candidate — dot and guide alike — is
    **color-coded by the axis its turn goes about** (#949, `snap_rotation_axis_toward` →
    `rotation_axis_color`: the world axis the rotation axis lies nearest, in that axis's own
    color), so a sphereful of spots reads as groups instead of one indistinguishable blue;
    end point C's candidates all spin about the same A→B axis, so they stay candidate blue.
    The candidate under the cursor reads **gold**; hovering
    it previews the move it would produce, and clicking takes it. Candidates sit mid-edge or
    in the air rather than on a corner, so they're kept as `MovePointRef::OnEdge` (their own
    quantized world position) instead of being re-found by matching. The generic
    corner/edge/face hover stands down entirely while this picker is armed (#744) — only the
    candidates are pickable, and they mark and glow on their own (`MovePickHover::EndB`).

  - An optional **third pair**, **Start point C** on the moving bodies and **End point C**
    anywhere, pins the one freedom the B pair leaves: with start B lined up on end B the
    bodies can still **spin about the `end A → end B` axis**, and C decides that turn
    (`extrude::move_snap_roll_axis_angle`) — with all three pairs the placement is fully
    determined. The spin is the signed angle about that axis taking the already-translated,
    already-rotated start C onto end C, measured from their directions **flattened onto the
    plane perpendicular to the axis**: only C's bearing about the axis is C's to decide, since
    how far along the axis and how far out from it are already A's and B's. So — unlike end
    point B — **any** end C gives a well-defined answer and none is refused. Start C does
    ride a **circle** about the axis, though (#914): while End point C is armed, four spots a
    quarter turn apart on it are offered as the same blue/gold candidate marks the B pair
    uses, each with a dashed guide from the circle's centre (`extrude::snap_spin_candidates`).
    The first is the **no-extra-spin** position — start C carried over by the minimal rotation
    between the two axes — and the rest follow at 90°, 180° and 270°. They sit in mid-air, so
    they hang off end point A's body like the mid-air end-B spots, and clicking one takes it. A start C **on the axis itself**
    has no bearing to line up, so no spin is derived and the move is what B alone gives.
    Re-picking or clearing start C clears end C, the way B cascades, and the focus chain arms
    Start point C once the B pair completes.
    **Hover previews both end pickers:** with End point A or End point B armed, hovering a
    valid point shows the ghost as if that point had been chosen (`move_hover_probe` — a
    probe copy of the in-progress move with the hovered point filled in). A C pair that's
    already set **rides along** in that probe (#948), since taking the hovered end B keeps it;
    a C pair the new axis can't satisfy simply derives no spin, so the ghost falls back to what
    B alone gives. The ghost's pose
    **eases** toward its target (`move_ghost_pose`, exponential glide,
    `MOVE_GHOST_EASE_SECS`), so hopping the hover between candidates reads as the body
    sweeping over — quick, but smooth — never teleporting. The ease is a rotation **about the
    A pair's pivot** (#826): the pose carries the pivot's destination and the rotation, and
    the matrix is rebuilt as `translate(end A) · R · translate(-start A)` each frame, so start
    A sits exactly on end A for the whole animation. Pulling the hover off mid-flight simply
    changes the target, so the same ease runs it back where it was.
    The **Selection Exploder** follows suit (#746/#747): with End point B armed its fan
    offers exactly the sphere candidates — each loupe a blue dot — never faces, edges, or
    corners the picker can't take; and a loupe whose content is bare point dots skips the
    faint grey orientation mark other loupes carry.

  A Snap move with either A point still unpicked — or with no bodies at all, as for a plane or
  image move — falls back to its `tx`/`ty`/`tz` expressions
  (`MoveOperation::has_snap_translation`), so the tool stays usable mid-pick and gizmo drags
  keep working; only a *resolved* snap is excluded from move coalescing. Committing creates an editable **move
  operation element** (`Document::move_ops`, `ShapeKind::MoveOperation`) with one moved
  output body per input (`BodySource::Moved { op, target }`); inputs become shadow bodies,
  exactly like the Combine tool. "Edit move" re-opens the tool (outputs grow/shrink with
  the target list; removed ones go away). Meshes transform on every target (works in the
  lean build); the BREP shape transforms through the kernel (`Shape::transformed`,
  `bearcad_shape_transform` natively and in the web kernel module) so moved bodies chain
  into booleans and export as real BREP. **Translation drag gizmos (#215):** with bodies
  picked, three axis arrows (X red, Y green, Z blue) at the targets' bounding-box centre drag
  to set the translation — the same offset-arrow handle as the extrude gizmo, driving the
  `move_x`/`move_y`/`move_z` values (so scriptable/testable via the gizmo API, §8).
  Each free-translate arrow also carries a **value input floating beside its handle** (#648),
  so a component can be typed where it's being dragged. Scripting:
  `bearcad.move_bodies{ bodies = {…}, images = {…}, x?, y?, z?, rotate?, from?, to?, name? }` and
  `bearcad.edit_move{ index, … }`; naming both `from` and `to` makes it a snap translation
  (`{ body = i, vertex = {x,y,z} }` or `{ body = i, edge = {{x,y,z}, {x,y,z}} }`, millimetres
  on the body's mesh). `from`/`to` are lists of mate points: a second pair adds the rotation,
  a third the spin. `rotate = { x, y, z }` is the free-mode turn. `bearcad.ui.begin_move{ … }` takes the
  same arguments but **arms the tool instead of committing** — the picks land in
  `creating_move` with the Move tool up, so a script can drive the live preview (the ghost,
  the A connector, the B and C paths) rather than only the finished operation. A point table takes
  `vertex`, `edge` (its midpoint), or `on_edge` (a position along one). **Moving construction planes (#217):** a Move op can also
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
  Dropping an image from a move restores its authored base. Images join the move set from a
  viewport click on the quad, the Elements pane, or the selection, like bodies. An image-only
  set switches the tool to Free so the translation gizmos come up (#1587). **Coalescing (#217):** re-moving the same
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

- **Mirror tool (#523/#566):** reflects whole bodies across a mirror plane. The **first** viewport
  click picks the mirror plane — a construction plane or a flat body face (`pick_sketch_face`,
  the same planar-face pick sketching uses); subsequent clicks toggle **bodies** into the
  reflected set. The context pane shows **both** the plane and the bodies through the unified
  element picker: a single-pick **Mirror plane** picker accepting planes or flat faces
  (`ElementKind::Plane`/`ElementKind::Face`, `PickerTarget::MirrorPlane`, whose empty state shows
  the dedicated plane and **face** glyphs — `IconId::Face`, not the body icon) sits above the
  bodies picker, each with the standard **✕**-to-clear rows. The Mirror plane picker, the Bodies
  picker, and the Do button read as **one contiguous block** — no dividers between them (#602).
  The plane picker is the focused one until a plane is set, then focus moves to the bodies picker.
  While picking, the viewport **hover-highlights** exactly what a click would take: the plane or
  flat face under the cursor before a plane is set, then the whole body afterwards (#604/#605).
  A translucent **ghost** of each reflection previews live before commit, rendered through the
  same GPU preview-solid path as the extrude/repeat ghosts (`build_viewport_scene_input`, #603).
  **Enter** commits, creating an editable
  **mirror operation element** (`Document::mirror_ops`, `ShapeKind::MirrorOperation`) with one
  output body per input (`BodySource::Mirrored { op, target }`). An **Output** row (#639 / #1592) —
  the same segmented New body / Join body / Cut icon group (Combine Mode size), **Output (Y)**
  label, and placement the Revolve tool uses — chooses how each reflection lands (`model::MirrorMode`):
  - **New body** (default): the reflection is its own body and, unlike Move, the
    **originals are kept** — a mirror *adds* alongside the source, so inputs are never
    shadowed; deleting the op removes only the reflections.
  - **Join** / **Cut**: the output is its source fused with (or minus) the reflection — there's
    no body to pick, each reflection combines with **its own** source — and that source is
    consumed into the output as a shadow body, the way Move and the edge treatments consume
    theirs (`MirrorMode::consumes_input`, `set_mirror_input_shadows`; re-editing back to New
    body releases it again). These are real kernel booleans, so the mesh path tessellates the
    result instead of taking the cheap transform-only route.

  The reflection is a Householder
  transform across the plane (`mirror_op_transform`, determinant −1): the BREP shape transforms
  through the kernel (`Shape::transformed`) so mirrored bodies chain into booleans and export as
  real BREP, and the mesh path reflects each triangle with **reversed winding** so normals stay
  outward. "Edit mirror" (double-click / the pane button, or a lone selected mirror op) re-opens
  the tool with its plane + bodies loaded; outputs grow/shrink with the target list (removed
  ones go away). Scripting: `bearcad.mirror_bodies{ plane = <face|plane-ordinal>, bodies = {…},
  output? = "new"|"join"|"cut", name? }` and `bearcad.edit_mirror{ index, plane, bodies,
  output? }`; `plane = 0` is construction-plane ordinal 0. The default output stays implicit so existing scripts round-trip unchanged. In the elements graph the plane's body and
  every input body feed the Mirror node, and each reflected body nests beneath it.
  - **In a sketch (#528):** the Mirror tool reflects **sketch geometry** instead. The first
    click picks a **straight sketch line**, a **sketch origin axis** (LX/LY), or an
    in-plane **world axis** as the mirror axis; further clicks toggle lines and
    circles into the reflected set; a translucent preview shows the reflections; **Enter**
    commits. Committing creates a **`SketchMirrorOperation`** (`ShapeKind::SketchMirrorOperation`,
    `Document::sketch_mirror_ops`) whose reflected lines/circles are separate entries nested
    under the op and regenerated (`rebuild_sketch_mirror`) whenever the sources or the mirror
    line change — the same output-slot reuse as the offset op, so indices stay stable. A line
    reflects endpoint-for-endpoint (bezier handles included); a circle reflects its centre,
    radius unchanged; the mirror line itself is never reflected. The context pane shows the
    mirror-line label (with a ✕ to re-pick) and a Shapes element picker. Editable via
    double-click / "Edit mirror". Scripting: `bearcad.mirror_sketch{ sketch, line, lines,
    circles }` / `bearcad.edit_sketch_mirror{ index, … }` (`line` is a sketch-line
    ordinal, `"x"`/`"y"` for the sketch origin axes, or `"gx"`/`"gy"`/`"gz"` for a
    world axis that lies in the sketch plane).

- **Linear repeat tool (#182/#257):** copies of whole bodies spaced along an axis, chosen with
  a single-pick **element picker** (#955) taking one straight reference — a world X/Y/Z axis, a
  sketch line, or a **feature edge of a body** — or a **circle** to ride round (#840); the ✕
  clears it (#257/#643). Whether the copies follow the path or turn about it is the **Repeat**
  toggle directly below, so the picker row names the path rather than repeating "Along"/"Around"
 — the picker is the only way in, the
  X/Y/Z quick buttons having been dropped as a second, inconsistent path. While the axis picker
  is the focused one, the viewport hover switches from whole bodies to those straight
  references, so every pickable axis lights up under the cursor; once an axis is set, a click on
  a body's edge goes back to toggling that body. Origin axes and body edges only resolve as an
  axis in that state, keeping body picking unambiguous. **Pane polish (#440–#447):**
  the Gap/Distance measure-toggle icons hover **gold**, and so do their **labels**, which are
  the same click target (#640); each of Count/Gap/Distance carries a **lock** — a green one
  (`theme::LOCKED_ACCENT`) on the value the app computes, grey (`theme::UNLOCKED_GRAY`) on the
  two the user sets, and clicking a grey lock moves the green one there
  (`CreatingRepeat::set_computed`, #642); the three fields render at the **same width** in both
  states so the input column never jumps (#641);
  editable fields are expression inputs with autocomplete/error display and a `= value`
  computed preview beside them; the "N instances" label is gone (Count shows it) and the
  commit button sits in the input column. A **distance gizmo (#644)** hangs off the targets'
  start plane along the axis — the same click-to-grab arrow+handle the Extrude tool uses
  (`repeat_gizmo_anchor` for the anchor, `offset_gizmo_hit`/`offset_from_normal_drag` for the
  grab and drag): one click grabs it, it then follows the cursor writing **Distance** live
  (which also makes Distance one of the two *set* variables), and the next click releases it.
  It shows the computed span while Distance is the computed variable, so the handle always sits
  at the real end of the pattern. Grabbing it also **focuses the Distance field** (#655), so the
  pane's ring follows the value being dragged rather than staying on the Bodies picker and a
  precise value can be typed straight after; **Enter** still commits from any of the tool's
  value fields. **Esc** drops the in-progress repeat (clearing
  the ghost previews, #450). **Selection seeding (#439):** activating the tool seeds its
  targets from the current selection (bodies/planes/sketches), the axis starts **unset**
  (`CreatingRepeat::axis: Option` — commit refuses without one), and exactly one picker reads
  focused: the axis while it's unset and targets exist, the bodies picker otherwise — and
  **neither** while a value field holds the keyboard (#646, `RepeatControl::value_field_focused`).
  The Default-units section is hidden while the tool is active.
  One multi-select body picker; the original stays as instance 0; each
  further instance of each target is an output body (`BodySource::Repeated { op, target,
  instance }`) nested under an editable **repeat operation element**
  (`Document::repeat_ops`, `ShapeKind::RepeatOperation`). **One kind of body (#1116):** a
  repeated output is sketchable and extrude/revolve-able on its own faces just like the
  original — its flat faces pick as `FaceId::RepeatedFace` (source analytic face placed by
  the instance transform), not a second-class mesh-only surface. The context pane exposes three
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
  picked bodies' meshes translated to each `repeat_offsets` offset). A **replayed extrusion**
  (#220) ghosts too (#990): it makes no output body, so the body-target loop never saw one and
  picking a cut to repeat previewed *nothing at all* until commit. Its ghost is the extrusion's
  own prism (`extrude::extrusion_mesh`) — the tool that carves the hole — parked at each extra
  placement, so a repeat holding only cuts still previews. Instance counts
  clamp at 512. End-to-start measurements use the targets' combined extent along the axis.
  "Edit repeat" re-opens the tool and resizes the output list. The fill length `L` may instead
  be **bound to a target's extended plane** (`RepeatOperation::length_target`, an
  `ExtrudeTarget` like an extrusion's "up to face" #126): `L` is then the along-axis distance
  from the pattern start to that plane and follows the face if it moves, overriding the `length`
  expression (#186). The pane exposes that as a **"Distance to" element picker** (#645), the
  Repeat tool's version of the Extrude tool's "Up to": focus it and the next viewport click
  takes a construction plane, a face, or a vertex (`pick_extrude_target`, shared with Extrude);
  Distance becomes one of the two *set* variables so the fill reads it, its field goes read-only
  showing the measured length, and the distance gizmo (#644) follows the target. The picker's ✕
  hands Distance back to its expression. Scripting:
  `bearcad.repeat_bodies{ bodies, axis, mode, count?, spacing?, length?, to?, name? }` /
  `bearcad.edit_repeat{ index, … }`, where `to` is the same target table Extrude's `to` takes.
  - **Around the path (#839):** the **Path** picker (formerly Axis) is followed by a two-icon
    toggle — lay the copies out **along** the path, or turn them **around** it as an axis of
    rotation (`RepeatOperation::around_axis`). Turning reads `spacing`/`length` as **angles**
    (degrees, `extrude::repeat_angles`) and the copies are rotated about the axis rather than
    slid along it (`extrude::repeat_instance_transform` / `repeat_step_transform`, shared by
    body meshes, kernel shapes, plane and sketch instances, and the ghost preview). In the
    pane the **Distance** row becomes **Angle** (default 360°) and keeps the start/end
    measure toggle: last copy *at* the angle (stacked on the first for a full turn) or
    ending there so n items divide the sweep (360°/5 → 72° steps, #1473). The
    **Distance to** picker and the distance drag handle stand down, and the section title
    reads *Rotational repeat*. Scriptable as `bearcad.repeat_bodies{ …, around = true,
    spacing = "60deg" }` / `mode = "count_fit_ends"` for equal spacing around the sweep.
  - **Flip (#989):** a **Flip** checkbox sits under the **Path** picker — with it, not with the
    spacing, because which way to run is a property of the path and is only answerable once one
    is picked. A path has **two** directions and picking a line, edge or axis says nothing about
    which one you meant: the direction falls out of how that geometry happens to be stored, so
    half the time the copies march off the wrong way and there was nothing to say so.
    `RepeatOperation::flip` reverses all three kinds of step — the slide along a straight axis,
    the **sense** of the turn when `around_axis`, and which end a curved path is followed from
    (the polyline is **reversed** rather than stepped backwards off its start, so the copies stay
    on the path). It is applied in `extrude::repeat_offset_transform` and nowhere else: that is
    the one place a step becomes a transform, so every preview, ghost, plane/sketch instance and
    output picks it up for free. Scriptable as `bearcad.repeat_bodies{ …, flip = true }`.
  - **Along a curved path (#840):** when the picked path is a **curved** sketch line, the
    copies follow its bend instead of a straight direction: the path samples to a world
    polyline (`extrude::repeat_path_polyline`) and each instance is offset by the vector from
    the path's start to the point that far along it (`point_along_polyline`), so the gap and
    span are **arc length**. Copies keep their orientation (they slide along the curve, they
    don't rotate into it), and the items have no single direction to measure their own extent
    along, so they space centre-to-centre like plane targets. A curved path can only be
    followed — its "around the path" option is disabled and ignored. A pattern longer than its
    path runs on past the end along the last segment's direction. A **circle** is a path too
    (`RepeatOperation::path_circle`, set by clicking a circle while the Path picker is what's
    being picked): the copies ride round its circumference, keeping their orientation — the
    difference between that and the rotational mode, which turns them as it goes.
  - **Repeating construction planes (#221):** a repeat can also target construction planes
    (`RepeatOperation::plane_targets`), picked from the Elements pane / viewport with the tool
    active. Each further instance is a generated `ConstructionPlane` carrying a
    `RepeatPlaneInstance { op, target, instance }`; its cached frame is the source plane's
    *current* frame offset along the axis, so instances step along the axis (planes are
    zero-thickness, so the step is the bare gap/pitch) and follow the source if it moves.
    Instances are grouped under the repeat op in the Elements pane, and go away with it. Count
    and spacing are the same expressions/modes as body repeats; a repeat may target bodies and
    planes at once.
  - **Repeating an operation (#220/#1475):** a repeat can target an **extrusion**
    (`RepeatOperation::extrusion_targets`). A **cut** extrusion's tool is subtracted again
    (`occt_body_shape_from_indices`) to punch N holes. An **add** extrusion fused onto another
    body is fused again (`occt_fused_extrusions`) to grow N bumps on that body. A **standalone**
    add extrusion (the one that *is* a body) is promoted to a body target: each instance is its
    own `BodySource::Repeated` body, so disjoint copies are separate solids and each can take a
    material (#1474). Scripting: `bearcad.repeat_cut{ cuts = {ei}, axis, mode, count?, spacing?,
    length? }` (works for add or cut targets). The Repeat tool picks an extrusion operand by
    clicking it (Elements pane / selection → `extrusion_targets`, shown as an operation count in
    the context pane, #235); the op is a selectable/deletable `RepeatOp` whose deletion drops the
    replay (or the copy bodies, for a standalone add).
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
    parametrization as the 3D repeat). **Context pane (#835):** laid out like the 3D section one
    dimension down — an **Entities** element picker holding the picked lines/circles (rows drop
    individually or clear), a single-line **Direction** element picker (empty = the sketch's U
    axis; focusing it arms the next viewport click, and its ✕ hands the direction back to the U
    axis), then the **count/gap/distance** rows with the same picture toggles, the same green
    **lock** marking the computed one, its computed value read back in the disabled field
    (`extrude::sketch_repeat_extent`), and the blue commit button. A live dashed **ghost
    preview** of the duplicated lines/circles is drawn at every computed offset while the draft
    is in progress.
  The GUI/scripting to *pick* the length face is a tracked follow-up.

#### 3.4.4 Materials (#834)
- A **material** (`model::Material`) is a name plus a rendered color, living in
  `Document::materials`, an `arena::Arena` (#1055). Every body carries
  `Body::material: Option<MaterialKey>`; `None` means the **first material**
  (`Document::default_material`), which every document seeds as **Unobtainium** (#924/#925)
  — colored exactly like the neutral body grey bodies rendered in before materials existed,
  so nothing looks different until another is picked. A body renders
  in its material's color (`gpu_viewport::scene::body_material_fill`); selection and hover
  colors still win over it.
- **Seeded palette (#927/#928):** a new document already holds `Material::DEFAULTS` — the whole
  set is in the dropdown from the first frame, so choosing what something is made of never
  means making a material first. Unobtainium leads; the rest walk hues that **contrast with
  their neighbours** — Blue, Green, Red, Yellow, Purple, Orange, Cyan, Pink, Grey — so two
  materials picked (or created) one after the other never look alike. Every entry stays light
  enough (Rec. 709 Y > 0.35) that a shaded solid still reads as its own color where the
  lighting falls away. `Material::NEW_COLORS` — what **New material…** walks — is that palette
  minus Unobtainium. (Color-blind separation is deliberately *not* what this palette
  optimizes for; that belongs to a mode of its own.)
- **Context pane:** selecting one or more bodies (any tool) shows a **Material** dropdown —
  every material with its color swatch, plus **New material…**, which creates one (named
  `Material N`, next color from `Material::NEW_COLORS`) and assigns it to the selection. A
  body with no material of its own shows **Unobtainium** selected, like any other material
  (#924), with its **Name** and **Color** editable in place; every body using it re-renders.
  Selecting bodies whose materials differ reads *Mixed*.
- **Inherited by extrusion (#926):** a new body extruded off another body's face is made of
  that body's material (`extrusion_source_material` → `model::body_index_for_face`), so a boss
  or a lug matches the part it grows from. A sketch on a plane or a profile has no source body,
  so its extrusion starts on the default.
- Actions: `AddMaterial { name?, color?, bodies }`, `SetBodyMaterial { body, material }`,
  `SetMaterialName`, `SetMaterialColor` — each one undoable like any other edit. Persisted as
  `material` DAG nodes carrying their key (`save_arena_nodes`).
- Scripting: `bearcad.material{ name?, color? = "#rrggbb", bodies? = {..} }` and
  `bearcad.set_material{ body, material }` (`material = nil` for the default). A script names
  a material by its **ordinal** among the live ones — keys are not something you write by
  hand — resolved to a key at the script boundary (#1055).

- **Slice tool (#181):** cuts whole bodies with planar cutters. Two real element pickers
  (#955) — **Targets** (bodies, multi-select, refusing one already consumed by another
  operation) and **Cutters** (construction planes and/or planar body faces, multi-select) —
  with the focused one taking the next viewport click; clicking a picker makes it the focused
  one. The cutters are consumed by the operation, so that picker carries the **red** selected
  highlight (#213/#961) — the example this spec has always cited for the override. Each target is split independently: for every cutter the
  current pieces are divided by the cutter's plane, so *n* cutters through a body can yield
  up to *2ⁿ* fragments. Each fragment is an output body (`BodySource::Sliced { op, target,
  piece }`) nested under an editable **slice operation element** (`Document::slice_ops`,
  `ShapeKind::SliceOperation`); the input body becomes a **shadow body** exactly like the
  Combine tool, and fragments chain as ordinary bodies into further operations. The
  **Infinite cut** toggle (default on, #588) treats each cutter as an infinite
  plane; turned off, a cutter only separates material within its own face footprint. The
  slicing runs through the OCCT kernel (half-space booleans); a cutter that misses a body
  leaves it whole. "Edit slice" re-opens the tool and resizes the fragment list; the whole
  slice undoes as one step. Scripting: `bearcad.slice{ bodies, cutters, extend?, name? }` /
  `bearcad.edit_slice{ index, … }`.
  - **2D in-sketch offset:** `SketchOffsetOperation` (`Document::sketch_offset_ops`) makes
    **parallel copies** of picked sketch lines and **concentric copies** of picked circles at a
    signed distance expression. Lines connected end-to-end offset as one chain with **mitered
    corners** (`offset::offset_segments`: chains walk shared endpoints, exactly-two-segment
    joints miter via infinite-line intersection with a miter limit; T-junctions break the
    chain); **positive grows** — a closed loop offsets outward regardless of winding (signed
    area picks the normal side), a circle's radius increases; negative shrinks (collapsed
    circles clamp to `MIN_CIRCLE_RADIUS`); an open chain's positive side is left of its first
    segment. Outputs are real lines/circles (never dimension-locked, bezier flattened,
    projection stripped) nested under the op in the Elements pane — and the op itself nests
    **under its sketch** (#941) — excluded from the sketch's
    own listing, deleted with the op, and **re-offset on every geometry recompute**
    (`rebuild_sketch_offset` from `recompute_document_geometry`) so they track source drags
    and parameter-driven distances. A **construction toggle** emits the copies as construction
    geometry. **GUI**: the Offset tool (toolbar icon; outside a sketch it clicks a face to
    begin sketching, like the draw tools — the tool **survives into that sketch** since it's a
    sketch-edit tool, #594) toggles lines/circles into the pick set with hover glow. Clicking a
    **body edge** — e.g. the boundary of the body face the sketch sits on — **projects it into the
    sketch** as a construction line and adds that to the offset set (#595), so a face's own outline
    (a rectangle, say) can be offset without projecting it by hand first. Clicking a **face** takes
    all of its edges in one go (#938): a body face projects its whole boundary loop
    (`construction::coplanar_face_boundary`), and a click over open sketch space inside one of the
    sketch's own closed profiles (`face::pick_sketch_face`) adds every line of that loop. Faces and
    profiles hover-highlight like the rest. It previews the result as **solid preview-colored**
    lines — dashed only when Construction is checked (#940), matching the mirror/extrude/revolve
    preview styling — and sets the distance via an in-plane
    **push-pull handle** (an arrow gizmo drawn through the GPU scene's `arrow_gizmos` so it lands
    on top of the render, #939; dragged along the offset normal, negative flips side) or the context
    pane's expression input with computed preview; Enter or the pane button commits; Esc
    clears the picks (a second Esc drops the draft and its context block with it, #941). Selecting a committed op offers **Edit offset**, which re-opens the
    tool (and the op's sketch) with the existing inputs. Scripting:
    `bearcad.offset_sketch{ sketch, lines, circles, distance, construction }` /
    `bearcad.edit_sketch_offset{ index, … }`; selectable as kind `sketch_offset_op`.
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
    constraints (`constraint_outputs`, removed and regenerated on rebuild like the fragments) stitch
    the pieces so the loop resolves into two faces. The split pieces inherit the crossed edges'
    corner coincidences, so uncrossed neighbours attach to the correct side. This works because
    `closed_line_loops` now extracts **minimal, vertex-simple** faces: it drops self-touching cycles
    (running twice through a cut point) and any loop an internal chord subdivides (the reconstructed
    outer perimeter), so exactly the two half-faces survive — a no-op for ordinary sketches, whose
    loops are already minimal and simple. Scriptable via `faces = { {l0,l1,…}, … }`.
    **Interactive in-sketch Slice tool** (#238): with a sketch open, the Slice tool picks target
    lines/circles/faces and cutter lines with **two roles**, like the Combine tool's side-A/side-B
    real element pickers (#955; `CreatingSketchSlice`, `picking_cutter` chooses which the next viewport click
    feeds). Clicking a line/circle toggles it as a target; clicking empty space inside a face
    toggles that face (`face_loop_at_world` picks the smallest containing loop); while the Cutters
    picker is active, a click toggles a cutter line. The context pane shows both pickers and a
    Slice button; Enter commits. The draft is cleared when the tool changes or the sketch is
    exited.
  Picking side-wall faces as cutters remains a tracked follow-up (#191).

- **Shell tool (#1156):** hollows whole bodies to a wall thickness expression, optionally
  removing picked open faces (adjacent opens remove the shared wall). Two real element
  pickers — **Bodies** then **Open faces** (faces on the selected bodies only; first body
  arms Open faces). Inputs become shadow bodies; each hollowed output is `BodySource::Shelled`
  under an editable `ShellOperation`. Preview is the hollowed solid (semi-transparent).
  Scripting: `bearcad.shell{ bodies, faces?, thickness?, name? }` / `bearcad.edit_shell{ index, … }`.

- **Joint tool (#891/#894):** joins two parts — bodies, components, or unit instances
  (`model::JointRef`) — with a kinematic relationship (`model::Joint`,
  `Document::joints`, `ShapeKind::Joint`).
  **Joint identity (#1055):** `Document::joints` is an `arena::Arena`, and a joint is named
  by a `JointKey` — `SceneElement::Joint`, `HierarchyNode::Joint`, the joint icon badges,
  `JointRestCommand`, and the pose resolver's error list. Deleting a joint removes it, so a
  reference to it reads as gone rather than resolving to whichever joint used to sit after
  it; a script still names one by its **ordinal** among the live joints.
  A joint changes where things *are*, never
  their shape: at recompute the **driven** side is posed **in place** (`joints::
  resolve_joint_poses`), the way a Move's plane/image/instance targets are — in the node
  graph the joint has **two input edges and no output edge**, reading as a relationship,
  not a feature. Bodies pose at the presentation seam: the cached posed mesh
  (`extrude::body_solid_mesh`) and STEP body export (`extrude::posed_body_shape`) show
  the assembly, while feature inputs (booleans, moves) keep reading the un-jointed
  geometry (`extrude::body_solid_mesh_unposed`) — a part that needs to be cut in its
  jointed pose is served by an explicit Move.
  **Kinds (#892, `model::JointKind`):** `rigid` (the only kind that ties **more than two**
  members — a rigid group (#900) is a rigid joint with a longer member list), `slider`,
  `revolute`, `cylindrical`, `planar` (two in-plane slides + a spin about the normal),
  `ball` (three turns, no travel), `pin_slot` (slide along the primary axis, turn about
  the secondary), and `screw { lead }` (turn coupled to travel by a mm-per-turn lead
  expression).
  **Picking the two sides (#991):** the pane leads with the **Type** dropdown, because which
  kind it is decides what the rest of the section even asks for. Every kind but `rigid` joins
  exactly two parts, and which one *moves* is the whole meaning of the joint — so those are
  picked as two named single-slot pickers, **Mobile** first and **Fixed** second, instead of one
  two-slot list plus a "swap which side is held" button that made you read the joint backwards to
  check it. They **replace** the Parts list rather than sitting beside it: two pickers claiming
  the same picks would put two focus rings on the pane and hand the click to whichever was
  registered first. `rigid` keeps the plain **Parts** list and its Base swap — it ties any number
  of members and none of them moves. The model is unchanged: `members` plus `base`, the index of
  the held side, so the mobile one is simply the other; a single member with a `base` past the
  end reads as "mobile picked, nothing holding it yet". The empty slot is what the next click
  fills (`CreatingJoint::set_mobile`/`set_fixed`), so the parts step still runs mobile → fixed
  off one `JointFocus::Members`.
  **Telling the sides apart in 3D (#992):** while a two-sided joint is being made or edited, its
  parts take **different fills** — `SOLID_FILL_JOINT_MOBILE` (green) for the side that moves,
  `SOLID_FILL_JOINT_FIXED` (blue) for the side holding it — through the scene's `tinted_bodies`
  channel. A fill rather than an aura, because for a solid the fill *is* the visual; and it
  **outranks the selection blue**, which is why those members stop folding into the render
  selection while the tint is on — lighting both sides the same answered "these two", which was
  never the question. Only while previewing, and only for a two-sided kind: a Rigid group has no
  moving side to tell apart, and a committed joint's parts are ordinary bodies again.
  **Pane layout (#997/#1021):** the section is in two named parts. **Mate** says where the
  parts start out; below it, a section named for the **kind** (*Slider*, *Revolute*, …) holds
  that kind's own freedoms and the limits on them. Rigid has neither, so it gets no second
  section.
  **The mate is a move (#1021/#1068/#1079).** A joint mate is *a move — optionally a no-op,
  when the part is already where it belongs — plus the joint's limits*, so it **is** one:
  `Joint::placement` is a `MoveOperation` and the Joint tool drives the same `CreatingMove`
  the Move tool does (§7.4). Sharing the state rather than a component is what stops the two
  drifting into two behaviours. Every move mode is available: **Face Snap** is the default and
  the common case (two clicks — a face on the moving part, a face on the fixed one), **Point
  Snap** and **Free** place a part that isn't going flat onto anything, and **In place** says
  "leave it where it is" with no picks and no values.

  The pane keeps its two named parts: **Mate** says where the parts start out — the same
  **Moving face** / **Fixed face** rows, plus Face Snap's **Flip**, **Gap** and **Turn** — and
  below it **Freedom** says how the joint works, then a section named for the **kind**
  (*Slider*, *Revolute*, …) holds that kind's own limits. Rigid has no freedoms, so it gets no
  kind section.

  The mate remains a **starting placement and nothing more**: it works out to a rigid transform
  (`extrude::move_op_transform`, the same call the Move tool resolves through) that the kind's
  freedoms then act on top of, and has no bearing on how the joint moves.

  - **Only the mode's own pickers are registered (#1081).** The registered list is what
    focus, hover, the Selection Exploder and `bearcad.ui.pickers()` all read, so a row the pane
    doesn't show must not be in it: Face Snap registers its two staged rows, Point Snap its
    six point rows, Free its **Reference Point** (#1235; the handle its typed amounts move
    from), and In place nothing. Registering all six regardless left **Start point A** live
    and focused beside Face Snap's rows, so the hover offered corners while the tool was
    asking for a face.
  - **Face on face, in two picks a side.** Each of **Moving face** and **Fixed face** is a
    staged picker (#1075): the **face** first, then a **point on that face**. The face shows
    in the row on its own while its point is being chosen, and the side isn't finished until
    the point lands — which is what lets the second pick be scoped to the first, so the hover
    and the Selection Exploder offer only points on the face already chosen. Clearing a side
    drops both; picking a different face starts that side over. A *scripted* `face = {}` names
    the face alone and takes its middle, which is the sensible default when there is no cursor
    to point with.

    A face offers **nine points** to mate on (#1083, `extrude::face_snap_points`): its
    corners, the midpoint of each boundary edge, and its centre — the **area** centroid
    (#1080, `face_group_area_centroid`), which is the same point whatever way the mesh
    triangulated the face. The face's *key* averages triangle vertices instead, counting a
    shared vertex once per triangle, so on a plate with a hole it sits a couple of tenths off
    the true middle and a peg seated there came out visibly off-centre. The key keeps that
    average — every stored face reference is matched by it — but nothing that has to be
    accurate uses it, and naming a face without a point (a script's `face = {}`, a pane pick
    before its point lands) means the accurate middle — so a rectangular face gives
    the nine a user reaches for. Collinear boundary vertices are dropped, because a mesh
    splits a straight edge into several triangles and each would otherwise contribute a
    midpoint. The pick is the candidate **nearest on screen** within the usual point-hit
    radius, with no requirement that the cursor be over the face: a corner sits *on* the
    outline, so insisting on that made the very points you aim at unpickable. A spot that is
    none of the nine is not a pick — a mate lands on a feature of the face, not wherever the
    cursor happened to be. Hover and click share the search, so what lights up is what a
    click takes; the picked point is marked, and the moving-to-fixed connector draws for the
    Joint tool exactly as it does for the Move tool.

    Face Snap then lands the moving point **on** the fixed one, with the normals opposed so
    the surfaces touch. **Flip** puts
    the part on the other side, **Gap** holds it off along the normal, and **Turn** spins it
    about that normal through the mate point. This replaced the *line-up rows* that used to
    take away what a face pair left free: a mate point plus a turn pins all three of those
    freedoms directly, and says so in the same vocabulary the Move tool already used. It is
    only as exact as the meshed face's centroid, which is why mating on a hole's own centre is
    tracked separately (#1080).
  - **Grounding against the world (#1018):** the fixed side takes a datum plane, a world axis
    or the origin as readily as another part's geometry, which is how the first part of an
    assembly is placed.
  - **The freedoms' frame (#1079, `model::JointFrame`):**  - **The freedoms' frame (#1079, `model::JointFrame`):** how the joint works is its **own**
    state — an **Origin**, an **Axis** and a **Second axis** in the pane's *Freedom* section —
    not something derived from the mate. It used to be derived, and could be, because a mate
    was always a face on a face; a joint's placement is now an ordinary move (#1068) and three
    of the four move modes name no plane at all, so a joint mated by Point Snap, Free or In
    place would otherwise have no axis to slide along or turn about.

    The placement **seeds** it — a Face Snap mate's fixed face becomes the Axis
    (`actions::seeded_joint_frame`, run where the joint is built, so a scripted joint is
    seeded exactly like a drawn one) — and every field stays editable either way. An Axis input
    takes anything with a **direction**: a face or datum plane contributes its normal, an edge
    or world axis its own direction, a hole its centre line; a bare point has none and is
    refused. The Origin defaults to where the mate landed the moving face rather than to the
    axis's own reference point, because choosing an axis says which way the part moves, not
    where the mate put it.

    A joint with **no** Axis has no frame at all: it holds its parts where the placement put
    them, which is what a Rigid joint does anyway and is the honest answer for the others
    until an axis is given. Nothing re-derives one behind the user's back.

    Scripted as `frame_origin =` (a point), `frame_axis =` and `frame_axis2 =` (mate picks) on
    `bearcad.joint` / `edit_joint` / `begin_joint`; left out, the mate seeds them.
  - **Holes and shafts (#1013):** a round wall is one element with a **centre line** of its
    own (`SceneElement::BodyCylinder` / `BodyAxis`, fitted by `extrude::fit_cylinder` from the
    mesh, so an imported part gets them as readily as a modelled hole). Lining a hole up on a
    shaft is then a matter of naming that centre line — as the joint's **Axis**, or (#1080)
    as the point a Face Snap mate lands on.
  - **Grounding against the world (#1018):** the fixed side takes a datum plane, a world axis or
    the origin as readily as another part's geometry, which is how the first part of an assembly
    is placed. World-fixed picks don't ride the base's pose; body picks do, so a chain lines up
    against the fixed part where it actually sits.
  - **Durability (#1019):** picks are body-local keys (`model::MateRef`) re-found on the live
    mesh, stored un-posed so a part picked where it is drawn still reads body-locally. A mate
    whose picks no longer resolve places nothing, so the parts stay where they are — the same
    identity mate an empty one gives.
  **Telling the mate apart in 3D:** the mate's picks are marked where they sit, moving picks
  green and fixed ones red, and the driven part ghosts at the pose the mate implies
  (`joints::preview_pose`, #1017), live as each pick lands.
    **Grounded tree (#893):** one side is the **base** (default the first picked; the pane
  swaps it), the rest are driven. Joints resolve in dependency order so chains compose;
  a joint that would close a **loop**, or drive a part another joint already drives, is
  refused at commit with the reason, and one that decays into that state reads through
  document health (`document_health::mark_broken_joints`). Whatever no joint drives is
  grounded.
  **Limits (#896, `model::JointLimits`):** slide min/max as mm expressions or **up to a
  face/plane** (`ExtrudeTarget` stops resolved where the joint's axis meets the target's
  extended plane, via `extrude::target_distance`); turn min/max as signed degree
  expressions; either end open. The motion clamp (`joints::resolve_limits`) covers every
  kind, including the screw's coupled travel through its lead.
  **Preview (#895):** while a joint is created or edited its ghost sweeps slowly back and
  forth through its range (`joints::sweep_positions`) — between its limits, ±20 mm/±30°
  where open, the full turn for a free revolute — on an eased, looping glide; rigid shows
  the static mated pose. Committing leaves the joint at its position. The pane's **Animate**
  checkbox (#906) turns the sweep off — one app-wide switch (`AppState::animate_joints`, on by
  default, `bearcad.ui.animate_joints(bool)`), so turning it off on any joint's pane turns it
  off for every joint; the preview then holds the joint's own position.
  **Drag (#897/#903):** with the Select tool, press-and-drag a driven part anywhere on it —
  face, edge, or corner — and it moves through its joint, with **nothing selected first**.
  The press arms a grab (`JointSelectGrab`) and still selects normally; only once the cursor
  leaves `JOINT_DRAG_THRESHOLD_PX` (4 px) of the press point does the part start moving, so a
  plain click neither nudges the joint nor announces anything. The cursor then projects onto
  the joint's freedom (`joints::body_drag_joint` walks rigid ties up to the nearest freedom),
  stops at the limits, writes the number back to the position expressions, and lands as
  one undoable edit on release. A part held by its joints refuses with the reason.
  **Auto-zoom stands down for the drag (#905):** the part is meant to travel, so framing
  never chases it — the camera holds still until the drag lands.
  **Rest pose (#898):** `Joint::rest*` — captured at creation, recapturable, and reverted
  singly or all at once from the pane's Rest row, the row's right-click menu, or
  scripting.
  **Presentation (#899/#921):** a hand-drawn icon per kind (`icons::icon_for_joint_kind`), on
  the pane row, beside the **Type** dropdown and on every one of its entries — laid out as one
  widget (`icons::selectable_icon_label`) rather than painted over a space-indented label, which
  put the glyph on the first letter of every entry and would have moved the collision around with
  any font or scale change (#999) — and drawn
  selectable in the 3D view at the joint's posed frame
  (`joint_viewport`); clicking the badge selects the joint, hovering it glows the joined
  parts.
  **Shortcut (#921):** **J** picks the tool; pressing it again **cycles the kind**
  (`JointKind::next`, the dropdown's order), clearing the positions as a kind change does.
  Scripting (#901/#1020): `bearcad.joint{ a =, b = | parts = {…}, kind =, lead?, base = "a"|"b",
  face? = { moving =, fixed =, flip?, offset?, spin? },
  frame_origin?, frame_axis?, frame_axis2?,
  position?, position2?, position3?, slide_min?,
  slide_max?, slide_min_to?, slide_max_to?, turn_min?, turn_max?, name? }`, where a mate pick is
  `{ body =, face = {x,y,z}, normal = {x,y,z} }`, `{ plane = i }`,
  `{ body =, edge = { {x,y,z}, {x,y,z} } }`, `{ axis = "x"|"y"|"z" }`, or a point
  (`vertex`/`on_edge`/`on_face`/`midpoint`/`origin`);
  `bearcad.body_faces(i)` and `bearcad.body_edges(i)` report a body's faces and edges in exactly
  that spelling, so a script names one without guessing its key;
  `bearcad.edit_joint{ index, … }`, `bearcad.ui.begin_joint{ … }` (arms the tool without
  committing, like `begin_move`), `bearcad.set_joint_rest(i)` / `bearcad.revert_joint(i)`
  / `bearcad.revert_joints()`, and `bearcad.count("joint")`; session-command export
  replays them all.

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
  `Extrusion`s with a `Polygon` profile (a rectangle being a four-line polygon), Shape-tool cuboids (same 12-edge box), and cylinder rims, and to the two
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
    untreatable. The kernel extrusion mesh is **validated watertight** before use (#582): OCCT's
    `ThruSections` loft can silently return an open shell (the side wall with no end caps — a pipe)
    for a circle taken up to a diagonal plane, so a non-watertight kernel mesh is discarded in favour
    of the hand-rolled mesher, which caps both ends (`mesh_is_watertight` in `extrusion_mesh`).
  - **Explicitly out of scope**: `Circle`-profile *vertical* edges (a smooth wall, nothing
    to bevel); STL/STEP-imported bodies (pure triangle soup, no analytic
    profile to derive an edge from — #31's generic mesh-feature-edge extraction still works for
    *picking/hovering* those edges for plane-referencing, just not for beveling them); and a
    **vertex miter** where 3+ treated edges would meet at a shared corner — rejected at commit
    time (`crate::extrude::edge_treatment_conflicts`) rather than attempting to blend three
    bevels together, a documented limitation rather than a crash or wrong-looking result.
  - **Data model (operation, #531)**: a 3D chamfer/fillet is a **first-class operation** —
    `Document::edge_treatment_ops: Vec<EdgeTreatmentOperation>`, where `EdgeTreatmentOperation
    { targets: Vec<usize>, edges: Vec<TreatedEdge>, kind: VertexTreatmentKind, amount: f32,
    outputs, .. }`. Its **inputs** are the bodies whose edges it bevels (`targets`) plus the
    edges (`TreatedEdge { target, extrusion, edge }`, the edge addressed by the stable analytic
    `ExtrusionEdgeRef` so it re-resolves parametrically, not by coordinate snapshot). On commit
    each input body becomes a **shadow** body and one new **output** body per input carries the
    bevel (`BodySource::EdgeTreated { op, target }`); the operation is its own graph/timeline
    node with its outputs nested under it and its inputs feeding it (shown, editable, rollback-
    able, undoable like every other body op). Geometrically the output *is* the input body built
    with these treatments spliced onto its extrusions — reusing the extrusion chamfer/fillet
    machinery (mesh **and** kernel), so the default kernel-off build still bevels
    (`crate::extrude::occt_edge_treated_output_shape`). `kind` reuses `VertexTreatmentKind`
    (Chamfer/Fillet) from the 2D case. Legacy files with baked `Extrusion::edge_treatments`
    still render (that path is intact) and editing one migrates it into an operation.
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
    click again to remove"). The picker is the first instance of the generalized per-tool
    selection input (future tools may host several, e.g. boolean A/B sets).
  - **Coplanar profiles: smaller wins (#822)**: when the cursor is inside two sketch
    profiles on the same plane — a hole inside a plate outline — the **smaller** one takes
    the pick (`pick_sketch_face` compares profile areas on a screen-distance tie). Depth
    can't separate coplanar faces, and the bigger one's centroid is often nearer the eye, so
    a hole used to be unpickable with the face-picking tools.
  - **Whole-edge hover (#807)**: hovering with Chamfer/Fillet lights up the **entire analytic
    edge** the cursor is nearest — every chord `treatable_edges` emits for that
    `ExtrusionEdgeRef` (`ViewportHoverHighlight::Curve`), so a hole's rim reads as the one
    circle it is to the tools instead of the single tessellation facet under the pointer.
    (The rim is still *drawn* as chords — analytic curve rendering is a separate matter.)
  - **Multi-edge sets (#157/#166/#1504)**: the in-progress treatment holds a *set* of edges sharing
    one amount/gizmo. Click toggles a treatable edge (or a face's edges) into the set — the
    same rule as Offset, and the same rule the 2D vertex path uses. Switching to Chamfer/Fillet
    with body edges already selected (Select mode, #156) **preloads** the selection — filtered
    to treatable edges — and shows the gizmo immediately. One commit builds **one operation** carrying every
    edge in the set; edges that individually fail (e.g. a vertex-miter conflict) are skipped
    with a status note while the rest apply. Because the whole operation is a single undo group,
    **Undo removes it entirely** (releasing its shadow inputs and beveled outputs) without
    touching the extrusion. The ghost preview shows the gizmo-anchoring extrusion's edges
    (a set spanning several extrusions still commits everywhere, but only the primary
    extrusion gets a ghost — the preview mechanism shows one extrusion at a time).
  - Scriptable via `bearcad.chamfer_edge{ extrusion =, edge = {...}, distance = }` and
    `bearcad.fillet_edge{ extrusion =, edge = {...}, radius = }` (`primitive =` for a
    Shape-tool cuboid/cylinder), where `edge` is `{ kind =
    "vertical", face =, edge = }` or `{ kind = "cap", face =, edge =, top = }`. A whole set goes
    in one call as `edges = { {...}, {...} }` (#672) — each entry either a bare edge spec covered
    by the top-level `extrusion`, or `{ extrusion =, edge = {...} }` to name its own. This is not
    a convenience: **one call is one operation**, and an operation bevels its target extrusion's
    *own* body, so N single-edge calls would each round the same sharp input and leave N
    overlapping outputs that render as an untreated block. `Instruction::EdgeTreatment` therefore
    carries the whole `edges` list, and an interactive multi-edge commit records as **one** script
    call (rendered with the singular `edge =` when the set holds exactly one).
  - **Elements-pane node + edit-after-the-fact (#192/#259/#531):** each committed chamfer/fillet
    is its own selectable operation row (`HierarchyNode::EdgeTreatmentOp`, chamfer/fillet icon,
    "Chamfer/Fillet N" label) with its beveled output body nested under it and its shadowed input
    dimmed. Double-clicking the row (or right-click → "Edit chamfer/fillet") reopens it via
    `EditEdgeTreatmentOp` — reloading its edges/amount into the push/pull gizmo and tombstoning
    the old operation — so committing a new amount rebuilds it, changing a radius after the fact
    without re-picking edges. It can be renamed, hidden, and deleted like any operation.
- **Shell** — hollow a solid to a wall thickness, removing selected faces.

### 3.4.1 Tracing images (#163)
- **Import (#169):** File → Import Image…, or right-click a construction plane in the
  Elements pane → "Import image on this plane…" to target that plane directly (#175),
  or the same command in the command palette when a construction plane is selected (#1564)
  (or `bearcad.import_image("p.png")` /
  `bearcad.import_image{ path =, plane = }` /
  `bearcad.ui.palette("import image on this plane", "p.png")` with that plane selected)
  embeds a PNG/JPEG in the document (base64 in
  the saved JSON, so files stay self-contained like imported meshes) and places it on a
  construction plane (default: plane 0), centered on the plane origin at an initial scale
  of **1 px = 1 mm**. The new image is selected as the only thing so you can
  immediately calibrate its scale (#1582). The image is an Elements-pane row nested under its host plane —
  renamable, hideable, deletable, undoable. Clicking the image with a face-click tool
  (Sketch, Line, Rectangle, Circle, Text, …) begins a sketch on that host plane (#1588),
  just like clicking the plane or a body face. `Document::tracing_images` is an
  `arena::Arena` (#1055): an image is named by a `TracingImageKey` — by the move ops that
  target it and the calibration constraints on it — and deleting one **removes** it, so a
  key to it stops resolving instead of naming whichever image slid into its place. A
  script still names an image by its **ordinal** among the live ones, resolved to a key at
  the script boundary (`lua_script::scene_element_from_kind`/`element_index`).
- **Rendering (#170/#1548/#1562):** each image draws as a **textured quad** on its host plane.
  Fresh imports default to **0.9** opacity; selecting the image shows a slider and
  ValueInput (0–1). Depth-tested (bodies in front occlude it) but never writing depth, and
  drawn **before** construction planes, so sketch geometry, fills, and planes in front of
  the image always read on top. Decoded pixels and GPU textures are cached by content, so
  the per-frame cost is one quad. Scriptable: `bearcad.image_opacity{ image, opacity }`
  (`opacity` is a number or expression) and `bearcad.get{ kind = "image", index }.opacity`.
- **Scale calibration (#163/#171/#1547/#1586/#1612/#1613):** selecting only a tracing image with the Select
  tool immediately enters calibration mode. A line with a point at each end always sits
  on the image's plane — on a fresh import, top-middle to bottom-middle — drawn like a
  selected line (bright, bold, on top of the picture) while the image is selected, and
  always carries a dimension that cannot be removed. That overlay is shown only while the
  image is selected; with the image unselected (and no sketch on its plane) the endpoints neither
  hover nor take a click — the image quad does. Drag either endpoint on or off the image
  (still in-plane); that updates stored UV and never rescales. Type the real length in the
  Context pane and press Enter (or the checkmark), or double-click the dimension
  (a ValueInput: any length expression), to rescale the image uniformly about the span
  midpoint so the two points stay at the same locations on the image and the world-space
  span matches the typed length. Recalibrating then re-solves so existing image constraints
  stay honored (a corner pinned to the origin stays there). The calibration (reference
  segment in image-UV + assigned length/expression) is stored on the image. Scriptable: `bearcad.calibrate_image{ image,
  from, to, length }` (`from`/`to` optional; `length` is an expression),
  `bearcad.ui.focus_calibrate()`,
  `bearcad.calibration_point{ image, index, x, y }`, `bearcad.get{ kind = "image", index }`.
- **Image constraints & viewport pick (#425/#1561/#1589):** a calibrated image's two
  reference points (`ConstraintPoint::ImageCalibrationPoint`), its nine displayed-quad
  box points (`ConstraintPoint::ImageAnchor` — four corners, four edge midpoints, centre),
  and its four edges (`ConstraintLine::ImageEdge`) are first-class constraint
  geometry: pickable/snappable in sketches hosted on the image's plane and usable in
  coincident/midpoint/distance/parallel constraints against vertices, lines, and the
  origin/axes. Solving **translates** the whole image from a box or calibration point
  (`set_point_uv` shifts `origin` and `base_origin`; scale never changes); the edges are
  **fixed** by the image's current pose, so sketch geometry constrains *onto* them. The
  solver holds the non-image side of a point-point coincidence so the image follows its
  target. The Select and Move tools pick an image by clicking its quad in the viewport,
  not only from the Elements pane.
  Scriptable: `bearcad.select{ kind = "image", index, point = 0|1 }` (calibration),
  `anchor = "center"|"top_left"|…` (box point), `edge = "left"|"right"|"top"|"bottom"`.
  *Known limitation:* calibration mutates the image in place and is not yet individually
  undoable (3D edge treatments had the same gap and now undo via a transient snapshot
  marker, #168 — calibration can adopt the same mechanism).

### 3.4.3 Sketch text (#282)
- **Text identity (#1055):** `Document::sketch_texts` is an `arena::Arena`, and a text is
  named by a `SketchTextKey` — `SceneElement::SketchText`, `HierarchyNode::SketchText`,
  `ConstraintPoint::TextAnchor`, and `ExtrudeFace::TextGlyph`. Deleting a text removes it, so
  an anchor constraint or an extruded glyph face on it reads as gone rather than sliding onto
  whichever text used to sit after it. A script names a text by its **ordinal** among the live
  ones, resolved to a key at the script boundary.
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
  (+ expression), baseline origin, rotation, flip, optional wrap width, the baked `contours`, and the
  embedded `font_bytes`. The baked contours (outer loops + counters/holes, separated by winding)
  render as closed polylines on the sketch plane, transformed by the element's origin/rotation/flip. A
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
  in degrees, a **Flip** checkbox that mirrors the glyphs about the box's vertical
  centre (#1571), and a **Wrap width** field (mm; empty
  grows the box to fit, a value word-wraps to that width, #282). Every change re-bakes the
  glyphs immediately. A size expression is stored as typed; the evaluated size only moves once
  the expression is valid, so mid-edit states don't clobber the text.
- **Rotation gizmo (#286/#1570):** with the **Text** tool (create/edit) or the **Move**
  tool and one text selected, the handle-disc rotation gizmo appears in the sketch plane
  around the text's baseline origin, sized to the glyph outlines; dragging the handle
  turns the text about its origin, live. The handle and the context **Rotation°** field
  read the same model value, so they stay in sync. Scriptable as the `"text_rotation"`
  gizmo (degrees, like every angle in the API — #1657).
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
  place. Scriptable: `bearcad.constrain("coincident", { kind = "sketch_text", index = i, anchor = "center" }, …)`. Legacy documents with a
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

- **The clock comes from `crate::time` (#1048).** `std::time::Instant::now()` and
  `SystemTime::now()` compile for wasm32 and **panic at runtime** — "time not implemented on
  this platform" — which took the web app to a black screen before its first frame, from one
  startup timestamp. `crate::time` re-exports `web_time` on wasm and `std::time` everywhere
  else, and a test (`no_raw_std_time_outside_this_module`) fails the build if either raw path
  reappears anywhere else in the crate. That guard matters because **the wasm CI job only
  builds**: a build cannot see a runtime panic, so "the wasm job is green" means it compiles,
  not that it runs. `Duration` and a `UNIX_EPOCH` compared against a filesystem timestamp are
  deliberately left on `std::time` — neither can panic, and the latter would be a type error.
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
  `web/index.html`; `web-time` stands in for `std::time::Instant` (which panics outright on
  wasm — see #1048); wgpu's `webgl` feature provides the fallback for browsers without
  WebGPU.
- **Every asset URL carries the build it belongs to (#1049).** wasm-bindgen mangles each
  imported JS function's name with a content hash, so a cached `bearcad.js` served alongside
  a freshly fetched `bearcad_bg.wasm` does not merely misbehave — instantiation fails with
  `LinkError: import object field '__wbg_…' is not a Function`, and the app never starts.
  Unversioned URLs let a browser expire the two independently, which is exactly how that
  happens after a deploy. `scripts/build-web.sh` substitutes a `__BEARCAD_BUILD__`
  placeholder in `index.html` with the wasm's own content hash — so it changes precisely
  when the artifacts do — and every import, the `init({module_or_path})` wasm URL, and the
  Emscripten modules' `locateFile` all carry it. The two Emscripten modules need it passed
  in rather than only on their import, since they fetch their own `.wasm` themselves.
  A `LinkError` also reports itself as a stale cache rather than as a missing GPU: the
  startup error box previously told everyone to check WebGPU support whatever went wrong.

### 3.5 Advanced features
- **Create Shape tool** *(#909)* — `Tool::Shape`, 3D only (picking it from a sketch leaves
  the sketch). Its toolbar button shows the **last used** shape's icon
  (`AppState::shape_kind`), and its plain-letter shortcut **B** picks the tool, then cycles
  cuboid → cylinder → sphere on each further press (like Rectangle's anchor toggle). The
  context pane shows a **Shape** row of three icon buttons and that shape's own labelled
  ValueInputs (cuboid: Width/Depth/Height; cylinder: Radius/Height; sphere: Radius) over a
  Create button; **Enter** commits when every dimension has a size, **Esc** clears the
  in-progress shape, and leaving the tool drops it. Switching shape mid-placement keeps the
  frame and the dimensions already typed. Double-clicking a shape's Elements row reopens the
  tool with it loaded (`CreatingShape::editing`), where the button reads **Apply changes**.
  **Placement (#912):** before the first click a generic ghost of the shape follows the
  cursor, sized to the view (`cam.distance / 8`) rather than the model, drawn through the
  same translucent `preview_solid` the revolve/loft ghosts use. A cuboid's ghost hangs its
  **corner** on the cursor (#929, `primitives::ghost_origin`) — its first click places a
  corner, so straddling the cursor would misread; a cylinder and a sphere are placed by their
  centre and sit on it. Click 1 **anchors** it on whatever the
  cursor is really pointing at — the candidate **nearest the eye** (#932, `nearest_anchor`):
  the analytic face or construction plane under it (which brings its own frame), or any
  body's flat **mesh** face (how a shape lands on another shape — primitives have no analytic
  faces of their own), else the ground; the shape then grows along that plane's normal. A
  mesh face brings no frame of its own, so its in-plane axis comes from
  `primitives::plane_u_axis` — the world axis least aligned with the normal, projected into
  the plane — rather than `Vec3::any_orthonormal_vector`, which is free to answer differently
  for the same plane. Without that, a body's horizontal top face and the ground beneath it
  disagreed, and a cuboid dropped on the face landed rotated 90° against one dropped on the
  ground beside it (#1050). A body's analytic frame follows the polygon's first edge, which
  on some cuboid walls runs against that world axis; the hover ghost uses the world axis
  (unless the pick is a construction plane) so the two picks of one wall cannot hang the
  cuboid from opposite corners as the pointer moves. What
  the next clicks set is per kind — cuboid: the opposite base corner, then the height;
  cylinder: the radius, then the height; sphere: the radius (and it's done). Each phase
  focuses its own ValueInput, so the size can be typed the moment the click lands, and a typed
  dimension stops following the cursor (`CreatingShape::typed`) until it's cleared. Every
  dimension is **mirrored in the 3D view** beside the edge it drives (#930,
  `primitives::field_anchors` → `draw_shape_dimension_mirrors`), in the value field's own
  boxed style with the phase's one framed in amber. The mirrors are *drawn*, not widgets: a
  field under the cursor would swallow the next placement click (the viewport stops being
  hovered), so the pane's rows stay the ones taking the keyboard, and each mirror is pushed
  away from the cursor as it moves. **Enter**
  in a shape field creates it, like the sketch Rectangle's typed dimensions; the height is
  otherwise the signed free-cursor offset along the normal (`offset_along_normal_from_cursor`)
  so the tip stays even with the pointer, including behind the placement face
  (`shape_growth_from_height_offset`, #1763).
  **Snapping (#913/#931):** the tool joins the sticky **Snapping** toggle (`AppState::snapping_enabled`,
  shared with the drawing tools, `bearcad.ui.snapping(bool)`). While it's on, the anchor and
  base clicks land on the nearest body **corner** or edge **midpoint** inside the pick radius
  instead of the raw point on the anchor plane; off, the cursor's own point stands. The point
  it has caught is **ringed in the viewport** exactly as the sketch tools ring theirs — the
  same cyan ring and dot — so it reads before the click (#931).
- **Shapes** *(model, #909)* — cuboids, cylinders and spheres placed straight into 3D, with
  no sketch behind them. A `Primitive { kind, origin, normal, u_axis, width, depth, height,
  radius, name }` in `Document::primitives` — an `arena::Arena` keyed by `PrimitiveKey`
  (#1055) — stores the anchor frame: the point placed on,
  the plane normal it grows along, and that plane's first in-plane direction — plus its
  dimensions as **expressions**, so a shape follows parameters like any other feature. Each
  shape owns one body via `BodySource::Primitive`, and one `ShapeKind::Primitive` undo marker
  covers both; deleting the shape takes the body with it. It sits **on** its plane: a cuboid
  centred on its base rectangle, a cylinder on its base circle, a sphere on the point it
  rests on (centre one radius up the normal). `primitives::mesh` tessellates it analytically
  (64 radial segments, 32 sphere stacks) and `primitives::kernel_shape` builds the solid —
  a prism for the cuboid, `BRepPrimAPI_MakeCylinder` for the cylinder, and
  `BRepPrimAPI_MakeSphere` for the sphere (#936 — revolving a half-disc *looks* like the way
  to build one, but its profile touches the revolution axis at both poles and OCCT refuses
  it, which left every boolean against a sphere landing an empty body). A shape missing a dimension is refused rather than landing
  an empty body. It's a top-level row in the Elements pane, named by kind ("Cuboid 0"), with
  its body nested under it. Scriptable as `bearcad.cuboid{ at?, normal?, u_axis?, width,
  depth, height, name? }`, `bearcad.cylinder{ radius, height, … }`, `bearcad.sphere{ radius,
  … }`, and `bearcad.edit_shape{ index, shape?, … }` (unmentioned fields keep their value);
  `at` defaults to the world origin and `normal` to +Z.
- **Sweep** *(implemented)* — sweep one or more coplanar closed profiles
  along a path of sketch lines into a solid. The **Sweep** toolbar tool collects
  profile faces by clicking (same face picking as Extrude: sketch profiles **and** bare
  flat body faces — primitive caps/sides, extrude caps/sides, revolve flats — via an
  implicit boundary sketch, #1237), then path lines: any lines —
  straight or bezier-curved, plain/construction/projected, in any sketch — that chain
  end-to-end and cross the profile plane (an in-plane line is refused with a status hint).
  Pick order doesn't matter: segments are chained tip-to-tail at evaluation and the chain
  is oriented to start at the end nearer the profile plane. Clicking a picked face or line
  removes it again. The context pane shows the picked faces and path lines as real element
  pickers (#955; each row has a ✕; faces/lines are still added by clicking in the viewport) —
  Profile over analytic faces, Path over sketch lines, with exactly one focused.
  A translucent ghost of the swept solid previews after every pick. The result lands as a
  **new body**, **fused into touching bodies** (resolved at commit by mesh-bounds
  intersection), or **cut from picked bodies** — the same segmented icon group as Revolve;
  in Cut mode the preview replaces each targeted body with the finished cut result
  (mirroring the extrude cut preview). Selecting a committed sweep offers **Edit
  sweep**, re-opening the tool with its faces/path/mode loaded and re-pointing the
  operation in place on commit. Data model: `Sweep { sketch, faces, path, mode }` in
  `Document::sweeps`, an `arena::Arena` (#1055) keyed by `SweepKey`, with
  `SweepMode::{NewBody, AddTo(bodies), Cut(bodies)}`;
  add/cut relationships live on the sweep (bodies consult `sweeps_targeting` at
  mesh/kernel build time), and a NewBody sweep gets `BodySource::Sweep`. One
  `ShapeKind::Sweep` undo marker covers the feature and its body. In the elements
  graph the profile sketch and every path line feed the Sweep operation node, and
  the output body nests beneath it. Kernel builds use `BRepOffsetAPI_MakePipeShell`
  (right-corner transitions for straight chains; a chain containing any curved segment is
  interpolated into a B-spline spine) with the profile corrected normal to the spine; the
  no-kernel fallback carries profile rings along parallel-transport frames, stitches the
  walls, and caps both ends, oriented against the transported profile centroid. Scriptable
  as `bearcad.sweep{ polygon|circles =, path = {line indices},
  body = "new"|"add"|"cut", bodies = {..} }`, and interactive sweeps replay to the
  command log as the same call. Limitation: the path must be one connected chain, and a
  tight bend radius smaller than the profile's half-width can self-intersect.
- **Revolve** *(implemented)* — spin one or more coplanar closed profiles around an axis
  into a solid. The **Revolve** toolbar tool collects profile faces by clicking (same face
  picking as Extrude), then an axis: any line in the sketch (plain, construction, or
  projected) or a global X/Y/Z axis. The sweep angle defaults to **360°** and is set by
  dragging a push/pull disc handle **around an arc** — the arc sweeps from the profile to the
  current angle and the handle rides its far end (#262) — or by typing (bare numbers are
  degrees; `rad`/`deg` suffixes and parameter expressions work); **Symmetric** sweeps half the
  angle to each side of the
  profile plane. The context pane shows the picked profile faces and the axis as their own real
  element pickers (#261/#955; each row has a ✕ to remove it; faces/axis are still added by
  clicking in the viewport). Profile takes analytic faces (`SceneElement::SketchFace`); Axis is
  **single-pick** and takes a straight reference only — a sketch line with no curve to it, a
  body's feature edge, or a world axis (`PickRule::Straight` over
  `SceneElement::from_revolve_axis`), so a circle is never offered as an axis. The result lands as a **new body**, **fused into touching bodies**
  (resolved at commit by mesh-bounds intersection), or **cut from picked bodies** — chosen
  with a segmented icon button group (New body / Add to touching / Cut, the same icons the
  Extrude "into" picker uses) (#261); cut targets are clicked in the viewport and listed in
  the context pane's generic selection picker. Data model: `Revolution { sketch, faces, axis, angle_deg, pitch_mm, symmetric, mode }` in
  `Document::revolutions`, an `arena::Arena` (#1055), with
  `RevolveMode::{NewBody, AddTo(bodies), Cut(bodies)}`; a revolve is named by a
  `RevolutionKey` — by `BodySource::Revolve`, by `FaceId::RevolveCap`/`RevolveSide` for a
  sketch hosted on one of its flat faces, and by its component membership — and deleting one
  removes it. Add/cut relationships live on the revolution (bodies consult
  `revolutions_targeting` at mesh/kernel build time), and a NewBody revolve gets
  `BodySource::Revolve`. One
  `ShapeKind::Revolution` undo marker covers the feature and its body. Kernel builds use
  `BRepPrimAPI_MakeRevol` (full revolutions via the no-angle constructor — the angle
  constructor normalizes mod 2π and would build a sliver from a float 2π) with symmetric
  sweeps pre-rotating the profile; non-zero `pitch_mm` (axial travel per full turn, #1242)
  pipes the profile along a smooth B-spline helix with fixed BiNormal = revolve axis
  (`BRepOffsetAPI_MakePipeShell`, #1249) so springs stay true curved BREP for STEP and
  adaptive tessellation for the viewport (not a density-capped lathe). The no-kernel
  fallback lathes rotated (and axially advanced) profile rings with sweep-end caps,
  oriented against the rotated profile centroid (correct for washer profiles that don't
  contain the axis).
  The context pane's **Angle** field toggles to **Revs** (turns, decimals allowed;
  both signed) via an icon+label toggle (#1246: angled lines for degrees, `#` for turns),
  and a **Gap/Offset** field (same icons as Repeat) sets the helical pitch — Offset is
  start-to-start after 360°, Gap is clear space between coils. Scriptable as
  `bearcad.revolve{ polygon|circles =, axis = "x"|"y"|"z"|{line = i}, angle = |
  revolutions =, pitch =, symmetric =, body = "new"|"add"|"cut", bodies = {..} }`,
  and interactive revolves replay to the command log as the same call. Limitation: the
  profile must not cross its axis.
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
  The result lands as a **new body**, **fused into touching bodies**, or **cut from picked
  bodies** (#479) — the same segmented New/Add/Cut icon group as Revolve/Sweep, with cut
  targets clicked in the viewport, listed in the context pane's picker, and stored as
  `LoftMode::{NewBody, AddTo, Cut}` on the feature (`lofts_targeting` at mesh/kernel build
  time; pre-#479 files load as `NewBody`). The mesh is a ruled loft rebuilt parametrically
  from the live profiles: each section boundary is resampled to a common ring size, rings
  are aligned (consistent winding, twist-minimizing start offset) and stitched with wall
  quads, and the end sections are capped; the kernel path builds the same ruled blend as
  pairwise `ThruSections` segments fused into one BREP solid, which is what Add/Cut boolean
  against. Scriptable as
  `bearcad.loft{ profiles = {…}, body = "new"|"add"|"cut",
  bodies = {..}, name = }` (`profiles` is two or more circles or line loops;
  each face's sketch is inferred as in `bearcad.extrude`),
  and interactive lofts replay to the command log as the same call. In the Elements pane a
  loft shows as its **own operation node** (`HierarchyNode::Loft`) with its output body nested
  beneath it and its cross-section **sketches** feeding it as Graph-view dependency edges
  (#252) — previously the loft body surfaced as a bare top-level element with no sign of what
  produced it.
- **Pattern** — linear and circular patterns of features/bodies.

Each operation is exposed identically through the GUI, the action DAG, and the scripting
API (§8). Failures from the kernel (e.g. a fillet that can't be applied) must surface as a
recoverable error on the relevant feature node, not a crash.

### 3.7 Cross-section views (#1671)

A **cross-section view** is a saved way of *looking* at the model: a set of cutting planes
that hide whatever sits in front of them, with the exposed faces hatched. It carves nothing —
no geometry is produced and nothing in the model depends on it — so, like a drawing, it lives
outside the shape/undo DAG.

- **Model:** `Document::cross_sections` is an `arena::Arena<CrossSection>`; a view is named by
  a `CrossSectionKey` (`SceneElement::CrossSection`, `HierarchyNode::CrossSection`). Each
  view holds a name and its `cuts`: a `CrossSectionCut` per plane — the frame it was placed on
  (`origin`/`normal`), how far it has slid along its own normal (`offset_mm`), how deep the
  cut reaches (`depth_mm`, #1845), its two in-plane tilts (`roll` / `tilt_v`), and which side
  survives (`flip`).
- **Where they live:** views group under a **Views** section at the bottom of the Elements
  pane, the way drawings group under Drawings (#1205) — with their own filter toggle. The
  section collapses; the views inside are ordinary elements (selectable, renameable,
  deletable). Each cutting plane is its own element nested under its view
  (`HierarchyNode::SectionPlane` / `SceneElement::SectionPlane`): selectable, renameable,
  deletable, and hideable (a hidden plane does not cut). The modeling workbench shows the
  views but not their cutting planes. Scriptable as
  `bearcad.count("section_plane")` / `bearcad.get{ kind = "section_plane", index }` /
  `bearcad.select{ kind = "section_plane", index }`.
- **Creating one:** the Elements pane's **+ → New cross section**, or **View → Create Cross
  Section**. Scriptable as `bearcad.cross_section{ name? }` (#1671), and readable back with
  `bearcad.count("cross_section")` / `bearcad.get{ kind = "cross_section", index }`.
- **On a drawing page (#1689):** a technical drawing imports a **whole view** — every live
  body, cut by that view's planes (`Action::AddDrawingCrossSectionView`, or the Add-view tool
  clicking a view's row / its **Add to drawing** menu entry) — or **bodies from a view**: an
  ordinary projection with `DrawingView::cross_section` set, so those bodies show cut. The page
  geometry follows: `drawing_view_solid_mesh` and the crease edges come from the cut mesh, and
  the hatch joins the stroked edges, so hidden-line removal treats it like any other line while
  dimensioning and circle detection still see only real geometry. Scriptable:
  `bearcad.drawing_view{ drawing, cross_section = i }` for the whole view, the same call with a
  body source to section those bodies, and `bearcad.drawing_view_section{ drawing, view,
  cross_section = i | false }` to section a projection already placed.
- **Drawing a section (#1688):** while a view is open, every visible body draws **cut**: its
  kernel solid minus a half-space per plane (`extrude::cross_section_body_mesh`, memoized on
  the document's mesh fingerprint plus the cuts, since a boolean per body per frame is not
  free). The kernel closes each opening, so the exposed face is real geometry rather than an
  open shell — and it is **hatched**: parallel lines at 3 mm, phased from the plane's own
  origin so they run unbroken across the whole face rather than restarting per triangle.
  The kept side is the one the plane's normal points toward; **Flip** keeps the other.
  **Cut depth (#1845)** bounds how far the plane reaches: blank cuts all the way through,
  while a length pairs the plane with a second one that far behind it facing back, so only
  the slab between the two is hidden — a chunk out of the middle of the model rather than a
  whole half. Both planes open a hatched, outlined face, and both draw their quad while the
  plane is selected.
  A face coplanar with a cutting plane is drawn only if some of the body sits on the keep
  side. Visible views cut the model in every workbench — hide a view to lift its cut.
  The hatch is thick dark grey. The yellow plane quad draws only while the plane is selected
  (or while its draft is up).
- **The cutting-plane tool (#1687/#1745/#1757):** the View workbench's own tool, on the same
  picker / gizmo path as the construction-plane tool. The **Anchor** picker takes a face,
  construction plane, edge, or axis. A face gets one offset gizmo and two in-plane tilt
  rings; an edge gets the construction-plane offset + angle gizmos. After a pick the
  Anchor loses focus and Offset takes the keyboard. **Enter** or the blue primary button
  hangs it on the open view. Double-click a hanging plane (or right-click **Edit**) to
  reopen the same inputs with a live preview. Cutting planes nest under their view in the
  Elements pane and are hidden from that pane in the modeling workbench.
  Scriptable: `bearcad.section_plane{ view?, plane|origin+normal, offset?, roll?, depth?,
  flip?, bodies?, exclude_bodies? }`, `bearcad.edit_section_plane{ view?, cut, …, depth?,
  bodies?, exclude_bodies? }`, `bearcad.delete_section_plane{ view?, cut }`,
  and `bearcad.section_planes(view?)` reads them back (`roll` in degrees; `depth` is a
  length or `false` for a cut that runs through; `bodies` is
  `"all"` or body indices, `excludes` the spared ones). `bearcad.section_stats(body)` gives
  a body's `{ volume, triangles, bbox }` **as the open view shows it** (#1845), so a script
  can see what the planes actually took. A scope is a `bodies` list and an
  `exclude_bodies` list (#1769): a pane picker each, each with an All-bodies switch — a
  body is cut when it is listed (or all) **and** not excluded.
- **The View workbench (#1686):** creating a view — or double-clicking one's row, or its
  right-click **Edit** — opens it in the **View** workbench (`AppState::editing_cross_section`),
  whose toolbar carries the view's own tools plus a **Back to the 3D model** button.
- **Workbenches (#1686):** `Workbench` (Model / Sketch / Drawing / View) is **derived** from
  `AppState`, never stored — an open sketch *is* the Sketch workbench — and exactly one is
  open at a time: opening a view leaves a drawing page and vice versa. The toolbar's leftmost
  control is a **picker naming the current workbench**; choosing another leaves what is open
  and enters the target by opening its most recent sketch / drawing / view, and a workbench
  with nothing to work on yet is listed but not selectable. `tooltable::workbench_tools` is
  the one place a bar's tool set is decided, and switching drops any tool the new bar lacks.
  Scriptable: `bearcad.ui.workbench()` reads it, `bearcad.ui.workbench(name)` switches.

### 3.6 Technical drawings (#180)

A **technical drawing** is a black-on-white sheet for print/PDF output. A document holds any
number of them; each references bodies but produces no solid geometry, so drawings live
outside the shape/undo DAG (undo is snapshot-based, §4.3).

- **Drawing identity (#1055):** `Document::drawings` is an `arena::Arena`, and a drawing is
  named by a `DrawingKey` — `SceneElement::Drawing`/`DrawingElement`, the hierarchy's
  drawing/projection/dimension/annotation nodes, `ComponentMember::Drawing`, the open
  drawing, and the drawing selection. A script names a drawing by its **ordinal** among the
  live ones, resolved to a key at the script boundary.
- **Annotation identity (#1055):** a page's `annotations` is an `arena::Arena` too, and a text
  note is named by an `AnnotationKey` — `DrawingElementRef::Text`,
  `HierarchyNode::DrawingAnnotation`, and the add/edit/move/remove actions. Deleting one
  removes it, so the note beside it does not slide into its place under an open editor. A
  script names an annotation by its **ordinal** among the live ones on that page.
- **Create & manage:** the Elements pane has a **＋ New Drawing** button (and a `Drawing`
  node, with its own icon, per drawing). Right-clicking a drawing — or clicking its row —
  **opens it** in the drawing pane, which takes over the central area. The **editor** is
  white-on-black to match the app's dark-mode aesthetic (#254); **export** inverts back to
  black ink on a white sheet.
- **Workbenches (#254/#271/#272):** opening a drawing switches to the **Drawing workbench**,
  whose toolbar shows **Back, Select, Projection, Aligned view, Dimension, Zoom loupe, Text** (#295: no Move
  tool; the Select tool drags projections directly, #293 — and **only** the Select tool: with
  any other tool, e.g. Dimension, dragging across a card moves nothing, #374). Entering the
  workbench with any
  other tool active drops back to Select. An **icon-only Back button** (left of Select, #318/#1231,
  same size as the other toolbar icons) returns to the model; **Escape no longer exits** the
  workbench (it cancels in-progress tool actions). Clicking anywhere on a projection card selects
  it (not just the caption, #316). A card's border and Delete ✕ show only while selected or
  hovered (#1229). The model-only **Selection** element picker is hidden here (#317), since
  projections and annotations have their own selection state.
- **Aligned-projection tool (#296):** the workbench's **Aligned view** tool (projection icon;
  tool name `drawing_align`/`aligned_view`) derives an orthographic child from an existing
  projection. It picks a **base view** to align to: a single selected projection is used
  automatically on entering the tool, otherwise it's chosen from the tool's **Base view** element
  picker in the context pane or by clicking a projection on the page (#365). Right-clicking a
  projection card offers **Create aligned view** (#1225) — selects that card and switches to
  this tool with it as the base — instead of dumping every orientation into the menu
  (orientation stays on the context-pane navigation bear). Then move the mouse —
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
- **Zoom loupes (#1846):** the workbench's **Zoom loupe** tool draws a **detail callout** on a
  projection: a small circle ringing part of the view and a bigger circle elsewhere on the page
  redrawing what the small one covers, magnified, with a thin line joining their **rims**. Four
  clicks place one — centre then rim for each circle, like the Circle tool — and Esc drops a
  half-made one. Both circles live in the view's own **projected millimetres**
  (`DrawingView::loupes`, `DrawingLoupe { at, radius, to, to_radius }`), the same space
  `point_dims` use, so a loupe travels with its projection at any page scale. The magnification
  is the **ratio of the radii**, so enlarging the big circle zooms further in rather than
  showing more. `drawing::loupe_drawing` builds what to draw — the two circles, the rim-to-rim
  connector, and the view's stroked lines clipped to the detail circle then scaled onto the
  magnified one — so the editor sheet and the SVG/PDF export put down the same thing. Loupe
  chrome strokes at `drawing::LOUPE_STROKE` (half the model outline); the magnified geometry
  strokes like the view it came from. Under **Select** each circle is its own element
  (`DrawingElementRef::Loupe { view, index, magnified }`): click to select, drag the middle to
  move, drag the outer ring to resize, Delete on either drops the pair. Scriptable:
  `bearcad.drawing_loupe{ drawing, view, at, radius, to, to_radius }`,
  `bearcad.edit_drawing_loupe{ …, index, at?, radius?, to?, to_radius? }`,
  `bearcad.delete_drawing_loupe{ drawing, view, index }`, and
  `bearcad.drawing_loupes{ drawing, view }` reads them back (with the derived `zoom`);
  `bearcad.ui.drawing_loupe_rect{ view, index, magnified? }` reports where a circle was drawn.
  A length dimension on a loupe (#1849) is labelled with the edge's real length. If a
  measured end sits outside the detail circle, that end dashes instead of closing with an
  arrow (#1913) — ISO 129 keeps arrows only at shown feature ends, so the crop is not read
  as the whole measurement. `drawing_loupes` reports `open_a`/`open_b` on each dimension.
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
  (#330). The view editor reappears under the Select/Dimension tools. The **Default units** section
  is now shown only for the selection/sketch-editing tools — it is suppressed under the modeling,
  transform, dimension, and constraint tools (Extrude, Sweep, Loft, Revolve, Combine, Move, Mirror,
  Slice, Repeat, Text, Dimension, Constraint), whose own context sections don't need it (#585) —
  and under **Joint** (#998), whose section is busier than any of them and whose units are
  whatever its parts' already are.
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
- **Projection tool (#289):** the workbench's **Projection** tool (the film-projector
  icon — named "Add view" with a ＋ icon until #753; tool name `drawing_add`, unchanged)
  replaces the old inline "Add view:" combo row. With it active, clicking a
  **body or sketch** in the Elements pane drops a projection of it onto the page and selects
  it; the **context pane** then shows the view editor — source label, **orientation**
  dropdown, **Scale** field, and **Remove view** — and the card can be dragged into place.
  Clicking any existing card (any tool) selects it and opens the same editor (selected card
  gets an accent border; `AppState::selected_drawing_view`).
- **Drag from the pane (#290):** with a drawing open, **dragging a body or sketch row** from
  the Elements pane onto the page places a projection at the drop point (the page shows an
  accent border while a compatible drag hovers), selected and ready to configure — the same
  result as the Projection tool. The row's **name and its type icon** are both grab handles
  (#368). Plain clicks on those rows still select as usual.
- **Orientation bear (#315/#1226):** a selected view's orientation is chosen with an **interactive
  navigation bear** in the context pane (the same widget as the viewport's HUD bear, replacing
  the dropdown; `view_cube::show_orientation_picker`): drag it to spin, click a face for that
  straight-on view or a corner/edge for the isometric, and — when the widget has focus — the
  numpad picks views (**4** left, **5** front, **6** right, **8** top, **2** bottom, **0**
  back). Dragging spins the bear **smoothly** on a local camera; the **drawing view snaps** to the
  nearest discrete face/edge/corner once that pose is closer than the current one
  (`view_cube::nearest_cube_pick`) — so Front → Front-Right → Right as you drag, while the blue
  highlight jumps in snaps. Dragging the bear must **not** pan the paper sheet. It maps the
  picked pose to a `DrawingOrientation`. Adding a view now selects it, so the bear appears
  immediately. The **currently-selected view is highlighted in blue** on the bear (#323/#340) —
  a face fill for the six straight-on views, a dot on the top-front-right **corner** for
  Isometric, or the matching **edge** for a diagonal edge view
  (`drawing_orientation_to_cube_pick` → `view_cube::CubePick`). The highlight is drawn
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
  key light, under the visible edges). Coplanar triangles merge into one flat, and the paint
  order is a depth sort **repaired pairwise** (#1820): a flat spans a range of depths, so
  overlapping pairs are re-asked which is really in front where they meet and topologically
  sorted (`drawing::painter_order`) — otherwise a bar's shaded side leaks through onto the top
  of the block it grows out of. Sketch views have no solid, so they always draw wireframe.
  Two more styles draw by hand: **Loose pencil** (#1809) wobbles and overshoots the visible
  edges, and **Colored pencil** (#1821/#1825) is the viewport's colored-pencil mode on the
  page — the same hand-drawn edges, in a deepened version of the body's own color when the
  view shows one, over a ground of that color, with `crate::pencil` scribble strokes laid
  across each flat and the solids' shadows dropped onto one another. One tone on every side.
  **Watercolor** (#1829) is the same again with the color washed on instead: a covering
  ground, pooling, and the rim it dries to along each flat's boundary. `is_hand_colored` is
  what both share; `ShadingStroke::width` lets a brush lay a broader mark than a pencil.
  Strokes travel as `ShadingStroke`s, and `StyledViewGeometry::scribbled` marks the fills as a
  pencil **ground** rather than a shaded surface. The geometry mixes that ground once and the
  tint carries the result; the editor then reads the whole style as **ink on paper**
  (`ink_on_dark_sheet`) rather than as color, because mapping a pale print ground and a dark
  print rim straight onto a dark sheet inverts them and the painting comes out inside-out. **Both pencil styles letter their own text
  (#1830)**: a pencil view's caption and dimensions are set in the bundled hand-lettered
  **Klee One** (`pencil::LABEL_FONT`, OFL — see `THIRD_PARTY_LICENSES.md`), registered with
  egui once in `theme::apply`; every other style keeps the sans. `Canvas::set_hand_lettered`
  carries it to the exports — SVG names the family (falling back to the sans where it isn't
  installed), and PDF stays on Helvetica, since embedding the face would add megabytes to
  every exported file. The projection logic is
  `drawing::styled_view_geometry`, shared by the editor pane (greys darkened for the dark
  sheet) and both exports (the `Canvas` trait gained a filled-polygon primitive).
  `Action::SetDrawingViewStyle`.
- **White paper (#1831):** the editor's sheet is dark to match the app, but a drawing can be
  put on **white paper** — the exports' own pairing of black ink on white — so what the print
  will look like can be checked without exporting it. Right-click the drawing in the Elements
  pane; `Drawing::white_paper`, so it saves with the document and every view on the page follows
  it. On white the shaded fills and the hand-drawn marks use the print values directly, rather
  than the dark sheet's `ink_on_dark_sheet` reading. Exports are unaffected: they were always
  black on white. `Action::SetDrawingPaper`, `bearcad.drawing_paper{ drawing, paper }`.
- **View card size (#1207):** each projection stores `size_x`/`size_y` as page fractions
  (default 0.42). Under **Select**, a selected card shows corner grips; dragging a corner
  resizes about the card centre (`Action::SetDrawingViewSize`, one undo step for the
  whole drag). Above/Below aligned partners share width, Left/Right share height —
  resizing any of them propagates the linked axis through the alignment graph
  (`drawing::apply_view_size`). Aligned children inherit the parent's size on creation.
  Scriptable: `bearcad.drawing_view_size{ drawing, view, width?, height? }`.
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
- **Aligned projection lines (#377/#1206):** an aligned child can draw **two dashed, lightweight
  lines** connecting its silhouette extremes to its base view's across the gap — at the far
  left/right of the pair for an above/below child, the top/bottom for a left/right one —
  each endpoint on the **facing body edge** at that extreme (not a floating AABB corner) —
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
  Right-click a **body** → **Create drawing** (#1158) opens a new drawing with a default
  view of that body (named after the body when it has a name). Scriptable too (#403):
  `bearcad.drawing_view{ drawing, sketch = i, orientation? }` — the call takes exactly one of
  `body` or `sketch`.
- **Projection elements (#254/#281):** each placed view shows in the Elements pane as a
  **projection** node (`HierarchyNode::DrawingProjection`, its own icon) nested **under its
  drawing**. In the Graph view it also draws a **dependency lane** to its source body —
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
  measurement centred beside it (the label's visual centre is offset off the
  dimension line so the glyphs never sit on the stroke — editor and both
  exports) — from one shared `drawing::dimension_line_geometry`. Dimension lines, their extension lines, and diameter lines
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
  in the exports). Editor and exports treat that layout point as the **visual centre** of
  the glyphs (SVG `dominant-baseline="central"`, PDF baseline shifted 0.35em) so a label
  that sits beside its line on screen does in the PDF too. All dimensions are keyed to the
  edges' quantized world endpoints (a geometry identity that survives rebuilds), stored per
  view.
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
  "force rebuild" command (**File → Rebuild Geometry**, `bearcad.rebuild_geometry()`,
  `--rebuild`) discards `geometry_cache` and replays the DAG.
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
- A parameter has: name, expression (text), evaluated value, unit, optional
  description, and optional **minimum** / **maximum** / **step** expressions (#1176).
- Parameter changes are DAG nodes (§4.1).
- **Primary/secondary / Private (#727/#1176/#1180):** `Parameter.primary` marks a file's
  front-door knobs — what someone importing the file is expected to change (secondary =
  internals; advisory only, nothing blocked). UI toggles the **inverse** via the
  **Private** checkbox in the per-row gear-options panel (not an eyeball): checked =
  secondary, unchecked = primary. Scripted as `bearcad.edit_parameter{ name, private = bool }`
  (`true` ⇒ secondary). Defaults: existing documents load **secondary**
  (`serde(default)` on `primary`); a **new** parameter is primary (Private unchecked) iff
  its expression is a plain self-contained value (`new_parameter_primary_default`),
  computed once at creation and never on later edits.
- **Bounds (#1176):** `Parameter.minimum` / `maximum` / `step` are optional expressions
  (`Option<String>`, empty clears). Unit kind (length vs angle) follows the default
  value's units. Scripted as `bearcad.edit_parameter{ name, min|max|step = expression? }`
  (`false` or `""` clears; `Action::SetParameterBound`). The gear cog on each Parameters-pane row toggles that
  row's options panel (multiple open at once). Tab from a focused Min field commits and
  focuses Max so typing sets the max; Tab from Max focuses Step (#1576). Scripted as
  `bearcad.parameter_options(name[, open])`, `bearcad.parameter_edit(name, "min"|"max"|"step")`,
  `bearcad.parameter_editing()`. **Imported unit instances** cannot edit
  these options; overrides (`SetUnitParameterOverride` / `set_unit_parameter`) are
  clamp-and-snapped to min/max/step (`clamp_and_snap_override_expression`). With both min
  and max resolved, a **slider** sits on the row below the parameter, spanning the name
  and value columns, and snaps to step — the same layout for this document's own
  parameters and for imported unit instances (#1559;
  `bearcad.parameter_slider(name[, value])`).
- When a parameter's name or value field is focused in the Parameters pane, the Elements
  pane highlights every element that uses that parameter (the dimensions referencing it and
  the geometry they drive), dimming the rest.
- Hovering a parameter's row (or focusing its fields) styles the pane rows in the **same
  green** as the 3D glow (#633, `USES_VARIABLE_TEXT` = `PARAMETER_HIGHLIGHT`), and the
  highlight set also includes each used element's **owning sketch**, so a collapsed/
  filtered pane still shows which sketches use the parameter.
- **Hovering a parameter's row** (any cell) or focusing its name/value field also
  **green-glows those users in the 3D viewport** (#620,
  `ParametersPaneState::hovered_name` → `elements_using_parameter` →
  `ViewportSceneInput::parameter_highlight_elements`, drawn in `PARAMETER_HIGHLIGHT`):
  dimension badges, the constrained lines/points, and any extrusion whose distance
  expression references the parameter (outlined as a green wireframe of its own mesh).
- Each parameter row has a muted-red **✕** delete button (`Action::DeleteParameter`, #270).

#### 5.1.1 Inline parameter creation
- In **any value input** (GUI field or scripting), prefixing the entry with
  `name=` creates a new parameter on the spot and uses it for that input. For example,
  typing `width=20mm` in an extrude-distance field creates a parameter `width = 20mm` and
  binds the field to it (the field now holds the expression `width`). This mirrors
  Autodesk Fusion's inline-parameter behavior.
- The assignment target follows the normal scoping rules (§5.1); creation is a DAG node
  like any other parameter creation. Scripted dimensions take it too (#797):
  `bearcad.dimension{ kind = "line", index = i, value = "leg = 40mm" }` and
  `bearcad.dimension{ kind = "angle", …, value = "corner = 90deg" }` define the parameter and dimension with it.
- If `name` already exists, the input must either **reuse** it (binding the field to the
  existing parameter) or, if a value is also supplied, treat `name=value` as redefining
  that parameter — the UI must make which one is happening unambiguous (e.g. reuse on
  bare `name=`, redefine on `name=value`, with a clear indicator). Reject names that
  collide with reserved words or that would create an expression cycle (§4.1).
- **A deleted parameter's name is free (#995).** Deleting a parameter removes it —
  `Document::parameters` is an `arena::Arena` keyed by `ParameterKey` (#1055), so nothing
  has to stay behind to keep the others addressable, and its name stops being claimed the
  moment it goes. A script names a parameter by **name or handle**
  (`add_parameter` / `set_parameter` / `delete_parameter`), resolved to a key at the script boundary.

#### 5.1.2 Derived parameters (#432)
- A parameter may be **driven by a measurement** (`Parameter::source`,
  `model::ParameterSource`): a line's length (`LineLength`, the original #measured flow),
  the world-space distance between two points (`PointDistance`, any two
  `ConstraintPoint`s — 2D or 3D), the distance between two **parallel** lines
  (`LineDistance`), the angle between two non-parallel **same-sketch** lines
  (`LineAngle`, stored in degrees), a **body feature edge's** length (`BodyEdgeLength`, #647),
  or the distance between two **body mesh corners** (`BodyVertexDistance`, #647, which may sit
  on different bodies). The two body kinds are keyed the way `SceneElement::BodyEdge`/
  `BodyVertex` are — the body plus quantized world points — and re-resolve against the body's
  live mesh (`body_edge_world_segment`/`body_vertex_world_position`), so they read the current
  geometry and go **unavailable** if a rebuild moves the edge/corner off that key, exactly as a
  deleted line's source does.
- Derived parameters are created from the **Dimension tool's** context-pane block
  (#618/#629, below); creation goes through `Action::CreateDerivedParameter` →
  `parameters::add_derived_parameter` (duplicate measurements are refused).
- Derived expressions are **read-only** (names stay editable) and re-sync from geometry
  on every rebuild (`sync_computed_parameters` → `derived_source_value`; lengths format
  in the document length unit, angles in the document angle unit). A derived parameter's
  row shows a **lock icon left of its name** (#631, `IconId::Lock`) whose hover text is
  the measurement it tracks — no "Driven by …" tag after the row.
- Focusing a derived parameter's row highlights its defining elements
  (`derived_source_elements` feeds `elements_using_parameter`).
- Focusing a derived parameter's **name field** (not its read-only value) additionally draws
  the source geometry — the measured line, line pair, or two points — in **green** in the 3D
  view (#536, `focused_derived_parameter_source` → `draw_derived_source_highlight`), so it's
  clear which geometry drives the value being renamed.
- The **Dimension tool measures in 3D mode** (#453/#618): outside a sketch it selects like
  the Select tool (measurable lines/points **and body edges/corners** hover-glow, #647, with a
  corner outranking the edge under it as in the click path; accumulated in the pane's element
  picker), and the context pane shows a **derived-parameter block**: a **Parameter name**
  text box **prefilled with the derived default name as editable text** (#629,
  `default_derived_parameter_name`; refreshed on selection changes until the user types
  their own — `AppState::dimension_param_name`/`dimension_param_auto`), a **Value** row
  with the selection's live measurement — one line → its length, two parallel lines → the
  distance between them, two non-parallel lines → the angle, two vertices → the distance, one
  body edge → its length, two body corners → the distance
  (`derived_source_from_selection`/`derived_source_value`) — and a **Derive parameter**
  primary button **labeled with visible text** (#629, `primary_text_button`) that records
  it as a read-only derived parameter and clears the name and selection for the next one.
  Nothing fires on click alone (the pre-#618 auto-capture is gone), the pane shows no
  Construction toggle in this mode (#630), and the Parameters pane's old "Derive from
  selection" button is gone (#629) — the Dimension tool is the one derive entry point.
- Scriptable: `bearcad.derive_parameter{ kind = "line_length"|"point_distance"|
  "line_distance"|"line_angle"|"body_edge_length"|"body_vertex_distance", a =, b =, body =,
  body_b =, name = }`. The body kinds take `a`/`b` as plain `{x, y, z}` **millimetre** points on
  the body's mesh, re-quantized to the selection grid — so they need only land on the picked
  geometry, not match bit for bit.

### 5.2 Expressions
- **Any input that accepts a value accepts an expression**, e.g. `1 + 2 + lengthOfThing / 2`.
- **One look for every value field (#881):** the floating dimension field the line tool
  draws with — amber frame, the typed expression in monospace, its computed value in
  smaller muted monospace *underneath* — is the one every **floating** tool field uses:
  extrude depth, sketch-vertex and body-edge chamfer/fillet amounts, the Move gizmo's
  X/Y/Z arrows, sketch offset distance, and the revolve angle (#881/#884–#888) — and, as of
  #889/#890, the **pane** fields too: `expression_input::ValueInput` draws through the same
  `expression_input::boxed::show`, so the Context pane's rows and the Parameters pane's value
  cells are the same control. One implementation, one look; the pane rows keep their own
  hint text, `no_definitions()`, and parameter-cycle context, and the floating fields keep
  the focus targeting, select-on-focus, and inline `name=value` commit layered on top by
  `show_sketch_dimension_field`.
- **One standard value input (#456):** numeric fields share `expression_input::ValueInput`
  — the styled expression field (autocomplete, error tooltips, inline `name=value`
  definitions) plus the **computed value on its own line inside the box whenever it differs
  from what was typed**, units included: a bare `10` in a length field previews `= 10.0 mm`
  (the default unit made explicit), `1in` previews `= 25.4 mm`, while `12.5 mm` (or any
  formatting-equivalent like `12.5mm`, `10mm`, `10.00 mm`) previews nothing
  (`value_input_computed_display`/`canonical_value_text` — trailing zeros after the
  decimal are not a difference). **Errors wait for a commit**
  (#824): half-typed text is always invalid — `thick` isn't defined until `= 5mm` lands —
  so the red text and error tooltip stay away while the field has the keyboard, and appear
  once Enter says "I meant that" (and go away again on the next keystroke). The computed value
  sits inside the box under the expression, and the **autocomplete dropdown opens below the
  box**, so the two never overlap (#793). **Tab** belongs to the autocomplete only while
  there is a name to complete (`autocomplete_has_candidates` gates the field's `lock_focus`);
  with nothing to complete it walks to the **next input** in the pane (#937). The same rule
  applies to live sketch/shape fields: first Tab accepts the highlighted name, a second Tab
  switches to the other dimension. Kinds: `Length` (document
  length unit), `Angle` (document angle unit), `Count` (unitless). Optional **min/max** clamp
  the computed value and show a warning (extrude taper angle: −90°…89°). The Parameters pane's
  value cells use it with **definitions disallowed** (the row is the definition) and
  cycle checking; the repeat panes (3D + in-sketch), the pane's Move X/Y/Z/Angle,
  sketch-text size/rotation/wrap, and calibration length all use it — the chamfer/fillet amount is also
  **mirrored into the Context pane** ("Radius" for a fillet, "Distance" for a chamfer) with
  the blue **Go** button the other tools commit with, so the treatment tools read like Move
  and the rest (#792). Deliberate exceptions: the
  drawing view **scale** field (ratio syntax `1:20`) and the page-dimensions editor
  (inch drag-values).
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
  A bare number **inside an expression** is read in the effective default unit (#1394), so
  `1.5` in an inches document is 1.5 in — internal storage stays mm/radians regardless.
  **Scripting (#1668):** every dimensional option of the Lua API is a ValueInput too — a
  number, or an expression string naming parameters/units — so a script can be as parametric
  as a hand-built model. A bare *Lua number* is the canonical mm/degrees the read-back calls
  (`line_endpoints`, `body_stats`) report; the string form follows the document default above.

---

## 6. Constraints

BearCAD has a geometric **constraint solver** supporting both 2D (sketch) and 3D constraints,
modeled on SolveSpace (https://solvespace.com).

**Constraint identity (#1055):** `Document::constraints` is an `arena::Arena`, and a
constraint is named by a `ConstraintKey` (`constraints::ConstraintId`) — `SceneElement`/
`HierarchyNode::Constraint`, `PickTargetKind::Constraint`, `DimEditTarget::Constraint`, the
dimension-label targets, the in-sketch operations' `constraint_outputs`, and the solver's
failed-constraint report. Deleting a constraint removes it, so a stale reference reads as
gone rather than naming whichever constraint took its place. The libslvs bridge still hands
the solver dense small handles — a constraint enters as its **ordinal** among the live ones,
and a failed handle maps back through that same ordering. A script names a constraint by its
ordinal too.

### 6.0 Constraint tool (implemented subset)

- **Tool:** Constraint, shortcut **`C`**. Distance/dimensional constraints remain on the
  **Dimension** tool (`D`).
- **Dimension flow — place, then type (#40/#763):** a **new** dimension is never committed
  the moment its geometry is picked. As soon as the selection describes something
  dimensionable — a fresh pick, or a selection carried in when the Dimension tool is chosen —
  the tool enters a **placement phase** (`AppState::placing_dimension`, a `PlacingDimension`
  holding the target and the pixel offset): the dimension is previewed in the preview color
  and follows the cursor, and a **click drops it there and moves on to typing the value**,
  carrying the placed offset onto the constraint's `dim_offset`. For an **angle**, two
  crossing lines have two distinct magnitudes (supplementary, one on each pair of opposite
  wedges) and whichever wedge encloses the cursor is previewed, with the arc's radius
  tracking the cursor's distance from the vertex (#188). For a **length**, the dimension line
  slides out to the cursor's perpendicular distance from what's being measured. A dimension
  that **already exists** skips placement and opens its value editor straight away — it
  already has a place on the sheet. The preview is painted **after** the GPU scene, so it
  isn't buried under it.
- The click that places the label leaves the value input **holding the keyboard** with its
  contents selected (#879), so a number or parameter name typed straight after lands in it —
  no second click on the field.
- **While the value is typed (#774):** the dimension it was just placed as **stays drawn** —
  extension lines, dimension line and arrows, or the angle's arc — but **without its number**,
  since that's what's being typed. The floating value input sits where the label will land,
  pushed one input-height further out along the same direction, so it never covers the
  dimension line (angles keep `angle_dim_edit_input_layout`'s bisector placement). Committing
  draws the finished label as normal.
- **Context pane (#775):** while a dimension is being typed the pane shows the same value in a
  mirrored `ValueInput` — labelled **"Angle"** for an angle and **"Span"** for a length — plus
  the blue **Go** primary button every other tool commits with (`DimensionEditControl` /
  `DimensionEditEdit`). Typing in either place feeds the same edit buffer.
- **Face edges hover too (#821):** the sketched-on face's own boundary edges are pickable
  (#26/#27) but have no `PickTargetKind` of their own, so nothing lit up for a click that
  worked. Select/Constraint/Dimension now highlight the edge under the cursor
  (`face_edge_hover`, drawn as a `Curve` highlight) while a sketch is open.
- **Hover follows selectability (#800):** inside a sketch the Dimension tool highlights
  **everything a click can take** — the sketch's lines and circles, its points, and the
  sketched-on face's own edges and corners (`element_in_sketch`, the same filter the click
  path uses) — not just the things that dimension on their own. A dimension is usually
  between two *different* kinds of thing, and half the picks used to light up nothing.
- **A single click never opens an existing dimension (#802):** clicking something that's
  already dimensioned just **selects** it (so it can pair with something else — a circle's
  centre to an edge, say); **double-clicking** it — or pressing `D`/Enter on the selection —
  opens its value for editing. Dimension labels keep their own click/drag behaviour.
- **Shift+click lands mid-edit (#780):** clicking an edge that already carries a dimension
  opens its value editor — and a **Shift+click** from there still reaches the viewport: the
  half-typed value stands down and the pick joins the selection, so a second edge turns it
  into the angle between the two. (Plain clicks keep going to the editor.)
- **Picking stays live while placing (#762/#763):** hovering anything else dimensionable
  stands the preview down and hovers that instead, so the next click goes to the pick, not to
  dropping the dimension. A **plain click** switches to dimensioning what was clicked
  (replacing the selection — clicking the same edge again just re-places it), and
  **Shift+click** adds the pick to what's selected: a second edge turns a length into the
  angle between the two, and the preview restarts for the new target. (This replaces #486/
  #487's "second plain click accumulates an angle" rule, which made a click meant for a
  different edge drop an angle dimension instead.)
- **Selection:** Sketch points (line endpoints — including a rectangle's corners — and circle
  centres), the sketch **origin**, lines (a rectangle's four edges are plain lines), and circles are selectable in the
  viewport. Point picks take precedence near vertices within the point pick tolerance.
  The origin plus a circle (or its centre) is a point-to-point distance — on a circular
  cap the origin *is* the extruded circle's centre, so this locates a hole from the
  centre. A circle paired with another point or the origin dimensions from its centre,
  not as a diameter.
- **Elements-pane hover → viewport highlight (#161):** hovering any row in the Elements
  pane (List or Graph view) highlights that element in the 3D viewport using the
  standard hover color: sketch entities get their usual pick highlight, a hovered sketch
  row highlights all of its entities, a construction plane its fill, and a body or
  extrusion a recolor in the hover color (#455). Drawn depth-test-disabled like other
  pick highlights (#153).

  **Every** row lights something (#977), and `every_pane_row_lights_up_when_hovered` guards it.
  Rows whose element isn't in the 3D view at all — a history operation, a component, a joint —
  light what they **made**: an operation its output bodies, a component every body under it,
  a joint the parts it joins (`hierarchy::produced_bodies`, which reads the same descendant
  map the pane's own tree does rather than re-deriving outputs per op kind). Those wear
  `DERIVED_OUTPUT_HIGHLIGHT`, a color of their own rather than the hover color: the hover
  color means "this is the thing under your cursor", and the cursor is on the row, not on the
  body. A joint additionally marks its **badge**, the one part of it that is in the view. A
  tracing image outlines its quad. The one row that adds no overlay is a **body**, which
  recolors in the main pass instead (#455).
- **Hover picking rejects on bounds first (#1026).** `resolve_pick_target` runs every frame
  the camera moves, and used to project **every triangle of every body** to answer what is
  under the cursor. Orbit and pan hid this because both suppress hover while a mouse button is
  down (`suppress_viewport_pick_hover`); the wheel doesn't, so zooming over a large document
  lagged while orbiting the same document did not. Each per-body walk — faces, edges,
  vertices, cylinder axes — now rejects a body on its **screen-space bounds**
  (`construction::screen_bounds_hit`, eight projected corners) before touching its mesh, and
  `pick_body_face` rejects each coplanar group the same way. The test is conservative by
  construction: a corner that can't be projected (behind the camera) *accepts* the box, since
  a wrong rejection silently drops a pick.
  **The bounds are fetched batched** (`extrude::body_world_bounds_all`), and that matters more
  than the rejection itself: every cached mesh accessor keys on `document_pose_fingerprint` /
  `document_mesh_fingerprint`. Geometry caches use a process-wide **revision counter**
  (`Document::mesh_rev`, #1027) bumped on each mutating `apply` — an integer compare, not a
  JSON serialize of the model. Pose caches still hash joint positions cheaply on top. Asking
  per body inside a loop used to cost
  one full document hash per body per frame. Measured on 20 faceted cylinders (~19k triangles,
  debug build): 8.5 ms per pick before, 3.4 ms with per-body rejection, **87 µs** once the
  bounds lookup was hoisted out of the loop.
- **3D body sub-element selection (#156/#555):** outside sketch mode, the Select tool can select
  a body's **edges, vertices, and faces** (the same feature edges/corners/faces the hover highlight
  shows, #144), not just sketch entities. Shift/⌘-click multi-selects them like any other element.
  Their selection identity is the quantized geometry (not a stable topological name): an edge/vertex
  by its quantized endpoint(s), a **face** (`SceneElement::BodyFace`) by its quantized
  centroid+normal; if a rebuild moves it the selection simply drops — acceptable for ephemeral,
  never-persisted selection state. A selected face is re-found among the body's coplanar-face groups
  by matching that key and shaded; selected body edges/vertices/faces draw depth-test-disabled like
  their hover highlights (#153). `resolve_pick_target` offers a face as a **priority-1** candidate
  (below edges/vertices at priority 0, via `PickOcclusion::eye`), so clicking near an edge still
  picks the edge and clicking a face **interior** selects the face (#565).
  **Round walls are cylinders, and have axes (#1013):** a circular profile facets finely
  enough that its whole wall already merges into one coplanar group, but that group is no
  plane — calling it a flat face gave it a nonsense normal. `extrude::fit_cylinder` names it
  for what it is: the axis from the fan of normals, the radius from a least-squares circle
  through the points projected square to it, and two gates that tell a round wall from a
  faceted prism — every facet must face straight out from the axis, and consecutive facets
  must sit close enough round it (a box's four walls fit a circle through their corners
  perfectly well). It comes back as `SceneElement::BodyCylinder` (its own
  `ElementKind::Cylinder` — a picker wanting something to sit a part flush on must not be
  offered one), and its **centre line** as `SceneElement::BodyAxis`, derived geometry with no
  owning entity like `GlobalAxis`, picked at edge priority and usable anywhere a straight
  reference is: a joint's Axis (#1079), a Revolve axis, a Repeat path. Fitted from the
  mesh, so an imported part gets them as readily as a modelled hole; memoized alongside the
  face groups (`extrude::body_cylinders`), with an un-posed twin for mate resolution.
  Scripting: `bearcad.body_cylinders(i)` reports each wall's axis, radius and length, and
  `bearcad.body_faces(i)` reports the **flat** faces only.
  **Curved edges select as whole curves (#626):** feature segments are partitioned into maximal
  tangent-continuous chains (`solid_mesh_edge_chains`: at every vertex where exactly two feature
  segments meet under a 30° turn — a smooth curve's tessellation — they join one chain; corners
  and junctions break it). Every segment of a chain carries the chain's **canonical segment**
  (its lexicographically-smallest quantized member) as its pick identity, so clicking any facet
  of a revolve's circular rim selects the *whole* circle as one edge; hover, selection, and
  loupe previews all draw the full chain (`body_edge_curve_chain`, endpoint-matched by proximity
  since selection geometry round-trips the coarser `quantize_body_point`). The exploder crowd
  dedupes a curve's facets into a single candidate the same way.
- **Edge pickers take the whole tangent run (#984):** the same rule reaches **sketch lines**.
  Hovering or clicking any line takes it plus every line tangent-continuous with it in both
  directions — a straight line that breaks into a tangent curve and exits again as a tangent
  line is one thing to pick, not three. The chaining is one shared union-find,
  `gpu_viewport::chain_by_tangency`, over each item's two `(vertex key, away-direction)` ends;
  `solid_mesh_edge_chains` and `element_picker::sketch_line_tangent_chain` differ only in how a
  vertex is keyed and a tangent read (a curved line's tangent at an endpoint is its **bezier
  handle**, a straight one's is its chord). The 30° threshold, the exactly-two-ends rule, and
  the corner/junction breaks are shared, so the two kinds of edge behave alike. Only the line's
  own sketch participates; deleted and shadow lines don't.
  The run is applied in `element_picker::expand_pick`, so every path that picks gets it at once
  — the viewport click, the Elements-pane click, and the scripted pick — and a picker that
  refuses part of a run (a `Straight` axis picker meeting a run's curve) simply keeps the
  members it accepts. Three deliberate exceptions: a **single-slot** picker never chains (the
  run has nowhere to go, the same reason it never takes a face's edges, #955); the **Dimension**
  tool never chains (a dimension measures one segment, or a pair); and the **Exploder's fan**
  never chains, since its job is to tell a crowd's members apart and a run would give every
  line of one run the same dedupe key — the chaining happens when its leaf is clicked, through
  the ordinary click path. Selecting a run is one act (`selection::click_scene_selection_many`):
  all-selected takes the whole run out, anything short of that completes it, so a partial
  selection converges on selected rather than toggling piecewise.
  Holding **Control** (`AppState::pick_single_edge`, mirrored from the frame's modifiers next to
  `tool_pickers`) picks only the edge under the cursor — on **every** platform, which is what
  moving the additive modifier to Shift alone bought (see below). Shift still composes with it:
  a Shift+Ctrl+click adds the one edge to the selection rather than its run.
  The hover says what the click does: the run's other members ride the same
  `extra_pick_highlights` channel a hovered exploder group loupe uses, which draws each line's
  real curve — rather than the multi-take `Curve` the picker-driven hover fallback reduces a
  set to, which would show a bezier as its chord.
- **Shift alone is the additive click (#984):** `selection::additive_click_modifiers` is
  `modifiers.shift`, on every platform. ⌘/Ctrl used to add to the selection as well, and that
  cost more than it bought: egui's `command` is Ctrl everywhere but macOS, so on Linux and
  Windows Ctrl was already spoken for and could not also mean "the single edge, not its run".
  One modifier, one meaning, the same keys everywhere — which is what lets Ctrl be the
  single-edge pick on every platform rather than only on the Mac.
- **Whole-body selection with the Select tool (#902):** outside sketch mode, clicking a body's
  flat **face** selects the **whole body** — bodies outrank faces — while an **edge** or a
  **corner** still outranks the body it belongs to. Hover follows the click: a face under the
  cursor recolors the whole body. The face itself stays reachable through the Selection Exploder,
  whose crowd fans a `PickTargetKind::Body` leaf (one per body under the cursor, grouped ahead of
  its faces) alongside each face/edge/corner leaf; only the Select tool takes that leaf. The
  Constraint and Dimension tools keep picking the face itself.
- **Selection Exploder (#551):** pressing **Space** — when no field has the keyboard, so a
  space typed into an expression stays in the expression (#794) — fans the crowd of pickable
  things inside the
  cursor's hitbox out into spaced-apart **handles** arranged on a ring around it, so a tiny buried
  vertex/edge/line/face can be picked unambiguously. Each handle is a round **loupe** — `ZOOM ·
  touch::hit(12)` radius (`exploder::loupe_radius()`, `ZOOM = 4`) — that draws the hitbox region
  **magnified `ZOOM`×** with the one element that handle stands for drawn on top and the rest of the
  crowd dimmed grey behind it, joined by a 1-px leader line back to where it really is; geometry is
  painter-drawn and clipped to the loupe disc (`draw_pick_target_loupe` / `clip_segment_to_disc`),
  so overlapping things read clearly. A loupe whose own thing has **no wireframe inside the disc**
  at that magnification — a **whole body**, a face far bigger than the hitbox — would show an
  empty or flat-colored disc with nothing to recognise, so it **zooms out to frame that thing**
  instead (#944/#945, `loupe_view` / `pick_target_loupe_wireframe`): the magnifier is kept
  whenever any of the thing's own segments or points lands in the disc, and only ever traded for
  a wider view, never a tighter one. The faint centre mark still shows where the cursor falls in
  the framing, or is dropped when that lands outside. Faces are **shaded** (translucent fill clipped to the disc via
  `clip_convex_to_disc`), not just outlined. A **whole body** likewise draws as its **shaded
  solid** (#972, `body_loupe_faces`): its mesh triangles painted far-to-near from the camera,
  flat-shaded by the same two-sided Lambert term the viewport uses, with no outline — so the
  loupe reads the way the body reads in the 3D view rather than as a see-through wireframe box.
  Both are for the **highlighted** thing only; a *context* body or face stays outline-only, since
  filling one would blanket the disc and bury the loupe's own subject.
  Only the **highlighted** vertex shows a dot; a
  line-endpoint vertex also gets a short **stub** of its line — a *fixed on-screen length* relative
  to the loupe, not a fraction of the line — so coincident endpoints of different lines are told
  apart by the direction their line leaves the vertex. Colors: the in-loupe element and its ring
  read **blue** while idle and turn the accent **yellow** the moment the loupe is hovered *or* its
  thing is selected, matching that element's own highlight out in the 3D view. The crowd spans
  **everything inside the hitbox, front and back** — sketch points/lines/circles, body
  vertices/edges, and **every body face near the cursor** (all of them, not just the nearest ray-hit
  one, so a narrow face seen edge-on and faces buried behind others each get their own loupe, #555/
  #556): `construction::collect_pick_candidates` returns them as `PickTargetKind`s with a world
  anchor and, unlike normal picking, does **not** occlusion-filter (buried things are exactly what
  the exploder is for), only respecting user hidden/shadow visibility. Faces are found by
  `face::body_faces_near` (min screen distance to each coplanar group's projected triangles ≤ the
  hitbox). The crowd also includes any **constraint annotation badges** whose hitbox is under the
  cursor (#568): `tick_exploder` appends a `PickTargetKind::Constraint(index)` for each drawn
  constraint icon (`constraint_hits`, deduped) anchored at `constraint_viewport::constraint_icon_anchor`,
  so a constraint icon buried beneath overlapping geometry can be fanned out and selected. Its loupe
  shows the constraint's icon glyph (`icons::paint_icon`), and hovering it (or a group of them) glows
  the real badge in the annotation overlay via `draw_constraint_icons`'s hovered set. The crowd is
  then pruned to what the **focused picker** can actually take (`exploder_keep_for_picker`,
  #560/#957) — e.g. the Extrude tool's Profile picker takes faces, so its fan holds only faces,
  never a corner's edges/vertices it couldn't use; constraint badges appear only where a picker
  takes constraints (Select, Constraint, Dimension), which apply the pick as a
  `SceneElement::Constraint`. One leaf per distinct thing the picker would take: a picker that
  takes whole bodies turns a face, an edge and a corner of one body into a single **body** loupe
  rather than three, and the leaf is relabelled as that body so the loupe shows what a click
  gets. **With no picker armed the fan does not open** — a tool that draws rather than picks
  (Rectangle, Line, Circle) has no pick to disambiguate. The crowd also carries
  **analytic sketchable faces** (`PickTargetKind::SketchFace`, exploder-only like `Constraint`,
  #625): every `face::pick_sketch_face` candidate near the cursor — sketch profiles,
  extrusion caps/side walls, revolve flat faces, construction planes — via
  `face::sketch_faces_near`. And the **world axes** and the **datum planes** in their own right
  (#975): each is pickable — a Revolve axis, a Repeat path, a plane anchor, a Slice cutter all
  take one — so each belongs in the crowd, or the fan can't offer what the armed picker is
  asking for. An axis is anchored at the point on it nearest the cursor, so its loupe's leader
  line points at the bit being picked rather than at the world origin. **Everything a picker can
  take must reach the crowd**; the filter that prunes it is the picker's, not the enumerator's. These are `ElementKind::Profile`, a different kind from the mesh
  `Face` over the same surface (§11.4a), which is what lets a picker say which representation it
  wants. The Extrude tool fans **exactly these** (minus construction
  planes, which its Profile picker doesn't take) instead of raw mesh facet groups, so its fan
  matches its own pick path: the sketch
  profile buried under a body appears, a curved surface it can't use doesn't. The **Sketch and
  Text** tools fan the same analytic faces **including construction planes** (#860) — a datum
  plane behind a body is exactly what the fan is for — and a click applies the chosen face
  directly (`Action::BeginSketch`) rather than re-picking at the redirected anchor, which
  would land back on whatever is in front of it. A revolve side's
  crowd anchor is the unrotated edge midpoint — always on the face, unlike a full washer's
  boundary centroid, which sits in its hole. It activates **on demand**: over a crowd it fans several handles, over a
  single thing just one, and over nothing it freezes the hitbox circle at the cursor with no
  handles. A faint **light-green** disc the size of the hitbox (`construction::EXPLODER_HINT_RGBA`,
  distinct from the yellow pick-hover) appears under the cursor when **two or more** things are
  there, as a hint. Handles sit at least a loupe apart (chord, not arc) so there's never ambiguity
  about which one a click means. Each loupe sits **on its own element's direction** (#570/#671):
  `display_centers` widens the ring past the just-fits radius by `exploder::RING_SLACK` so an even
  fan isn't the only arrangement that fits, then `fan_angles` starts every loupe at its element's
  own angle and pushes neighbours apart only where their discs would touch (Gauss-Seidel
  separation, wrap included). A bunched crowd therefore stays bunched on its side of the cursor
  instead of fanning evenly around the whole circle; the resulting `base` angle also orients the
  staggered concentric rings.
  An item's direction comes from its **anchor**, the spot on it nearest the cursor — but a crowd is
  by definition things stacked *at one point*, so those anchors all collapse onto the cursor and
  every direction reads the same. Each handle therefore also captures a **reach** (`fan_reach`,
  #671): a point further along the element — a line's far endpoint, a circle's centre, a face's
  centroid — and `display_centers` falls back to it whenever the anchor projects within
  `DEGENERATE_PX` of the origin. A **line-endpoint vertex** has no extent of its own but its loupe
  draws a short leg of its line, so it reaches for that leg's centre (`line_leg_center`, #673):
  the point half of `LEG_FRACTION` along the line from that end, walked along the polyline so a
  curve's leg reads as its local tangent rather than pointing at the far end. Any other vertex
  draws a bare dot and keeps its anchor; a group loupe averages its members' reaches. While exploded the camera is
  frozen, so the **mouse wheel zooms the loupes** instead (`ExploderState::zoom_mul`,
  `display_centers`): the fan grows, and while the angle-coherent single ring still fits it just
  scales; once the growing loupes would push that ring off-screen they **stagger** into concentric
  rings — a centre loupe plus rings filling outward, so some sit closer to the cursor — and the whole
  cluster is **shifted** to stay inside the viewport (`fit_offset`). Zoom is **clamped** (`max_zoom_mul`) where those rings fill the space, so loupes
  never grow off-screen and, at full zoom, they tile the whole viewport. While
  exploded **only handles** are hoverable/selectable — the raw crowd underneath is suppressed (the
  positional sketch pick/drag handlers stand down), and so do the **positional grab** handlers
  that act on an already-selected thing: a construction plane's corner grips and the Select
  tool's joint drag see **no pointer at all** while the fan is open (#986). They must, because
  the pointer they would see is the *redirected* one, aimed at the picked thing's anchor rather
  than at the cursor — and a plane's anchor is its **origin**, which falls inside a corner
  grip's radius whenever the view is zoomed out far enough. Clicking a plane's loupe then
  selected the plane and, in that same frame, grabbed the grip that had just come live under
  the redirected pointer; holding the button for the rest of the click dragged that corner out
  to where the loupe was. A grab already in hand still finishes — with no pointer it stops
  tracking and commits its extent on release — and hovering a handle highlights its **exact**
  target** (the whole line/edge/face) out in the 3D view, not whatever a re-resolved pick at the
  anchor would catch. **Every** kind the crowd can offer lights up that way (#974); the renderer's
  `PickTargetKind` hover is total, and the only two kinds that draw nothing *there* draw elsewhere
  in the same frame — a constraint's badge glows in the 2D annotation overlay (#568) and a whole
  body recolors in the main pass (#902), through **both** hover channels: the hovered leaf's
  `PickTarget(Body)` and a hovered group's members reach the body-fill recolor just like an
  Elements-pane row hover does (#985) — the pick-target hover pusher has no marker of its own
  for a whole solid, so the main pass is the only place a body hover can read. A kind that reaches the cursor as more than one pick
  target — a datum plane arrives both as itself and as the analytic face over the same surface,
  and `collect_pick_candidates` keeps whichever is nearer — must draw the same for each, so both
  go through the one face-hover helper rather than through a case per call site. **Clicking a handle selects that handle's exact target** (`scene_element_from_pick` for
  the selection-family tools) rather than re-resolving a pick at the redirected anchor, which would
  be ambiguous for an overlapping crowd or land on something outside the hitbox; for other tools
  the pointer is still **redirected** to the hovered handle's anchor so their own pick path runs.
  A loupe's contents wear **their own colors**, not the loupe's accent (#976/#979) — the loupe
  magnifies the scene, so what's in it should look like what's out there. Solids take their
  **material**, shaded per triangle; the **world axes** their red/green/blue, which is the only
  thing telling X from Y from Z; a **datum plane** its shaded quad with an outline, rather than
  the single dot it used to draw, which was indistinguishable from a vertex. Which loupe is hot
  is the **ring's** job (accent yellow, and thicker), which is what frees the contents to be
  their own color — **in the state it's in** (#980), by the 3D view's own rule: selected, then
  hovered, then the material. The material alone made a hovered loupe indistinguishable from a
  cold one, since the fill is the whole visual for a body and a ring around the disc is not the
  signal a hover needs. Every kind has content to magnify (`every_loupe_has_content`), including
  the two whose content is one spot by nature: a point on the ground, and a constraint, whose
  visual is its badge glyph at the loupe's centre.

  A loupe is **painted**, not depth-buffered, so a solid in one is drawn far-to-near and the
  order is the whole correctness argument (#981). The key is each triangle's **farthest** vertex,
  not its centroid: a big side wall whose centre is far can still have a corner nearer than
  everything in front of it, and a centroid key lets it paint over them. Each triangle is also
  stroked in its own fill, because egui feathers a polygon's edge for antialiasing and two
  triangles sharing one each fade out along it — across a mesh that reads as a web of cracks.
  A hovered leaf also draws a thin grey **leader line** from the element back to the **edge** of its
  loupe (not its centre, #572). Clicking **outside** every loupe just dismisses the fan and **leaves
  the selection untouched** (#575) — the dismiss frame redirects the pointer to nothing so the normal
  pick is skipped.
  Clicking collapses the fan; holding **Shift** keeps it open for multi-select; pressing **Space**
  or **Esc** again, or clicking empty space, dismisses it. While it's open the **camera is frozen**
  (orbit/pan/zoom stand down) so the handles stay put while you aim. The enumerator
  (`construction::collect_pick_candidates`) is the crowd-returning counterpart to
  `resolve_pick_target` (which keeps only the nearest). Suppressed only during a drag/gizmo, an
  in-progress draw, or a dimension sub-state. A keyboard trigger, so desktop-oriented.
  - **The crowd's order is total, and nearest-the-camera first (#987):** `(screen distance,
    depth from the eye, crowd key)`. The cursor sits *inside* every face it is over, so all of
    them tie at distance 0 and only depth can separate them — which is what makes the ordinary
    pick take the face you can see and leaves the one buried behind it to the fan. The dedupe
    is a `BTreeMap`: with a `HashMap` its randomly-seeded per-instance iteration order survived
    the stable sort, so the same crowd came back differently ordered on every call and a hover
    over two stacked faces flickered between them frame after frame. This only **orders** the
    crowd — nothing is pruned, so the fan still offers every buried face (#556), which is the
    only way to reach one.
  - **Hierarchical loupe grouping (#559/#563):** a level never shows more than **12 loupes**
    (`exploder::MAX_LOUPES`). When the crowd already fits within that cap it is **not grouped at all**
    (#571) — every item is its own leaf. Only a larger crowd is grouped so the level stays within it (a mix of
    single-element and group loupes). On Space the flat crowd (`ExploderState::items`) is built into a
    grouping **tree** (`ExploderNode::Leaf` / `Group`) by `build_exploder_tree`: the **top level
    groups by element type** (#563) — all faces in one group, all **edges** in one (sketch line
    segments and body feature edges share a single "edges" rank — a sketch line and a shape edge are
    the same kind of thing for the exploder), all circles, all vertices (via `crowd_type_rank`),
    keeping **all** of a type together in its group; a
    type with a **single** member is shown as a bare leaf, never a group of one (#567). Each type
    group is then subdivided **by proximity** (joined/touching things) with `build_spatial_tree`,
    which partitions the screen anchors with a deterministic farthest-first `cluster_points` (no
    RNG/clock — both throw here); when clustering can't split the set into ≥ 2 non-trivial pieces
    (e.g. **coincident** stacked endpoints) it falls back to even `chunks`, which always shrinks the
    input so the recursion terminates. (If the crowd spans more distinct types than `MAX_LOUPES` —
    rare — it falls back to pure spatial clustering so the level still fits.) Spatial groups recurse
    with a tighter arity of **11** (`exploder::GROUP_ARITY = MAX_LOUPES − 1`) so a drilled level
    (Back + ≤ 11) still totals ≤ `MAX_LOUPES`. **Every loupe shows the whole crowd** dimmed for context,
    with only its own thing(s) highlighted — a leaf highlights its one target, a **group loupe**
    highlights all its members (idle blue). A group reads distinct via a **blue border** — the same
    blue as its highlighted members (#562) — and a **big count badge** (member count, bottom-right). **Clicking a group loupe
    drills in** (`ExploderState::path` pushes its node index): the group's members **spring out** of
    the clicked spot as their own loupes at the new level (recursively grouped if still too many),
    while the **siblings** you drilled away from converge inward and gather into a loupe-sized
    **cluster loupe** — a mini-loupe per sibling (#561), each carrying a shrunken **count badge** when
    it stands for a multi-member group (#573); clicking the cluster pops one level. When there is
    exactly **one** sibling the cluster-of-one is skipped: that sibling is shown as a direct loupe
    (`DisplayItem::swap_to`, #574) — a group sibling becomes a **swap loupe** whose click collapses the
    current group and expands it (`path.pop()` then `path.push(sibling)`), a lone leaf sibling is just
    its selectable leaf loupe. Both
    directions of the transition are animated over `DRILL_ANIM_SECS` by a `DrillAnim` (#567): each
    arrived-level loupe slides out from a recorded start position (`DrillAnim::from`, index-aligned
    with `display_items`) while the **departed** level dissolves as fading `DrillGhost` loupes that
    slide toward the gathering point — the siblings into the Back cluster on drill-**in**, the group's
    children into its group loupe on drill-**out**. `ExploderState::build_drill_in_anim` /
    `build_drill_out_anim` capture the from/ghost positions from the old and new `display_centers` at
    click time. Any depth is supported. The current level's loupes and
    their ring/stagger/fit layout come from `display_items` / `display_centers`. Only a hovered
    **leaf** loupe redirects the tool's pick, owns the press, and sets the hover highlight
    (`ExploderState::hovered_leaf`); a hovered **group** loupe instead lights up **all its members**
    at once in the 3D view (`hovered_group_members` → the scene's `extra_pick_highlights`); a back
    loupe only navigates. Single-element loupes still select their element on click, unchanged.
- **How selection works** is normative in **§11.4a**, not here: pickers, focus, hover, the
  Exploder's fan, pick priority, pane clicks, tool-switch handoff, and highlight color. This
  section covers what the *constraint tool* selects, not how selecting works.
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
- **Selected-body fill (#174):** a selected body's solid also fills in a **more saturated
  blue** than the neutral body grey (in every shading mode), so selection reads on the body itself (#455).
- **Selected/hovered-body highlight (#455/#1110/#1155, replacing the #145/#148 aura):**
  selection and hover always apply **both** solid-body shading and a screen-space silhouette
  outline. In shaded modes the fill changes (`SOLID_FILL_SELECTED` saturated blue;
  `SOLID_FILL_HOVERED` warm gold-grey); in wireframe mode the **lines** recolor instead; and
  a mask pass always strokes a blue (selected) or yellow (hovered) outline around the body's
  silhouette. Selecting or hovering an **Extrude** element recolors only that extrusion's own
  solid within its (possibly merged) body (`push_sub_body_recolor`: a translucent overlay on
  the toward-camera-biased Overlay layer in shaded modes, its feature edges in wireframe).
  Destructive (cut-picker) bodies keep their red translucent fill (#264).
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
    order — Parallel `1`, Perpendicular `2`, Equal `3`, Coincident `4`, Midpoint `5`, Parallel-to-X-
    axis `6`, Parallel-to-Y-axis `7`. Pressing the digit **while the Constraint tool is active**
    applies that constraint if it is currently enabled; the digits do nothing on other tools, so
    they can't collide with global tool keys.
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
  - **Parallel to X axis / Parallel to Y axis (#583)** — `line`: one-click buttons (keys `6`/`7`)
    that constrain the single selected line parallel to the sketch's X or Y origin axis — the
    convenience form of "make this horizontal/vertical". They author a `Parallel` constraint against
    `OriginAxis(X)`/`OriginAxis(Y)`; the scripting names `horizontal`/`vertical` map to them for
    back-compat. Their pane buttons are **hand-painted glyphs (#751)**: a double-headed arrow in the
    axis's own color (X red, Y green) drawn at the axis's **current on-screen direction**
    (`ContextInput::sketch_axis_screen_dirs`, the projected local axes normalized — rotated with the
    view, never skewed), so which way the line will snap always matches what the viewport shows.
    The viewport itself labels the local axes at its edge (#751): **"LX"/"LY"** in the axis colors,
    where the axis line meets the view (`axis_label_edge_pos`) — the nearer of the two exits so a
    lopsided origin doesn't park the label on the far side (#1216), preferring the positive
    direction when equidistant — nudged **perpendicular** onto whichever side has room so the
    axis line never runs through the letters (#771, `axis_label_offset_pos`).
  - **No separate Horizontal/Vertical constraint *kind* (#577/#580):** the old standalone constraints
    were removed entirely in favour of the general **parallel-to-axis** solution (the buttons above,
    or select a line **and a sketch axis** and apply **Parallel**/**Perpendicular** directly).
    Because it refers to the
    sketch's own X/Y axes rather than the screen, it is unambiguous on any plane at any angle, so the
    camera no longer has to force a u-right/v-up orientation. The `ConstraintKind::Horizontal`/
    `Vertical` variants are **gone**; documents that still contain the legacy `horizontal`/`vertical`
    tags load via a serde `from` shim (`ConstraintKindWire`) that maps them to `Parallel` against the
    X/Y origin axis, so old files keep working and are silently upgraded on save. The Rectangle tool
    constrains its edges parallel to the X/Y axes, the sketch solver routes a line-parallel-to-axis
    to its dedicated (robust) horizontal/vertical equation, and the vertex-drag projection and the
    constraint badge icon both treat a line parallel to X/Y as horizontal/vertical.
- **Redundant-constraint cleanup:** when a point already constrained coincident with a line
  is then constrained to a *specific* point on that same line (one of its endpoints, or its
  midpoint), the earlier generic point-on-line coincidence is removed in favor of the more
  specific constraint.
- **Scripting:** `bearcad.constrain("parallel", a, b)` with explicit operands; selection-based
  apply is `bearcad.ui.add_geometric_constraint` (UI tests). Circle tool shortcut is **`O`** (`C` is constraint).

### 6.1 2D sketch constraints (full set)
Coincident, point-on-entity, parallel, perpendicular (horizontal/vertical are expressed as
parallel/perpendicular to a sketch axis, #577/#580), tangent,
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
  compile and statically link it via build.rs, and the web
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

A `.bearcad` is a SQLite database. One table per arena; `id` is `Key::to_bits()`.
Native open sniffs the 16-byte SQLite header (web JSON still works). Session RAM is
still the live model — hydrate into `Arena`s on open.

No `FOREIGN KEY` constraints. A dangling `sketch_id` / `material_id` loads; document
health reports it.

### 7.1 Versioning & migrations
- A `schema_migrations` table records every patch applied, so older files can be upgraded:
  ```sql
  CREATE TABLE schema_migrations (
    id          INTEGER PRIMARY KEY,   -- ordered migration id
    name        TEXT NOT NULL,         -- human-readable migration name
    applied_at  TEXT NOT NULL          -- ISO-8601 timestamp
  );
  ```
- Current version: **2** (`typed_entity_tables`). A newer file than the running binary
  is refused. Pre-alpha: no reader for the v1 `dag_nodes` dump.
- `meta` holds scalars only: `app_version`, `occt_version`, `schema_version`,
  `default_length_unit`, `default_angle_unit`.

### 7.2 What is persisted
- **Every arena** as its own table: parameters, sketches, lines, circles, constraints,
  construction planes, bodies, ops, drawings, joints, units, …
- **Typed columns** for names, expressions, flags, numeric fields, and integer refs
  (`sketch_id`, `material_id`, `unit_id`). `SELECT name FROM parameters` just works.
- **JSON** only for tagged unions (`FaceId`, constraint entities, `BodySource` extras,
  face lists, move points).
- **`blobs`** for fonts, tracing images, STEP bytes, packed mesh triangles, preview
  PNG/STL. First save of a new path is a full typed write, atomic (`*.tmp` + rename).
- Undo remains session snapshots (cap 200), not a SQL history. Nested units
  store the embedded copy as a `.bearcad` blob in `units.document`.
  `geometry_cache` holds per-body tessellation (optional BREP); open warms the
  in-memory mesh cache on fingerprint match.

### 7.2a Session I/O

Once a document has a path, the tab keeps a live SQLite connection. Each
committed edit (outermost `apply`, plus undo/redo) diffs arenas against the last
flushed document and `INSERT`/`UPDATE`/`DELETE`s only the changed rows **inside
one open transaction**. In-progress drags stay RAM-only. A body's
`geometry_cache` row is written when that body's mesh exists; other bodies are
left alone.

- **Save** (`⌘S` / Save As) = `COMMIT`, then refresh preview PNG/STL + OS
  thumbnail + unit save-ping, then `BEGIN` a new transaction.
- **Discard / quit-without-save** = `ROLLBACK`. Dirty `*` means the transaction
  has uncommitted changes. Untitled has no file until first Save As (full typed
  write, then `BEGIN`).
- Another connection (QuickLook, import-as-unit) still sees the last `COMMIT`.
- First save of a new path is a full typed write (`*.tmp` + rename). Save As to
  a new path `COMMIT`s the source if dirty, then writes the new file and
  switches the live connection.
- **No WAL** sidecars on a user document (`journal_mode=DELETE`).
- Web JSON still uses `to_json_bytes` / `from_json_bytes`.

### 7.3 Schema
```sql
CREATE TABLE meta   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE blobs  (id INTEGER NOT NULL, kind TEXT NOT NULL, bytes BLOB NOT NULL,
                     PRIMARY KEY (id, kind));
CREATE TABLE parameters (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, expression TEXT NOT NULL,
  is_primary INTEGER NOT NULL DEFAULT 1, minimum TEXT, maximum TEXT, step TEXT,
  source_json TEXT);
CREATE TABLE sketches (
  id INTEGER PRIMARY KEY, name TEXT, length_unit TEXT, angle_unit TEXT,
  face_json TEXT NOT NULL);
CREATE TABLE lines (
  id INTEGER PRIMARY KEY, sketch_id INTEGER,  -- may dangle
  x0 REAL NOT NULL, y0 REAL NOT NULL, x1 REAL NOT NULL, y1 REAL NOT NULL,
  construction INTEGER NOT NULL DEFAULT 0, shadow INTEGER NOT NULL DEFAULT 0,
  length_locked INTEGER NOT NULL DEFAULT 0, length_expr TEXT, name TEXT,
  payload_json TEXT);                         -- bezier, projection, chamfer parent
CREATE TABLE constraints (
  id INTEGER PRIMARY KEY, sketch_id INTEGER, kind TEXT NOT NULL,
  expression TEXT NOT NULL, name TEXT, payload_json TEXT NOT NULL);
CREATE TABLE bodies (
  id INTEGER PRIMARY KEY, source_kind TEXT NOT NULL, source_id INTEGER,
  material_id INTEGER, name TEXT, shadow INTEGER NOT NULL DEFAULT 0,
  source_json TEXT NOT NULL);
CREATE TABLE components (
  id INTEGER PRIMARY KEY, name TEXT, parent_id INTEGER,
  length_unit TEXT, angle_unit TEXT);
CREATE TABLE component_members (
  member_kind TEXT NOT NULL, member_id INTEGER NOT NULL, component_id INTEGER NOT NULL,
  PRIMARY KEY (member_kind, member_id));
CREATE TABLE units (
  id INTEGER PRIMARY KEY, source_json TEXT NOT NULL, link TEXT NOT NULL,
  source_mtime INTEGER, source_hash INTEGER, document BLOB NOT NULL);
CREATE TABLE unit_instances (
  id INTEGER PRIMARY KEY, unit_id INTEGER, name TEXT,
  tx TEXT, ty TEXT, tz TEXT, axis_x REAL, axis_y REAL, axis_z REAL, angle TEXT,
  overrides_json TEXT);
CREATE TABLE shape_order (seq INTEGER PRIMARY KEY, kind TEXT NOT NULL);
CREATE TABLE undo_groups (seq INTEGER PRIMARY KEY, size INTEGER NOT NULL);
CREATE TABLE geometry_cache (
  body_id INTEGER PRIMARY KEY,
  fingerprint INTEGER NOT NULL,   -- source tree + OCCT version
  occt_version TEXT NOT NULL,
  mesh BLOB NOT NULL,
  brep BLOB);
-- plus one table each for circles, planes, extrusions, materials, meshes, images,
-- lofts, revolutions, primitives, sweeps, boolean/move/mirror/repeat/slice/shell/
-- edge-treatment ops, in-sketch ops, sketch texts, drawings, joints.
```
Same pattern everywhere: new feature = new table or new columns. Preview PNG/STL are
`blobs` rows (`kind` = `preview_png` / `preview_stl`).

### 7.4 Imported units (#719)

A document can embed other BearCAD documents as **units**: `Document.units` holds one
embedded copy per imported source — its source (a path relative to the
importing file, or a library path, §11.z), a link mode (static or dynamic, #732), the
embedded `Document`, and staleness provenance (source mtime + content hash) — and
`Document.unit_instances` are the placements: unit id, instance
name (for qualified expression references, #729), parameter overrides, and a placement
transform. On disk the embedded copy is a nested `.bearcad` blob (`units.document`);
instances stay rows. Nested units recurse as blobs. Ten instances of one part cost one
embedded copy plus ten override lists, and the importing file opens and rebuilds with
the source file absent. Load hydrates the blob back into the in-memory `Document`.
Load refuses import cycles (matched on resolved source path) and nesting deeper than
`MAX_UNIT_DEPTH`. Incremental writes insert/replace that one blob when the unit copy
changes.

**Unit identity (#1055):** `Document.units` is an `arena::Arena` keyed by `UnitKey`, which
is what an instance's `unit`, `Action::SyncUnit`/`SetUnitLink`/`AddUnitInstance`, the
staleness set, and the source watcher hold. Importing the same source again still shares the
existing embedded copy (matched on source), and deleting the last instance still leaves that
copy in place — only a **refused** import removes the trial copy it just made. A script names
a unit by its **ordinal** among the live ones.

**Instance identity (#1055):** `Document.unit_instances` is an `arena::Arena`, and a
placement is named by a `UnitInstanceKey` — `SceneElement::UnitInstance`,
`HierarchyNode::UnitInstance`/`UnitChild`, `JointRef::UnitInstance`,
`BodySource::UnitInstance`/`UnitCut`, `FaceId::UnitFace`, `ParameterSource::UnitEdgeLength`,
`ProjectionSource::UnitEdge`, and a move op's `instance_targets`. Deleting a placement
removes it, and the derived body goes with it on the next `sync_unit_bodies` pass. The
embedded copies in `Document.units` stay index-addressed. A script names an instance by its
**ordinal** among the live ones, resolved to a key at the script boundary.

**Evaluation (#722, `src/units.rs`):** an instance evaluates by rebuilding the embedded
document with its overrides applied and meshing its live bodies, memoized by **(unit,
override set)** under a fingerprint of `Document.units` alone — identical instances share
one evaluation, and the importing document's own edits never re-evaluate. Placement
expressions (`tx`/`ty`/`tz`, axis+angle) evaluate in the importing document; the viewport
draws each instance's placed meshes. A unit that fails to rebuild reports per-instance on
`DocumentHealth::unit_instances` (and builds what it can) instead of breaking the document.

**Snappable and referenceable (#724):** each live instance materializes as a **derived
body** (`BodySource::UnitInstance`, kept in sync by `units::sync_unit_bodies` on the
every-mutation seam; its mesh is the placed evaluation, and `document_mesh_fingerprint`
includes units so override/placement edits invalidate it). Everything body-shaped then
works unchanged: Move's snap-point pickers, `BodyVertex`/`BodyEdge`/`BodyFace` selection,
face pickers, STL/STEP export. The body renders in its own warmer fill (`UNIT_SOLID_FILL`)
so unit geometry visibly isn't the document's own; whole-body viewport clicks select the
**instance** row, and the body has no pane row of its own. Dimensions: an edge picked on a
unit body upgrades to `ParameterSource::UnitEdgeLength { instance, face, edge }` — an
**analytic** identity resolved against the retained rebuilt embedded document
(`UnitEvaluation::document`) — so unlike quantized keys it re-resolves and the dimension
follows override changes; unit geometry without an analytic face keeps the quantized key
(STL parity).

**Qualified references (#729):** every value input accepts `instance.param` — a named
unit instance's parameter, resolved to the instance's override where set, else the
unit's own expression **re-qualified into the instance's namespace** (so `internal =
width * 2` inside the unit reads that instance's width). Backticks wrap a **single name
segment**, never a whole reference: `` `left bracket`.width `` (either segment may be
backticked); this is the one spelling accepted everywhere. One level deep by design —
nested units' internals are not reachable (their values are already folded into the
nested unit's evaluation). Unknown instance or parameter fails like any unknown variable,
naming the full `a.b`; cycles threaded through qualified bindings are refused by the
evaluator's visiting stack. Implementation: `value::document_parameter_bindings` flattens
qualified names into the one `&[(name, expression)]` table every evaluator already uses;
the tokenizer's `qualified_identifier_at` lexes `segment(.segment)?` with backticks.

**Scripting (#736):** everything above is scriptable: `bearcad.import_unit{ path, link,
name }`, `add_unit_instance{ unit, name }`, `set_unit_parameter{ instance, name,
expression }` (omit the value to clear), `unit_link(unit, mode)`,
`sync_unit(unit)` (index or `{ unit = n }`), `derive_parameter kind="unit_edge_length"`,
and `select{ kind = "unit_instance", index }` / select-by-instance-name. Session-command
export writes each as its replayable call. A unit's *children* are deliberately not
selectable kinds — they carry no scene identity (#723).

**Moving instances & nesting (#735):** Move accepts a unit instance like a plane: the
click gathers the **instance** (`MoveOperation::instance_targets`), and the op composes
onto the instance's placement at evaluation — the instance itself moves, no output
bodies, nothing consumed; snap and free translation both apply (a snap start point on
the moved instance resolves against the *unmoved* placement via a re-entrancy guard).
**Nesting:** A may import C; B importing A builds C's geometry through A's evaluation
(the per-fingerprint evaluation cache handles the nested re-entry), C reads as **one
opaque row** inside A's expanded contents at any depth, `foo.bar` never reaches a nested
unit's internals (#729), and the depth cap is `MAX_UNIT_DEPTH` at load/import (#719).
**Scaling:** instances will be scalable later; `UnitPlacement` is all-`serde(default)`,
so a `scale` field lands without a format change.

**Instance Context pane (#734):** selecting an instance shows, under the shared Name row
(renames drive #731): **Link** (Dynamic/Static selectable, `Action::SetUnitLink`,
`bearcad.unit_link(i, mode)`), **Source** (file name + library/relative tag, an amber dot
plus an **Update** button when stale, #732), and **Placement**/**Rotation** value rows
(the Move tool moves instances, #735 — the pane shows the numbers). Controls and values
only; each row's explanation is help-mode text keyed on its label.

**Syncing (#732) — the rules:** a **dynamic** link picks up changes to A; a **static**
link doesn't, but updates on demand (the instance row's right-click → *Update from source
file*, `Action::SyncUnit`, `bearcad.sync_unit(i)`). Syncing **replaces the embedded
copy** — B stays self-contained and never depends on A being present; all instances share
the copy, so one sync updates every instance at once. Decisions written down:
- **When dynamic syncs happen:** on opening B (before the first frame; the document comes
  up dirty since disk still holds the old copy), and while B is open, **when A is saved**
  (#733 — the honest boundary for a file link; unsaved edits in A are never visible).
  Mechanisms: a debounced half-second source poll (`units::UnitSourceWatcher` — a change
  must sit quiet for one full poll before syncing, so an editor's temp-write-then-rename
  and rapid rewrite bursts collapse into one rebuild and a half-written file is never
  read), plus the **save-ping** channel for sources open in another BearCAD instance:
  every successful save rewrites a stamp file in the config directory
  (`units::write_save_ping`), and other instances stat it each tick and sync stale
  dynamic units immediately (a completed save needs no quiet period). B's own save pings
  too but changes none of its units' hashes, so it never self-triggers. No per-sync
  dialogs. Nanosecond mtimes back the staleness check so same-second saves still read as
  distinct.
- **Breaks apply anyway**: a sync that orphans a face, parameter, or body goes through,
  the damage reports via the existing document-health machinery, and one undo restores
  the previous embedded copy (each sync is its own undoable action).
- **A parameter renamed in A is not followed** (A's file carries no rename record):
  overrides and `instance.old` references to it simply stop resolving and report
  unhealthy, like any dead reference.
- **Staleness is visible**: `DocumentHealth::stale_units` (mtime check first, content
  hash as the authority) puts an amber dot on the instance row with the update hint. A
  missing source is *not* stale — the embedded copy is then the truth, and a sync against
  it refuses with a clear error.

**Instance rename (#731):** renaming an instance (the row, the Context pane — one
`CommitElementName` action) rewrites every `old.param` reference across all expression
holders (`propagate_instance_rename` → `substitute_name_everywhere`: parameters, sketch
dimensions/constraints, extrusion depths, Move/Repeat fields, text sizes, unit
placements and overrides), spelling the new name backticked where needed; snapshot undo
restores name + references as one step. A name another live instance uses is refused.
Renaming a parameter **inside A** is a sync concern (#732), not a rename concern.

**Completion (#730):** the same autocomplete every value input already has covers
qualified names: an instance name completes like a parameter (spelled backticked when it
isn't a plain identifier — including while an unterminated `` ` `` is being typed), and a
`instance.` prefix offers that instance's parameters with **primary ones first**. The
token scanner (`qualified_token_at_cursor`) extends across the prefix; `10.` still
completes nothing.

**Instance parameters in the pane (#728/#1176):** selecting a unit instance puts **its**
parameters at the top of the Parameters pane, headed by the instance name: the unit's
primary parameters first, secondary ones behind an "Internals" eye toggle (off by
default, ephemeral), the document's own parameters unmistakably below a separator. An
edit writes that instance's `parameter_overrides` (`Action::SetUnitParameterOverride`,
`bearcad.set_unit_parameter{ instance =, name =, value = }`; omitting `value` clears) — never
the source file, never other instances. Overrides are clamp-and-snapped to the unit
parameter's min/max/step; with both min and max set a **slider** sits on the row below,
spanning the name and value columns (same as this document's own parameters). The
importer cannot edit the unit's options (min/max/step/private). Overridden values render
gold with a ✕ back to the unit's own value; help-mode text is keyed on "Unit
parameters"/"Unit parameter"/"Override"/"Internals"/"Parameter slider". Instances are also
findable/selectable by name.

**Cut and combine (#726):** a unit builds a real kernel solid
(`occt_unit_instance_shape`: the inner rebuilt document's bodies' shapes fused and
placed), so it participates in body operations. A cutting extrude on a unit face lands on
`BodySource::UnitCut { instance, cut }` — the importing document's **own** output body;
the unit is never mutated (merge into a unit is refused → new body, with a status saying
why). Combine takes unit bodies on either side through the ordinary boolean path.
**Design decision:** a consumed unit body is never used up — it may feed several
operations at once (the consumed-body validation exempts it); consumption only sets its
`shadow` flag (ghosted-in-viewport presentation), recomputed by `sync_unit_bodies` every
pass so deleting the consuming op un-ghosts it. A re-sync that replaces the embedded copy
re-runs these ops against the new geometry (they resolve through the evaluation), and
ops whose referenced geometry is gone report unhealthy through the existing machinery.

**Sketch on a unit face (#725):** `FaceId::UnitFace { instance, face }` hosts a sketch on
a unit's flat face — the inner analytic face resolved against the instance's rebuilt
embedded document and placed by its transform (frame `None` → the sketch reports
**invalid** with a reason, via `mark_orphaned_unit_face_sketches`). The Sketch tool picks
unit faces like any analytic face (`pick_sketch_face` enumerates `units::inner_face_ids`
placed polygons). Beginning the sketch projects the face's boundary in as associative
construction edges carrying `ProjectionSource::UnitEdge { instance, face, edge }` —
analytic, so `refresh_projections` re-projects them when overrides change and anything
constrained to them follows.

**Elements pane (#723):** an instance is **one selectable row** (`SceneElement::
UnitInstance`): rename, hide, select, and delete work like any element; deleting
removes the instance and the **embedded copy stays** (unit keys remain stable, and
re-importing the same source reuses it). The row's triangle expands the unit's contents
as display-only leaves (`HierarchyNode::UnitChild`) in the List. Read-only is enforced at
the scene-identity layer: unit contents map to **no `SceneElement`** — the single gate
every selection/visibility/mutation dispatch flows through — so nothing inside a unit is
addressable by a mutating action. The node graph shows one opaque node per instance; the
"Unit contents" pane-filter toggle (off by default) admits the content leaves.

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
  `bearcad.extrude`, `bearcad.constrain`, `bearcad.dimension`, `bearcad.add_parameter`, `bearcad.select`, …).
  All **GUI/UI manipulation** — simulated mouse/keyboard, camera, tools, panes, the command
  palette, and viewport drags — lives under the `bearcad.ui.*` sub-namespace
  (`bearcad.ui.move`, `bearcad.ui.click` (with an optional `{ shift = true }` for a
  Shift+click, #835), `bearcad.ui.key`, `bearcad.ui.type`,
  `bearcad.ui.orbit`, `bearcad.ui.pan`, `bearcad.ui.wheel`, `bearcad.ui.view`,
  `bearcad.ui.tool`, `bearcad.ui.pane`, `bearcad.ui.pane_rect`,
  `bearcad.ui.pane_scroll` / `bearcad.ui.scroll_pane` (#1619), `bearcad.ui.palette`,
  `bearcad.ui.wait`, `bearcad.ui.screenshot`, …). Examples and documentation should model
  with the top-level API and avoid `bearcad.ui.*` except where a UI interaction is the point.
- **Semantic gizmo manipulation (#114).** `bearcad.drag_vertex` and `bearcad.drag_line` take
  sketch-local (not viewport) coordinates, so they are top-level modeling calls. Besides the positional absolute forms, each has
  a table delta form that moves things like a mouse drag would without knowing coordinates:
  `bearcad.drag_vertex{ point = <point>, du?, dv? }` nudges a vertex from wherever it
  currently is, and `bearcad.drag_line{ line = <line>, du?, dv? }` translates a line. Both
  respect constraints — attempting to drag a fully constrained vertex/line raises a
  catchable Lua error, like the GUI refusing the drag.
- **Scriptable gizmos (#214).** Viewport gizmos — a tool's drag handle for its live value,
  each a single scalar — are enumerable and drivable from a script, so gizmo-driven tools are
  automatable/testable without a mouse. `bearcad.ui.gizmos()` returns the gizmos available in the
  current tool/creation state (`{ kind, name, value }` per handle; `kind` is `"push_pull"`,
  `"rotate"`, or `"offset"`; push/pull and offset in mm, rotate in degrees — #1657).
  `bearcad.ui.gizmo(name)` (#1879) returns that row or nil.
  `bearcad.ui.set_gizmo{ name, value }` sets the scalar; `bearcad.ui.drag_gizmo{ name, by }`
  nudges it by a delta. The value is applied the same way a drag does (the semantic path). Current coverage: the extrude
  push/pull depth (`"extrude"`), the chamfer/fillet amount (2D sketch-vertex and 3D body-edge,
  named `"chamfer"`/`"fillet"` by kind), the revolve sweep angle (`"revolve"`, degrees), the
  construction-plane offset (`"offset"`), and the Move tool's translation
  (`"move_x"`/`"move_y"`/`"move_z"`, mm). The Move values are exposed ahead of the viewport
  drag handles (#185/#215); the Move rotation gizmo went with the tool's rotation half (#663).
- `bearcad.ui.screenshot([path], [region])` captures the 3D viewport only by default (the
  view bear (the view-cube HUD) is suppressed for that frame). `region` is `true` or
  `"window"` for the entire window, or a pane name — `"context"`, `"elements"`,
  `"parameters"` — to capture that pane alone (#672). A pane capture is cropped to the
  pane's rect, cut off below its last control so the shot is the controls rather than an
  empty column, and fails (no PNG, a script error) if that pane is hidden. With no `path`,
  the image is written to `screenshot-bearcad.png`.
- Geometry-creation helpers are single calls that create the thing directly (no simulated
  mouse/keyboard) and enter a ground-plane sketch if none is open: `bearcad.rect{ width, height,
  x?, y?, name? }`, `bearcad.line{ length, angle?, x?, y?, name? }` (or explicit endpoints
  `bearcad.line{ x, y, x1, y1 }`), and `bearcad.circle{ r|radius|diameter, x?, y?, name? }`.
  A scripted line lands **unconstrained**, exactly like a click-drawn one; passing
  `dimension = "<expr>"` (or a number, or `true` for the as-drawn length) locks its length,
  the scripted equivalent of typing a length while drawing. Document Lua export (#1159)
  carries locked length expressions through `dimension =` on `bearcad.line{}`.
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
- `bearcad.plane{ offset?, from?, origin?, normal?, axis?, angle?, name? }` (#116/#465/#1876)
  declaratively adds a new construction plane. Offset along the normal of an existing one
  (`from`, a construction-plane index — defaults to plane 0 / Ground) is the scripted
  equivalent of picking a plane in the viewport and typing an offset. `origin` + `normal`
  together sit the plane on a face. `axis = "x"|"y"|"z"|line` with `angle` (degrees around
  that axis) is the same as clicking an axis with the Plane tool — a world triad axis, a
  sketch-line ordinal, `{ line = i }`, or a line handle.
- **Invalid input fails loudly (#104/#109/#110/#112):** when a declarative modeling call's
  underlying action is rejected — degenerate input (zero-size rect/circle/line, zero-distance
  extrude), an extrude face that doesn't exist or isn't a closed loop, a chamfer/fillet vertex
  that doesn't join exactly two lines or whose corner is within ~1° of straight (§3.1), an
  out-of-range 3D edge, … — the call raises a Lua error (catchable with `pcall`) instead of
  silently succeeding with nothing created. The GUI surfaces the same rejection message
  through the status bar. Options tables also **reject unrecognized keys (#403)** — a typo
  like `combine{ kind = … }` (the key is `op`) or `repeat_bodies{ gap = … }` errors
  immediately, naming the accepted keys, instead of being ignored and failing confusingly
  downstream. Repeat uses `spacing`; revolve helix uses `pitch`.
- **Read-back / introspection (#107):** the API is not write-only — pure read getters (never
  recorded as instructions) let scripts assert what they built: `bearcad.count(kind)` /
  `bearcad.get{ kind, index }` over lines, circles, sketches, constraints, construction
  planes, extrusions, bodies, and parameters (`count` also takes `drawing`, `sketch_text`,
  and `image`); `bearcad.body_stats(i)` (mesh
  volume/triangles/bbox); `bearcad.status()`; `bearcad.version()` (Help → About
  identity: release tag, or crate version + SHA); `bearcad.selection()`;
  `bearcad.parameter_value` / `bearcad.parameter_expression`; **`bearcad.ui.pickers()`** (#968) and
  **`bearcad.ui.picker(name)`** (#1879), the active tool's element pickers — per picker its `name`,
  whether it's `focused`, its `limit` (absent when unlimited), the element-kind names it
  `accepts`, and the `items` it holds. `picker(name)` returns that row or nil. This
  is the only way to tell an **accepted** pick from a **rejected** one: a body-set tool consumes
  the click either way, so `selection()` reads the same whether or not the pick landed. A test
  asserting a `PickRule` (#953) needs it. **`bearcad.ui.hovered()`** (#968) reports what the
  viewport is hover-highlighting as `{ kind, index }` — the pick a click would take — or nil;
  a hovered region or curve with no element of its own reports nil, which is itself assertable.
  A hovered **face** also carries a **`label`** (#987), because every face reports kind `face`
  and index `0`: without a name for the face itself, a hover flickering between a body's near
  face and the one hidden behind it read as *unchanged* from a script, which is how it went
  unnoticed. `face_label` for an analytic face, body-and-centroid for a mesh one.
  **`bearcad.ui.exploder()`** (#968) reports the Selection Exploder's fanned leaves as
  `{ kind, index }`, empty when it's closed — the crowd it is offering, which nothing else
  exposes, each with a `label` when it is a face (#988), for the same reason `hovered()` has one.
  Each leaf the current drill level shows as a loupe of its own also carries
  **`x`/`y`** (#986): where the fan put that loupe, in the viewport-local pixels
  `bearcad.ui.click` takes. Without it, *picking through the fan* was unreachable from a
  script — the leaves were readable but there was no way to say where to aim — so the whole
  click-a-loupe path went untested. A leaf currently inside a group loupe has no spot of its
  own and carries neither.
  Write side: **`bearcad.ui.picker_focus(name)`** arms a picker, the scripted equivalent of
  clicking it in the pane.
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
- **Point-level selection (#68):** `bearcad.select{ kind = "line", index, endpoint = "start"|"end" }`
  selects an individual vertex (a `ConstraintPoint`) rather than the whole element, so e.g.
  `bearcad.constrain("coincident", a:endpoint("end"), b:start())`
  can join two line endpoints (closing a polygon loop — including a rectangle's four corners)
  purely from a script — a line's two points are `start`/`end`, i.e. `(x0,y0)`/`(x1,y1)`.
  A table with no `endpoint` still resolves to the whole element as before; pass an explicit
  `point = true` to target a point that has no such field (e.g. a circle's center).
  A line handle names the same points as `line:start()` / `line:endpoint("end")`.
- **Positioning dimensions (#809):** `bearcad.dimension{ kind, …, value }` takes the two-thing targets the
  Dimension tool picks interactively, not just `line` (length) and `circle` (diameter):
  `{ kind = "point_line", point = <point>, line = <line> }` (perpendicular distance from a
  point to an edge — how holes are positioned), `{ kind = "point_point", anchor, mover }`,
  and `{ kind = "line_line", a, b }` (spacing between parallel lines). `point` takes the same
  tables point-level selection does (line endpoint, circle centre, face vertex, text anchor,
  sketch origin `{ kind = "origin" }`);
  `line` takes a sketch line, an origin `axis`, or a `face` edge. The side/direction each
  dimension is measured on is captured from the current geometry
  (`constraints::finalize_distance_target`), exactly as for an interactive pick.
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
- `mcmaster [part]` — show the McMaster-Carr catalog in a window of its own, printing each
  CAD file it downloads (#1022). The app runs itself under this to host the web view in a
  second process; it is a real subcommand rather than a private flag so the window can also
  be opened, and its behaviour checked, straight from a terminal.
- `testplan` — print a `[ ]` checklist of tools, tool variants, features, tutorials,
  and other actions for exhaustive manual or AI testing. Tutorials come from the
  registry in `src/tutorial.rs` (`TUTORIALS`). Extra one-off items are added in
  `src/testplan.rs` (`CUSTOM_ITEMS`).
- `install-cli` / `uninstall-cli` — symlink the running executable onto PATH as `bearcad`
  (default `/usr/local/bin/bearcad`), and remove it; also register / unregister the
  `.bearcad` file association so double-click opens BearCAD (#1285). Because macOS
  drag-to-Applications installs run no code, the PATH half is how the bundled binary
  becomes usable from a terminal; it is also exposed as **Help → Install "bearcad"
  Command in PATH**. Refuses to clobber a non-symlink at the target, and reports a sudo
  hint on permission errors. On Windows only the file-association half applies.

The command set is expected to **grow over time** toward full GUI parity. New GUI actions
should be added to the shared action layer so they become available headlessly by default.

- `--timeout <seconds>` — force-exit (non-zero) if the app hasn't closed on its own within
  the given duration, so an unattended/CI launch can't hang forever (#61).
- `--rebuild` — discard persisted tessellation (`geometry_cache`) after open and rebuild
  geometry (SPEC §4.4).
- `--headless` / `--no-headless` — run without a window: the egui pass loop and the 3D
  viewport render into an offscreen wgpu target, and the run ends when the script does
  (or errors). Scripts default to headless, so they work on displays-less machines and
  in CI; `--no-headless` opens a real window to watch a script run.

**Launch diagnostics (#978).** A window that comes up blank — title bar and menu present,
nothing drawn — has several possible causes and no way to tell them apart from the outside.
`src/diag.rs` splits the reporting in two so an ordinary run stays silent:

- **Always on stderr**, because they are wrong however the app was started: no wgpu render
  state, the GPU viewport failing to install, a scene built that the GPU could not paint, and —
  from a watchdog thread at 8s — **no frame drawn**, or **only the launch frames drawn**. Those
  last two are the blank-window discriminators. No frames at all means the app never got as far
  as drawing. A handful and then nothing means it drew and stopped being asked to, which is what
  the deferred macOS maximize used to cause. Silence from the watchdog means drawing continued,
  which points at presentation rather than scheduling.
- **`BEARCAD_LOG=1`** adds the startup trace: the requested window size and maximize mode, the
  GPU backend and adapter, the maximize command when it goes, and one line per early frame with
  the size it was built at. **`BEARCAD_GPU_LOG=1`** additionally opens wgpu/eframe/winit's own
  logging (§11.3a) from `Warn` down to `Debug`, which is where surface and swapchain faults are
  reported.

egui is **reactive** — it draws on input and on request, not continuously — which is why the
deferred macOS launch-maximize (§11) requests repaints across its whole sequence and a moment
past it. Without that the countdown can stall before it ever sends the command, and the resize
the command causes can land with no repaint behind it; either way the window ends up correctly
sized and never drawn.

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
- **STL exports are watertight (#1286):** the whole-document mesh used for STL (and other
  faceted formats) is accepted only when every undirected edge is shared by exactly two
  triangles. If the OCCT union tessellation is open or has degenerate poles (e.g. a BREP
  sphere), export falls back to each body's own solid mesh — hand-rolled primitives and
  already-validated extrusion meshes — so the file a slicer reads is a closed manifold.

### 9.3 Instruction scripts (for automation & testing)

**Directive:** The app should be fully scriptable. One must be able to run the app with a set of instructions (from a file) and the app must open and run each of the instructions. One must be able to export a screenshot of how the app looks as one of the instructions. This can then be leveraged for testing.

The application must be fully scriptable via a file containing a sequence of instructions.

- Invocation: `bearcad <script-file>` or `bearcad --script <script-file>` (or equivalent).
  A script run defaults to **headless** — no window, offscreen rendering, the process
  exits when the script finishes (an uncaught error exits non-zero) — see `--headless`/
  `--no-headless` above.
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
  instruction is executed, suitable for visual regression testing. A capture rides the next
  painted frame, and a frame the window server skips takes the request with it — on macOS
  every frame is skipped while the window is fully covered, minimized, or the display is
  asleep. The runner therefore re-sends the capture every few frames and, after a dozen
  tries, fails the script with that reason instead of hanging until `--timeout` (#872).
- Scripts shall support at minimum:
  - Core actions (new, open, save, clear, tool selection, rectangle creation flow including
    the click-to-place, mouse-move preview, dimension typing, tab, enter steps, etc.).
  - Camera/view control.
  - File I/O and export.
  - The screenshot instruction above.
  - Simple sequencing / waits if needed for UI settling or animations.
- **Exception — the AI is not scriptable.** No script may reach the AI pane: no MCP server,
  no skill install. A `.lua` file arrives from anywhere and gets run without being read, and
  none of them gets to open the user's document to a network service. Everything AI is done
  by hand, in the pane.
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
directory named by `$BEARCAD_SCREENSHOT_OUT`. `scripts/gen-doc-screenshots.sh` runs them into
`docs-site/static/img/screenshots/` (git-ignored build artifacts), failing if any expected PNG is
missing. Selection/sharding (#1297): `scripts/select-doc-screenshots.sh` picks scenes
(`full` / `missing` / `affected`); the harness accepts `BEARCAD_SCREENSHOT_MODE`,
`BEARCAD_SCREENSHOT_SHARD=N/M`, `BEARCAD_SCREENSHOT_ONLY`, and `BEARCAD_SKIP_BUILD`.
Website CI (`.github/workflows/docs.yml`) never rebuilds the app or recaptures shots on a
normal push: it restores the Actions golden cache and fetches the live site's files (via
`img/screenshots/manifest.txt`; Docusaurus does not copy dotfiles, so these are not
`.manifest`). Only the nightly schedule — or a `workflow_dispatch` with "generate
screenshots" — rebuilds them, and nightly skips when the repo SHA still matches the
deployed `screenshots-commit.txt`. A missing screenshot never fails the site build
(#1389, #1446). Goldens are never committed. This reuses §9.3's determinism guarantees
(fixed view, no animation waits).

**Annotated pane pictures (#672).** Every tool's documentation page shows its Context pane —
one shot per mode where the tool has modes — captured with **help mode** on, so each control
is explained beside it. The shots come from `docs-site/screenshots/pane-*.lua`, which turn help
mode on with `bearcad.ui.help(true)` and capture `bearcad.ui.screenshot(path, "context")`; a
scene that yields several shots writes them as `<script-name>-<variant>.png`. A PNG belongs
to the **longest** scene name its file stem matches, which is how the harness knows what to
clear before a scene's turn and what to count as its output: `chamfer-*` would otherwise
also match `chamfer-sketch.lua`'s shot, and `chamfer.lua` — running later — would delete a
picture nothing regenerates. Because the
explanations are the app's own help text, a page cannot drift from the pane it documents.

**Comparison series.** A scene may also shoot the *same* geometry more than once to show what
one control changes: `docs-site/screenshots/snap-pairs.lua` lands one slab on one plate three
times from one pinned camera — the A pair alone, then A + B, then A + B + C — so the Move
page can show what each snap pair decides side by side.

Framing is part of that determinism: `gen-doc-screenshots.sh` pins `BEARCAD_WINDOW` (default
`1600x900`, overridable) instead of letting the window maximize, and sizes the `xvfb` screen to
fit it. Otherwise the shot depends on the machine — a desktop maximizes to its whole (often
retina) display while CI, which has no window manager, keeps the 960x640 default — and anything
sized in points (loupes, toolbars, labels) covers a wildly different share of the viewport, so
the deployed image is framed nothing like the one the author reviewed. With the size pinned, a
retina machine renders the same composition at 2x, just sharper.

---

## 10. Geometry kernel integration (OCCT)

- Integrate OCCT via Rust FFI through a **hand-written thin C++ shim** exposing only the
  operations BearCAD needs (sketch profiles, prism/revol, boolean, fillet/chamfer, shell,
  sweep/loft, STEP/mesh I/O, tessellation). All `unsafe`/FFI is isolated behind a safe Rust
  `kernel` module (`src/kernel/`, shim in `cpp/`). The shim presents a flat `extern "C"` C
  ABI (no C++ types cross the boundary), so no `bindgen` is required.
- OCCT is **statically linked** into every native build (the former `occt` Cargo feature is
  gone — the kernel is unconditional, todoer #471). Static linking is permitted under
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
- **Booleans retry on a tangential coincidence (#1033):** OCCT's default tolerance can miss
  the intersection between solids that touch *exactly* — a sphere snapped onto a box corner
  has its surface pass through that corner, and OCCT then reports no intersection at all and
  silently hands back the unmodified A side, so the cut appears to do nothing. The shim
  therefore checks each completed boolean for a **silent no-op** (a cut that removed nothing,
  an intersection that kept nothing, a union smaller than its larger operand) and, when the
  operands' bounding boxes actually overlap, retries with a widening `SetFuzzyValue`. The
  escalation is relative to the operands' combined bounding-box diagonal and tops out at
  `1e-7` of it — for a 200 mm part, points 0.02 µm apart. Solids that genuinely don't meet
  fall through every attempt and return the input whole rather than an error.
- **CI/release wiring** (#89, single build mode since todoer #470/#471): every build ships the
  kernel — `cargo build` needs a built OCCT, and `scripts/build-occt.sh`/`.ps1` fetch a
  checksum-verified prebuilt keyed to the pinned submodule commit and script hash from the
  rolling `occt-prebuilt` release, falling back to a source build (todoer #469). CI's `occt`
  (Linux) and `windows-occt` (Windows/MSVC) jobs build OCCT once (cached on the pinned
  submodule + build-script hash) and run the full test suite plus the smoke/example/interaction
  checks against the kernel build. **All release binaries — macOS, Linux, and Windows — ship
  with the kernel** (#96, todoer #468).
- **Migration status**: extrusions (prism/loft), multi-body union, solid booleans (incl.
  extrude cut), 3D edge fillet/chamfer, and STEP I/O run on OCCT, each with a hand-rolled
  fallback retained for the per-frame ghost previews, for cases OCCT doesn't yet cover
  (multi-face profiles, imported meshes), and as the graceful path when a kernel op fails on
  degenerate geometry. The former lean (`--no-default-features`) build mode is gone (todoer
  #470/#471).

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
  so inputs line up down the whole pane; a label wider than the column **wraps** onto a
  second line rather than pushing its input out of alignment (#632, e.g. "Parameter name"). **Checkbox toggles use the same two columns** (#588,
  `context::checkbox_row`): the label in the left column, the box in the right with any shortcut
  hint to the **right of the box** (#597), and **clicking either the label or the box** toggles it. Applies to the draw-tool toggles
  (Construction, Snapping, Curve, Tangent), Extrude/Revolve **Symmetric**, and Slice **Infinite cut**.
- **No help text:** the context pane carries **controls and values only** — never
  explanatory prose, "pick an X next" prompts, or status/warning lines about the current
  configuration. Labels, icons, and hover tooltips do that work; a tool that can't commit says
  so by leaving its commit button disabled, not by adding a sentence.
- **Every tool has a title (#608):** whenever a tool is active, its context section is headed by
  the tool's **title**, drawn once at the very top of the pane — above the tool's pickers and
  controls — so every tool reads consistently (`ContextPaneContent::tool_title`,
  `context::tool_context_title`). The title reflects re-editing a committed operation ("Edit …").
  This holds for **all** tools without exception; only the Select tool (which shows selection
  properties) and the drawing workbench (its own section headings) have no tool title.
- **Primary button (#586/#598):** a tool that completes an action shows a single **blue, no-text**
  commit button (a checkmark icon) that **fills the right column** of the two-column layout
  (`context::primary_button`). It also fires on **Enter** (when enabled and no field has the
  keyboard). It is **enabled only when the action is ready** — all inputs picked and valid — and
  otherwise stays visible but **disabled**, so the tool always shows where "go" is. Present on
  Extrude, Sweep, Loft, Revolve, Combine, Move, Mirror, Repeat, and Slice, each gated on that
  tool's readiness (e.g. Revolve needs a profile + axis; Loft needs ≥ 2 sections). A
  **text-labeled variant** (`context::primary_text_button`, #629) carries the same blue
  fill and Enter behavior for actions whose name should read without hovering — the
  Dimension tool's **Derive parameter**.
- **The legacy row-list picker is gone (#955):** `EdgePickerControl` — a heading, an icon and a
  `Vec<String>` of rows, kept "until each tool migrates" — is retired. Its two remaining users,
  the Loft **Sections** set and the 3D Chamfer/Fillet **Edges** set, are ordinary
  `ToolPickerView`s over `SceneElement::SketchFace` and `SceneElement::ExtrusionEdge`. Row
  wording moved into `names::scene_element_label`, so a picker row and every other place that
  names the same thing agree by construction.
- **Every tool input is an element picker (#534):** anything the user can click to feed a
  running tool — a plane, line, axis, face, body, sketch, cutter, mirror line, etc., primary
  or secondary — is represented in the context pane by an **element picker**
  (`element_picker::show` / a `ToolPickerView`), never a bare text label. Each picker is
  scoped to its allowed element kinds and a pick limit (Single/Infinite), reflects the picked
  set, and lets the user re-pick/remove from the pane. This holds for **all** tools, without
  exception, and "is a picker" means real `ElementPicker` state — not the combo-box chrome over
  a list of label strings, which is what a dozen of these inputs used to be (#955). Every one is
  registered in `tool_pickers` whatever *where* it draws (§11.4a); there are no exceptions.
  A tool whose committed form keeps only *derived* geometry — the Construction Plane tool's
  frame, say — records what was clicked alongside it (`anchor_elements`), so its picker holds
  elements like every other. Model new pickers on the in-sketch Offset tool's "Entities" picker
  and the Move/Repeat "Bodies" pickers.
- A standard **application menu bar** (File / Edit / View / Help) sits above the
  workspace. Menu items dispatch the shared action layer (§8) so menu, toolbar,
  shortcuts, and scripting stay in sync. The **View** menu contains a **Panes**
  submenu that shows/hides each available pane via a checkbox — Elements, Context,
  Parameters, **Tutorials** (#1291), and View Bear. (The menu bar is drawn in-window
  rather than as a native OS menu so it appears in screenshot regression tests, §9.3,
  and stays consistent across platforms.)
- **Window title & unsaved changes (#522):** the OS window title reads
  `<file name> — BearCAD` (or `Untitled — BearCAD` before the first save), prefixed with
  `*` whenever the document has unsaved changes. Dirty tracking (`AppState::dirty`) compares
  the live document to a snapshot of the last saved/opened/new state, so undoing back to the
  saved state clears the `*` again. Quitting with unsaved changes opens a **Save / Don't
  Save / Cancel** prompt (the one sanctioned modal — a quit confirmation, not workflow UI);
  it is suppressed for script-driven and auto-exit runs (tests, `--exit`, screenshots) and in
  debug builds (`cargo run`) so development quits aren't nagged, and can be forced on/off with
  the `BEARCAD_QUIT_PROMPT` / `BEARCAD_NO_QUIT_PROMPT` env vars. Native only.
- **DEV menu (#627), debug builds only** (`cargo run`; not user-facing, not on the
  docs-site): a native **DEV** menu with **Report issue…**, opening its own OS window
  (immediate viewport) with a focused description textarea and
  two default-checked checkboxes — *Include a screenshot of the current window* and
  *Include the document JSON*. **Submit** (or **Cmd/Ctrl+Enter** from inside the
  textbox, #634) files the issue into the repo's local todoer db
  via the `todoer` CLI (`add <first line as title> -b <full text> --attachment …`); the
  screenshot rides the main window's `ViewportCommand::Screenshot` round-trip
  (`PendingIssueReport` → `process_screenshots`), the JSON is `storage::to_json_bytes`,
  both written to temp files. The window stays open after a submit — cleared, re-focused,
  showing todoer's confirmation — so reports can be filed back-to-back.
- **Import/Export menus & toolbar (#352/#1175):** the File menu groups the model interchange under
  an **Import** submenu (BearCAD File, McMaster-Carr, STL, STEP, Image, Lua Script — that order)
  and an **Export** submenu (STL, 3MF, STEP, Lua Script), and the model workbench toolbar **Import** /
  **Export** icon popups use the same items, order, labels, and dividers. (The drawing workbench's
  own Export icon, #348, exports SVG/PDF instead.)
- **McMaster-Carr catalog import (#1022):** **File → Import → McMaster-Carr…** (the Import
  toolbar button's popup, and the command palette) opens **their own site in a window** and
  catches the CAD download on its way out, importing it straight into the document. Pick the
  part the way you already would, choose STEP, and the body lands in the model instead of in
  Downloads.
  **Why the site and not their API:** McMaster's Product Information API is gated behind a
  signed agreement and a client certificate they issue per account, so it would work for
  almost nobody; scraping is against their terms and bot-blocked. Showing the site and
  catching what the user themselves downloaded needs no account and asks nothing of McMaster
  a browser doesn't.
  **The window is a second process (`bearcad mcmaster [part]`)** — this same executable under
  a subcommand (`script::CliOutcome::McMaster` → `mcmaster::run_catalog_process`), so there is
  no second binary to build, sign or package. It owns a `tao` event loop and a `wry` web view
  filling its window: a **real OS window** with real z-order, movable to another monitor. On
  macOS it is an **Accessory** helper (#1023) — no second Dock tile, not a peer app — same
  multi-process shape as a browser content process. **⌘`** / **⌘⇧`** cycle every OS window
  the app owns — main, detached tabs, Report issue, and the catalog helper (#1477). System
  window cycling is per-process, so each side activates the other by PID and the parent
  walks the full list. Hosting the view *inside* the app's window
  instead (`build_as_child`) was tried and rejected — the native view composites above the
  wgpu canvas and so floats over every egui window regardless of stacking, and on Linux wry
  **panics** without `gtk::init` and a GTK loop pumped alongside, which eframe/winit never does.
  `tao` initializes GTK itself, which is what makes Linux work here at all.
  **The wire between them** is the child's stdout: one line per caught file,
  `part<TAB>path<TAB>url` (`CaughtDownload::to_line`/`from_line`) — prefixed so the platform's
  own chatter on that stream isn't mistaken for a report. `CatalogSession` spawns the child,
  reads those lines on a thread, and notices stdout closing as the user closing the window;
  its `Drop` kills the child, so quitting the app never leaves a stray browser behind.
  **How a part lands:** the child's download-started handler redirects the file into the app's
  scratch directory — never the user's Downloads, never under a name the URL chose
  (`download_file_name` sanitizes it, so a catch can't escape the directory) — and
  `AppState::import_catalog_part` imports it by what it is (`CadFormat`: STEP or STL) through
  the same `import_step_bytes`/`import_stl_bytes` a File → Import goes through. A catalog part
  is an **ordinary body** afterwards, and the scratch copy is deleted either way.
  **Naming:** their CAD files are `<part>_<description>` and percent-encoded in the URL
  (`3042T88_Clamping%20U-Bolt.STEP`), so the name is decoded *before* it is sanitized — else
  the `%20` survives as a literal `20` — and read apart into **Clamping U-Bolt (3042T88)**,
  the way a part reads in the Elements pane. A bare part number, or the number from anywhere
  along the URL, are the fallbacks.
  **Through the kernel (#1023):** the caught file goes to `AppState::import_step_file`, the
  same reader File → Import → STEP… uses, **not** `import_step_bytes` — whose kernel arm is
  wasm-only, so on native it falls to the faceted-subset parser, and McMaster's CAD is
  SolidWorks BREP (`ADVANCED_FACE`, no `FACE_SURFACE`). That mismatch rejected every real
  catalog part with "no FACE_SURFACE entities found". One shared function now, because two
  copies is how it happened.
  **Not every download is a model (#1023):** their servers answer a CAD request that isn't
  ready with a short HTML fragment, saved under the `.STEP` name the URL promised.
  `mcmaster::caught_content` judges the first bytes, so that reports itself as a web page
  rather than as a broken STEP file. By content only — judging by *size* was tried and
  rejects a legitimately small STL, trading a broken import for a better message.
  **Where it landed (#1023):** macOS never tells us. wry's `download_did_finish` passes
  `None` for the path, hardcoded, so the completed handler can only know the destination the
  *started* handler chose — which is ours anyway. It is remembered per URL (several parts can
  download at once) and checked to exist before it is reported. Without this the file arrived
  on disk and was never imported, which is exactly what it looked like from outside: a click
  that did nothing.
  **The window stays McMaster's:** a navigation off their host is refused and handed to the
  user's real browser (`is_mcmaster_url`, host-matched so a lookalike domain can't pass),
  rather than quietly becoming a general-purpose browser inside the CAD app.
  **`window.open` means two things on their site (#1023),** and a web view that handles neither
  drops the click silently — which is what made a download appear to do nothing at all. A
  **download** opened in a new window is loaded in *this* window instead, because these are the
  handlers that catch it; any other popup (their help pages) goes to the real browser, since
  taking the catalog view for it would lose the page being read. `looks_like_download` tells
  them apart by CAD extension or a download-ish path, and **both branches log which they took**
  — a wrong guess is then visible and fixable, where the failure it replaces was silent.
  **Opening it at something (#1022):** the window takes one box's worth of intent —
  `mcmaster::catalog_url_for` sends a phrase to their **search results**
  (`/products/?q=…`, percent-encoded so a `1/4"-20` callout survives), something already
  shaped like a part number straight to its **product page**, and nothing at all to their
  front page. A part number is just a very specific way of saying what you're after, so it
  is one box rather than two. The command palette's **Search McMaster-Carr** is what fills
  it: the first palette command to take an argument (§11.2).
  Native only — on the web build the browser is already the browser. Scriptable:
  `bearcad.ui.mcmaster("show"|"hide"|"toggle", query?)`, `bearcad.ui.report_issue("show"|"hide"|"toggle")`,
  `bearcad.ui.windows()` (cycle order), `bearcad.ui.focused_window()`.
- **BearCAD-file import (#721):** **File → Import → BearCAD File…** (also the command palette's
  "Import BearCAD File" and `bearcad.import_unit{ path =, link = "dynamic"|"static", name = }`)
  imports another `.bearcad` document as a unit (§7.4): reads the file, refuses a cycle, embeds
  the copy (reused when the same resolved source is already imported), and adds a first instance
  named after the file stem (sanitized to an identifier, uniquified with 2, 3, …). Source
  classification: under the library directory (§11.z) → `Library(path)`; else a path relative to
  the importing document's own file — importing into a never-saved document is refused with a
  "save first" status. Link mode defaults to dynamic (#732 syncs it).
- **STL export from the GUI:** **File → Export → STL…** exports all bodies (via a save
  dialog); right-clicking a **body** row in the Elements pane exports just that body. Both
  mirror the scriptable `bearcad.export_stl` (§8, §9.2).
- **Component export (#521):** right-clicking a **component/folder** row in the Elements pane
  offers **Export STL…** and **Export STEP…**, which write **every body inside that component
  and its nested components** to one file (default-named after the component). Bodies are
  gathered by `AppState::component_body_indices` (a body's owning component resolved through
  its producing op, then the parent chain), then their solid meshes are concatenated — the
  same non-boolean combine the whole-document faceted fallback uses. A single-body component's
  STEP routes through the per-body path so kernel builds still write real BREP; an empty
  component reports an error rather than writing an empty file.
- **STL import (#70):** **File → Import STL…** (open dialog) reads an STL file — ASCII or
  binary, auto-detected by exact byte-length match against the binary format's
  header+triangle-count framing — and adds it as a new **Body** with no source feature (no
  sketch/extrusion to nest under, so it nests directly under the Elements pane's Document
  root (#87), named after the file). Scriptable via `bearcad.import_stl(path)`. The mesh
  lands in `Document::imported_meshes`, an `arena::Arena` (#1055), and the body names it by
  `BodySource::Imported(ImportedMeshKey)`. The mesh is
  stored and rendered as-is (no auto-centering/scaling); it participates in STL/STEP export,
  visibility, renaming, and deletion exactly like any other body, but — since it has no
  sketch/distance parameters — can't be edited or merged into by a further extrude the way
  an extrusion-backed body can (#32).
- **STEP export/import (#65/#71):** **File → Export STEP…** / **Import STEP…** (and the
  per-body Elements-pane export). With the OCCT kernel (every build, §10),
  a single-body STEP export — including the whole-document export when the document holds
  exactly one live body (#106) — writes **real BREP** (planar *and* curved surfaces) straight
  from the body's OCCT solid via `STEPControl_Writer`, and import reads **real BREP incl.
  curved/NURBS surfaces** via `STEPControl_Reader`, tessellating the result into a new **Body**
  (nests under the Document root, named after the file) while **keeping the STEP bytes** on the
  import so later booleans/cuts still have a solid to work with (#1029) — triangles alone are
  not enough. Scriptable via `bearcad.import_step` / `bearcad.export_step`; import/export/open/save
  failures raise catchable Lua errors (#106).
  - **No-kernel fallback:** builds without OCCT (and the multi-body export path, plus any body
    whose geometry isn't kernel-representable) use the hand-rolled `step.rs` path — export
    writes a conformant AP203 `FACETED_BREP` with full product scaffolding (parenthesized
    complex context entity, `SHAPE_DEFINITION_REPRESENTATION` anchoring; OCCT and third-party
    readers can parse *and transfer* it, #106), and import reads only that same
    `POLY_LOOP`-bounded planar `FACE_SURFACE` subset. In this mode, STEP files using full BREP
    geometry (`ADVANCED_FACE` with curved/NURBS surfaces, as most CAD tools export) are
    rejected with a clear error rather than approximated. Imported bodies behave like STL
    imports (no analytic face/edge structure to sketch or edit against).
- **Export document as Lua (#1159):** **File → Export → Lua Script…** (also a command-palette
  entry) writes a deterministic Lua script that recreates the current document by walking the
  element graph — no `bearcad.ui` module, no session history. Prefer this over replaying UI
  steps: the script is the minimum declarative model (`bearcad.rect`/`line`/`extrude`/…).
  Interactive `--show-commands` recording remains for debugging; the Help **Export Session
  Commands…** UI was removed in favor of document export.

  **DEV → Verify Lua export…** (debug builds only): generates the Lua, runs it in a temporary
  document, and reports field-level diffs (stdout + status line) when the round-trip diverges.
- **Import document Lua (#1160):** **File → Import → Lua Script…** (toolbar Import popup,
  command palette, `bearcad.import_lua(path)` / `bearcad.import_lua{ path =, force = }`)
  runs a document export script against the live document. Importing into a non-blank document
  warns in the GUI; scripts refuse unless `force = true`.
- **Document JSON dialog:** **File → Document JSON…** (also a command-palette entry) opens a
  dialog holding the whole document serialized with the web build's JSON codec
  (`storage::to_json_bytes`). Copy the text into a bug report to share exact document state;
  paste a reported document in and **Load into document** to reproduce it. Works identically
  on desktop and web (no file dialogs involved).
- **A row's click targets (#964):** an element row selects from **either its name or its type
  icon** — both are sensed for clicks and fold into one row-level pointer state
  (`hierarchy::RowClick`), so a click, double-click (edit, where the row supports it), or hover
  lands the same whichever the pointer was over. Applies to element rows and component rows
  alike. The eye toggle and the collapse triangle keep their own actions.
- **Elements pane view modes (#34/#252):** two icon-toggle buttons next to the pane heading
  switch between **List** (the default flat, topologically-sorted view) and **Graph**. Long
  row labels **clip** to the pane width (ellipsis; full name on hover) instead of stretching
  the pane (#1575). The
  List order depends only on the **element graph** — the nesting tree plus the input
  dependency edges, so a consumer always follows its inputs — with independent nodes
  tiebroken by kind+index (#540). It never depends on **when** an element was created;
  `shape_order` is purely an undo/redo ledger, not an ordering input. The
  former **Tree** view is retired (#252): a strict tree can't represent an element with
  multiple inputs (a body that is both one op's output and another's input), which is the whole
  point of the graph model — so its button is gone and a script-set `Tree` mode renders as
  List. **Graph** is a **one-node-per-line** view (#1670) laid out the way `gitk` draws
  commits (`graph_lane_layout`). Rows run top to bottom in a depth-first walk of the hierarchy
  that holds a node back until every input it depends on is already placed, so a consumer is
  always below its inputs; the relationships are drawn beside them as mostly-vertical **lanes**
  with short diagonal legs into each row. A node with consumers reserves **one** lane for all
  of them, and a lane is reused the moment its last consumer is passed, so a straight chain
  (sketch → extrude → body) holds its lane rather than drifting right. **One** consumer
  carries straight on down that lane; **two or more** sit one lane to its right and leg into
  the trunk, so a shared trunk runs *beside* its children rather than through their icons
  (#1683). Each node rides the lane of its **nearest** input — riding a farther one's would
  make the near line detour out of its lane and back, crossing whatever runs between (#1684).
  If that preferred lane is already carrying another trunk through this row, the node
  packs **left** into a free column rather than stepping further right.
  The **first lane stays clear across a top-level element's row** — nothing feeds it and it
  sits under nothing, so a trunk that would cross it steps right instead, and indentation
  keeps meaning "sits under something". The synthetic **Document root is not shown** here
  (#1682): "everything hangs off the document" is not a relationship worth a row. The graph
  therefore spills right only as far as the branches actually need, growing **taller**
  instead: height scrolls vertically and labels truncate at the pane edge (#34).
  Nothing is drawn over anything else it would obscure (#1683): a row's **label starts past
  the rightmost line at that row** (`row_line_extents`), each **icon sits on a chip of the
  row's own background** so the lanes disappear behind the glyph instead of showing through
  it, and where two lines do cross, the **vertical run keeps going and the angled one breaks
  around it** with a small gap (two angled lines settle it by whichever starts further left,
  so the choice never flickers). A **constraint** is the exception (#1670): it relates
  to the geometry it holds rather than being produced by it, so it claims no trunk, drops into
  the leftmost free lane at or right of its sketch's, and ties to each element it constrains
  with a soft dashed **related** leg. Each node draws as its
  element's icon — the same icon its List row uses, tinted by selection/health state
  (#152) at its lane, with the label following its own dot so a row's indent shows its place in
  the graph; only the synthetic Document root keeps a plain dot. Clicking a row in Graph view selects it like any
  other row; selecting a node highlights its ancestor and descendant nodes/edges with a distinct
  accent color/stroke. **Right-clicking a node opens the same context menu its List/Tree row
  shows (#623)** — the shared `element_context_menu` (edit entries, Create drawing / Add to
  drawing, Make shadow body / Make live body (#1218), exports, Move-to-component, Rollback,
  Delete), plus the drawing rename menu and edge-treatment Edit on their display-only nodes.
  **With a drawing open, a body/sketch/component/cross-section node drags onto the page just
  like its List row (#1819)** — same `DrawingDragPayload`, same drop.
  **Right-clicking already-selected geometry in the 3D viewport opens that same menu
  (#1224).**
  This is a per-session UI preference, not saved with the document.
  Beyond the single tree-parent lanes, the Graph view also draws **dependency lanes** — in a
  warm accent, so they read apart from the neutral parent lanes — from an element's **inputs** to it (`graph_dependency_edges`), covering **every
  operation** (#448/#449): boolean/move/slice input bodies (#266), a repeat's input
  bodies/planes/sketches/replayed cut extrusions, a move's planes and images, a slice's
  construction-plane cutters, a revolution's profile sketch and axis line, the in-sketch
  repeat/slice ops' source lines/circles (+ the slice's cutter lines), a loft's section
  sketches, and a drawing projection's source body/sketch (#281) — the input edges of the
  eventual full element graph (#252). A Document→element tree-parent spoke is omitted
  when that element already has any other input (`graph_parent_edges`, #1324) — a fillet
  fed by the body it treats does not also hang off the document root. When a visible node
  is fed by a **shadow element that is not shown** (shadow bodies are hidden by default,
  #1109), the graph draws a **skip-lane** from the shadow's visible parent to the
  consumer (`graph_shadow_skip_edges`, #1425) so the chain stays connected without
  resurrecting the hidden node. Row order alone puts every input above what it consumes, so
  the layout is fixed and deterministic — the same document always draws the same graph
  (#1670), and the rows are readable back from a script with `bearcad.ui.elements_graph()`.
- **Graph rollback (#524/#531/#545):** a rollback marker lets you view the model as it was just
  after (or just before) a chosen element, temporarily suppressing what **depends on** it. It is
  set from an element row's right-click **Rollback** submenu (#545): **Rollback to here** keeps
  the element and hides only its dependents (`RollbackMarker { inclusive: false }`); **Rollback
  to just before here** additionally hides the element itself (`inclusive: true`). While rolled
  back, a `⏮ Rolled back to [just before] <name>` status with a **Done** button (#619, to roll
  forward) shows atop the Elements pane — there is no separate roll-back button. Suppression is by
  the **element graph, not creation time** (#531): `rolled_back_elements` walks forward from the
  marker along both the nesting tree (an op → its output bodies, a sketch → its geometry, …) and
  the dependency edges (an input → the operation that consumes it), collecting the
  marker's **descendants** (plus the marker itself when inclusive) — so two independent branches
  never affect each other, and rolling back a body hides exactly the operations built on it and
  their results. Those elements are
  hidden in the viewport — a render-only `ElementVisibility::with_hidden` union on top of the
  user's own toggles, so their user visibility is untouched — and **faded** in the pane
  (`RowStyle::Faint`, above health/selection styling). Per-session UI state; the marker drops
  automatically once its element is gone (deleted, or a new/opened document). This suppresses
  operations that **add** geometry (extrude, mirror, boolean, pattern, sketch geometry, …);
  rolling back an **in-place** modification that has no node of its own — an edge chamfer/fillet
  (#77/#168) — is tracked in #537/#538, which turn those into operation nodes with shadowed
  inputs so the same descendant walk suppresses them.

### 11.2 Command palette
- VS Code-style palette listing **context-pertinent** commands. Commands come from the
  shared action layer (§8) so palette, shortcuts, GUI buttons, and scripting stay in sync.
  A selected construction plane offers **Import image on this plane…** (#1564), the same
  action as the Elements-pane context menu: the GUI picks a file; a script passes the path
  as the second value (`bearcad.ui.palette("import image on this plane", "p.png")`).
- Coverage includes **every modeling tool** — the sketch tools plus Extrude, Chamfer, Fillet,
  Offset, Projection, Loft, Revolve, Sweep, Combine, Move, Mirror, Repeat, Slice, and Text (each
  `SetTool`) — and the **Selection Exploder** ("Explode Selection Under Cursor", #576), which the
  palette opens at the cursor on the next frame exactly like a Space press
  (`PaletteOutcome::OpenExploder` → `App::exploder_palette_request`, consumed by `tick_exploder`).
- **Commands can take an argument (#1022).** A `PaletteCommand` carrying
  `argument: Some(hint)` doesn't run when it is chosen — it *asks*, and the palette becomes
  the prompt: the command list is replaced by its name and an input carrying `hint`, the next
  Enter runs it with what was typed (`PaletteCommand::outcome(argument)`), and Escape goes
  **back to the command list** rather than closing, with the filter text still there, so a
  wrong turn costs one keystroke. The prompt is in the palette's own pane — nothing pops up,
  because the palette is already the place you are typing. State lives on
  `CommandPaletteState::pending`/`argument`, cleared on open and close so a stale prompt can
  never be resumed. Scripted as a third value:
  `bearcad.ui.palette("mcmaster", "socket head screw")`.
  First used by **Search McMaster-Carr**, which opens the catalog window (#1022) with the
  search already run for whatever was typed.

### 11.3 Shortcuts
- Sensible defaults for the most common actions.
- **Every action is rebindable**; custom bindings persist (per §7.2, in-document; global
  defaults in app settings).

### 11.3a Diagnostics (#978/#1023)
- Three levels in `diag`, so a terminal shows what happened without showing everything:
  **`warn`** (something is wrong), **`info`** (something notable — a document opened, an
  import landed, an action refused), both always on stderr; and **`log`**, the fine-grained
  trace, on stderr only under `BEARCAD_LOG` because it is far too much to read past.
- **Every level is written to a log file as well**, ungated: by the time you know you wanted
  logging, the run that broke is over. `$BEARCAD_LOG_FILE`, else `bearcad.log` in the system
  temp directory — somewhere predictable and always writable beats somewhere tidy. The path is
  printed on startup. The previous run is kept as `bearcad.prev.log`: an app that dies at
  startup is one you restart immediately, which would otherwise overwrite the evidence. Two
  files, not a rotation series — a third is archaeology.
- The file is opened by `diag::init`, which only the real app calls, so a `cargo test` run
  writes nothing to disk. A **panic hook** (`diag::install_panic_hook`) puts panics in the log
  too, above everything the run did beforehand — a panic is exactly the failure you cannot
  reproduce on demand.
- **A warning carries what the app was doing (#1823).** egui's multipass id-clash warning
  arrives as a rect and two id hashes, which nobody can act on. The app republishes a
  `diag::UiContext` every frame — frame number, workbench, tool, the pane rects and the open
  windows — and `diag::annotate_warning` appends the frame, the workbench/tool, **which pane
  the warned rect lies in**, what else was open, and where to report it. Other warnings are
  logged exactly as they came.
- **Every action is logged** from the one funnel it goes through (`AppState::apply`): its
  variant name at trace level, and its **refusal reason at info level**. A refusal previously
  only reached the status bar, where the next action overwrote it — which is precisely the
  failure nobody could explain afterwards.
- **The launch keeps painting until it settles (#1023).** The maximize countdown alone left
  the app idle after a handful of frames. On a **cold start** — the first run of a freshly
  built binary, which is when a grey window is actually reported — the GPU is still compiling
  pipelines while those frames are built, so they can be drawn and never *presented*; egui is
  reactive, so nothing asks for another and the window keeps showing grey. `tick_launch_maximize`
  therefore requests repaints for `LAUNCH_SETTLE` (3 s) past the countdown, which guarantees a
  frame lands once the GPU is warm and then stops — an app that repaints forever is a different
  bug. The decision is a pure predicate (`launch_still_settling`) so it is testable without
  reaching into egui's repaint scheduling.
- **`launch: ready` and the watchdog's verdict are `info`**, so stderr reaches a conclusion on
  its own: a terminal that gets to "ready" started properly, and at 8 s the watchdog says
  either how many frames were drawn (so a blank window is a *presentation* fault) or that too
  few were (a *scheduling* one). Without both halves visible, one of them reads as silence.
- **The app brings itself to the front at launch (#1032), and that is a correctness fix, not
  a courtesy.** An unbundled binary — what `cargo run` and every scripted run produce — does
  not activate itself on macOS. The window is created and mapped, but stays behind whatever
  was frontmost, and the window server then reports it **occluded** and stops handing it
  drawable updates. The app goes on building frames into a surface nothing displays, so the
  window shows its uninitialised backing: a grey rectangle, with every app-side signal
  healthy — frames counted, correct size, viewport blit landing. `window_probe::activate_app`
  (AppKit `NSApplication::activate`) is called where the launch settles, alongside the
  maximize. The same occlusion is why scripted screenshots intermittently failed with "the
  window never painted"; with activation they succeed reliably.
- **What the window server says is sampled too**, since it is the only side of this egui
  cannot see: `window_probe::appkit_window_state` reports each window's visible/occluded
  state, alpha, frame, and content-view size into the watchdog's verdict. Sampled on the UI
  thread (AppKit is main-thread only) and throttled, because a grey window stays grey.
- **wgpu's, eframe's, and winit's own logging is captured (#1032).** Nothing installed a
  `log` implementation before, so every record those crates emitted was dropped — including
  the surface faults a grey window is actually made of: a failed swapchain acquisition, a
  lost or outdated surface, a surface configured to a size the window no longer has.
  `diag::install_log_bridge` forwards them into the same file, at `Warn` and above by
  default (wgpu at `Info` narrates every resource it creates); **`BEARCAD_GPU_LOG=1`** opens
  it to `Debug` for a run that needs the whole conversation. Errors and warnings arrive as
  `warn`, everything else as trace.
- **A grey window's remaining questions are answered where the watchdog gives its verdict.**
  "It is painting" is a dead end on its own, so the verdict carries what only the UI thread
  knows: the size of the last frame, whether the 3D viewport's blit landed (and on how many
  frames it did not — a single warning at the first failure cannot say whether it recovered),
  and what the window says about itself (inner/outer rect, scale factor, monitor size,
  focused/minimized/maximized). The UI thread leaves this in `diag::note_window_state` each
  frame because the watchdog runs on its own thread and cannot touch egui.
- **Frame-size *changes* are logged, not just the first few frames.** A window that maximizes
  after launch and one whose surface never follows the resize both look identical otherwise —
  "frame 1 — 960×640" and then silence. Changes are capped (`TRACED_RESIZES`) so a live
  drag-resize cannot fill the file.
- The **catalog subprocess logs to its own file** (`bearcad-mcmaster.log`): both processes run
  at once, and two of them rotating one file would each destroy the other's evidence. Its
  stdout is the report channel (#1022), so everything it has to *say* goes to stderr and its
  log — a stray line on stdout would be read as a caught file.

### 11.4 Theming
- Light and dark modes, ideally a general theme system.
- **Icons are always independent SVG assets** (#325): every toolbar/pane/button icon is a
  bundled SVG in `src/assets/icons/`, referenced through `icons::IconId`, and rasterized with
  `currentColor` so it inherits the theme's tint. **Never render an icon as a font glyph** (a
  Unicode arrow, box-drawing char, emoji, etc.) — those fall back to an empty box wherever the
  UI font lacks the codepoint, which is exactly the bug this rule prevents. A button that pairs
  an icon with a text label uses `egui::Button::image_and_text` with the SVG texture, not a
  glyph baked into the label string.

### 11.4a Selection model (#951)

**This is the normative description of selection. Every tool follows it; a tool that needs to
deviate has to say so here, against the stated default.** The per-tool sections declare *which*
pickers a tool has and what each takes — they do not restate *how* picking behaves.

**Adding a tool?** Its entry point is declaring its pickers. Give the tool an ordered list of
`ToolPickerView`s (the first is its **primary**), each an `ElementPicker` configured with the
element kinds it accepts, any `PickRule`s, a pick limit, and — only if its elements are consumed
by the operation — a selected-color override. Everything below then applies without further
work: focus stepping, hover, the Exploder's fan, pane clicks, tool-switch handoff, and the
viewport highlight. Nothing about a new tool should need a new arm in a `match tool { … }`.

The model in one place:

- **Selection is scoped to element pickers.** A picker holds a list of elements, and defines the
  **number** and **types** it will take, plus any instance-level **rules** (`PickRule`, #953) —
  the design's "restrict selection to particular elements/components/bodies". The rules gate
  every path equally, so a viewport click, a pane click, and a tool-switch handoff can never
  disagree about what a valid pick is.
- **Every picker is registered, wherever it draws.** A tool's pickers all live in
  `ContextPaneContent::tool_pickers`; `ToolPickerView::render` says whether one draws in the
  shared block at the top of the tool's section (`Shared`) or in place among the tool's own
  controls (`Inline`) — the Move tool's point rows sit between the Rotation heading and the
  Angle-snap slider, so they can't be hoisted. Focus, hover, the tool-switch handoff, the
  Exploder's fan and `bearcad.ui.pickers()` all read that one list, and a picker missing from it is
  invisible to every one of them (#958). A tool pushes its **primary** picker first.
- **Exactly one picker has focus.** A tool declares its pickers in order; focus walks to the
  first unfilled one, which is how a single-pick picker hands focus on the moment it's filled
  (`FocusChain`, #954/#962). A hand-picked focus pins the chain until that picker is satisfied,
  except on the primary — focusing that by hand means "I want to keep adding to it".
- **Picks land in the focused picker**, whether they come from the 3D viewport, the list
  Elements pane, or the graph Elements pane (#963); in the panes, from the row's **name or its
  type icon** (#964). The armed picker gets first refusal; what it turns down goes to the
  tool's **primary** picker, and only then does the click fall through to the ordinary
  selection rather than being forced in. The fallback is what keeps a tool's main set reachable
  while a secondary picker is armed: Repeat arms its Path once there is something to repeat
  (#439), and without it a second body to repeat could not be gathered at all.

  Move and Repeat gather more than bodies — construction planes and tracing images move;
  planes, sketches and cut extrusions repeat — and each of those was its own
  `(SceneElement, Tool)` arm in the pane's cascade, invisible to the viewport and to scripts.
  They route by picker target now, so all three paths gather the same things.
- **One resolution for the hover and the click** (`pick_for_focused_picker`, #958/#970): the
  crowd under the cursor, kept to what the picker can make of each candidate, ranked by that
  picker's own priority. What lights up is what lands, by construction. A tool's viewport
  handler calls `click_into_focused_picker` and keeps only what is genuinely its own — gizmo
  drags, Enter-to-commit, preview state — and the pick itself goes through `actions::apply_pick`,
  the same function a pane click and a script use.

  Three kinds of pick genuinely can't be an "element under the cursor", and their tools keep
  their own resolution — this is the whole list, and a new tool needs a reason this good to
  join it:
  - **A profile region** (Extrude, Revolve, Sweep, Loft): what's picked is the region the
    cursor is *inside*, which can be a boolean of two overlapping shapes
    (`ExtrudeFace::Boolean`). No element names it — `FaceId` flattens it to one operand — so
    `pick_extrude_face` builds it from the click.
  - **A derived point** (the Move and Joint snap pickers): an edge's midpoint or a face's
    centre isn't in the crowd at all; the click means "the point *of* the thing under the
    cursor".
  - **A geometric relation to an earlier pick** (a Sweep path line must leave the profile's
    plane; an Extrude/Repeat "up to" target must lie ahead along the normal): the test is
    against another pick, not a property of the element.
- **Switching tools carries the picked set** from the outgoing tool's primary picker to the new
  one's, keeping what it accepts (#956).
- **What a picker holds is styled as selected** in the viewport and in the Elements pane, in
  **that picker's** color (#961/#965).
- **The panes say what the armed picker can take** (#965): a hovered row the picker accepts
  wears a wash of the pick-hover yellow, the same signal the viewport gives. A row it refuses
  gets the ordinary hover — it is not inert, because a refused pick still falls through to the
  selection (see pane clicks above); it simply doesn't claim to be a pick. Both panes show the
  elements the **hierarchy** names — bodies, sketches, operations, components, planes, images,
  constraints. A body's faces, edges and corners have no row and no node in either: they are
  picked in the viewport, where they exist, and reached in a crowd through the Exploder.
- **One pick priority** decides among things crowding the cursor, overridable per picker (#959).
- **A face click fills an edges picker** with that face's edges when the picker takes edges and
  not faces (#960) — but never a **single-pick** one, which has one slot and nowhere to put a
  face's worth of edges.
- **Everything the focused picker can take highlights on hover**, and the Selection Exploder
  fans only what it can take. *Partly true in the code*: the hover path
  (`resolve_viewport_hover_highlight`) is still a per-tool match, but its catch-all no longer
  returns nothing — it collects **everything under the cursor**
  (`construction::collect_pick_candidates`, the same crowd the Exploder fans), keeps what the
  **focused picker** can make of each (`expand_pick`), and lights the best by that picker's own
  ranking (#958/#959) — the same `pick_for_focused_picker` the click uses, so what lights up is
  what lands. So a tool with no hand-written arm has hover feedback instead of none,
  and it hovers what its picker *wants* rather than whatever happens to outrank it globally — a
  datum plane no longer wins over the body a Joint takes. That path is also what gives a tool's
  **secondary** pickers hover of their own: the body-set arm and Repeat's axis arm are gone, so
  Slice's Cutters and Combine's B side hover what they take rather than what their tools' body
  sets take, and it is why the six tools that pick a **face to sketch on** — Sketch, Text,
  Rectangle, Line, Circle, Offset — share one picker (a plane or an analytic face, single-pick)
  instead of four arms: they also share its Exploder fan, which is how you reach a datum plane
  buried behind a body. The arms that remain are the picks the model can't express, and
  `resolve_viewport_hover_highlight` lists them: a closed profile computed from the click, a
  derived point, a picker with its own candidate set (Move's end point B), a pre-empt the
  priority can't state (Select hovers a whole body while its fan offers that body's parts), and
  the plane tool's second priority order.
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
  **always shown and always focused** — not by a flag on the control, but because it is that
  tool's only picker, so "exactly one picker has focus" already puts the focus there (#966).
  **It *is* the selection** (`SceneSelection` holds it), rather than a view rebuilt from one
  each frame: that is what gives its popup rows a stable order — they used to be sorted by each
  element's debug string purely so index→element agreed between the frame that draws a row and
  the frame that handles its ✕ — and what makes any rule it carries real rather than applied
  after the fact. Suppressed only while a draw
  construction owns the pane. Each picker instance is configured with: the subset of element
  kinds it accepts — the full set (`ElementKind::ORDER`, #952) is planes, images, sketches,
  lines, circles, **axes**, vertices, edges, faces, **profiles**, constraints, bodies,
  **components**, **joints**, and operations (and, for operations, which sub-kinds); every kind
  must be listed
  in `ORDER`, since that is what `ElementFilter::kinds` builds from and what the collapsed
  summary counts — a pick limit (a whole number or unlimited), an
  optional override of the selected-element highlight color (defaulting to the theme selection
  color), and any number of **pick rules** (#953).
- **The drawing workbench (#967)** is inside the model, not beside it. A thing on a page is a
  `SceneElement::DrawingElement { drawing, element }`, and its three sorts are three kinds —
  `Projection`, `Annotation`, `Dimension` — so each row keeps the icon the Elements pane gives
  it (#363) and the Aligned-view tool's **Base view** can ask for a projection and nothing else
  (a `Finite(1)` picker, where it used to be an `Option<Option<…>>` standing in for one). The
  Select tool's page selection is an ordinary multi-pick picker over the three. The **Add-view**
  tool has no picker: its click *creates* a projection of what was clicked rather than gathering
  it, which is a different act from picking.
- **Two representations of a face (#957):** a body's cap reaches the cursor twice — as the
  quantized mesh face (`ElementKind::Face`) and as the analytic surface that generated it
  (`ElementKind::Profile`: a sketch profile, an extrude cap or side wall, a revolve's flat
  face). They are different elements, so the crowd's dedup can't collapse them, and a picker
  says which it wants: the tools that *build* from a face (Extrude, Revolve, Sweep, Loft, the
  Slice cutters, the Mirror plane, the three "up to" targets) take `Profile`; only a picker
  taking everything takes both, and gets the mesh one.
- **Pick rules (#953):** a picker's kinds say *what sort* of thing it takes; its rules say
  *which ones*, the design's "restrict selection to particular elements/components/bodies". All
  of a picker's rules must pass, and they gate every path equally — `accepts`, `pick`, and
  `set_picked` — so a viewport click, a pane click, and a tool handoff can never disagree.
  `PickRule` is data, not a closure, because a picker lives inside the diffed
  `ContextPaneContent` and must stay `Clone + Debug + PartialEq`. The rules:
  `InSketch(sketch)` (what the open sketch owns, #742 — the same `element_in_sketch` the hover
  and click paths use, so there is one definition and not three; the in-sketch Mirror, Offset
  and Repeat pickers carry it, which is what let the Mirror tool's hover arm go),
  `ProjectableInto(sketch)` (the Projection tool's sources, #983: outside bodies/edges/corners,
  planes that cross the sketch, and the sketch's own projected lines — picked to un-project
  them), `LiveBody` (not deleted, not
  consumed by another operation), `OnBodies` / `OffBodies` (the Move tool's start points land on
  a **moving** body, its end points on stationary geometry, #649/#650), `Straight` (a
  Revolve axis or Repeat path takes no curve), `Construction(bool)`, and `NotIn(…)` (Combine's B
  side can't take what side A holds). The Select and Constraint tools mirror the live selection; the construction tools
  (Combine, Move, Repeat, Slice, Revolve-cut, Loft, Chamfer/Fillet) each present their own
  in-progress picked set through the same control — with the currently-active picker focused
  (a tool with several, e.g. Combine's A/B sides or Slice's bodies/cutters, switches which is
  focused when you click it). Whatever a picker holds is **styled as selected in the viewport
  and in the Elements pane** while the tool is active (#965; folded into the highlight set each
  of them shows, not into the persistent selection) — a body gathered into Move's set is as
  picked as one in the selection, and the two views should not disagree about the same thing.
  Which color it renders in comes from the **picker** (#961): the theme selection
  blue by default, the red cut accent for a picker whose elements the operation consumes. The
  viewport iterates the active tool's pickers (`picker_highlights`) rather than matching on the
  tool, so a set lights up because its tool *has* a picker for it, not because the viewport was
  told about that tool. A destructive picker's **bodies** take the red solid fill
  (`cut_highlight_bodies`); everything else it holds — a Slice cutter's plane or face, an
  in-sketch cutter line — has no fill to recolor and draws in the picker's color through
  `colored_element_highlights` instead. Deriving the pickers is therefore **not** gated on the Context pane
  being visible (#973) — the pane's visibility gates only its rendering.
  While a body-set tool (Combine/Move/Repeat/Slice) is active, the **body under
  the cursor hover-highlights** as selectable — the same whole-body resolution the click uses
  (#227).
- **A tool switch carries the picked set (#956):** changing tools hands the outgoing tool's
  **primary** picker (its first — the main set it works on) to the new tool's primary picker,
  which keeps whatever its filter, rules, and limit accept and drops the rest. So bodies
  gathered in Combine are what Move moves and Repeat repeats, without re-picking; switching to
  a tool whose primary picker takes faces carries nothing, because nothing offered is valid.
  The primary rather than the merely focused picker: once the Move tool has advanced focus to a
  point picker, its focused set is two mating points, which means nothing to Repeat — the
  bodies do. When the outgoing tool has no picker of its own, the Select tool's **selection**
  stands in, since that is its picker; the pre-existing "select bodies, then pick Combine"
  behaviour (#943/#900/#523/#439) is that same rule rather than a separate path.
- **Whole-body vs. sub-element picking (#218):** a viewport click picks a **whole body** only
  when the focused picker's accepted types exclude edges, faces, and vertices — so the
  body-set tools (Move/Repeat/Slice/Combine, Revolve cut), whose pickers accept only bodies,
  select a whole body by clicking anywhere on it (edge, corner, or flat face); the Select tool,
  which accepts sub-elements, picks the edge or corner instead (its **face** picks resolve to the
  whole body, #902). Regardless of that, an element
  **clicked in the Elements pane** (or otherwise selected) is offered to the **focused** picker
  first (#963) — the same picker a viewport click feeds — so you can gather from the pane even
  for tools where the viewport is picking sub-elements, and a tool's *secondary* picker is
  reachable from the pane at all (a construction plane clicked while Slice's **Cutters** is
  armed lands in Cutters). The picker decides: its kinds, rules, and limit say whether the
  element is a valid pick, and one it refuses falls through to the ordinary selection rather
  than being forced in. A picker is armed by clicking it, or from a script with
  `bearcad.ui.picker_focus(name)`.
- **No picking through bodies (#155/#265):** while selecting (Select/Constraint tools, picks
  made for a tool such as construction-plane references or dimension targets, and the
  body-set tools Combine/Move/Repeat/Slice/Revolve), geometry hidden **behind** a visible
  body under the cursor is not a pick candidate — clicking a body never selects a line buried
  inside or behind it, and a body-set tool can't pick a body through one in front of it. The
  probe point is the spot on the candidate nearest the cursor, so a partially hidden edge
  stays pickable along its visible stretch; hiding a body (Elements pane) removes it as an
  occluder, restoring the old X-ray behavior deliberately.
- **A face click fills an edges picker (#960):** when the focused picker takes **edges** and not
  **faces**, clicking a face means *all of that face's edges*, and hovering it lights all of
  them up — so what the click does and what the hover shows are the same thing. Without this a
  face click was simply refused, with nothing on screen to say why.
  `element_picker::expand_pick` is the rule: the element itself when the picker accepts it,
  otherwise the face's boundary filtered to what the picker *can* take (a mesh face's feature
  edges, a sketch profile's lines, a circle profile's circle), and nothing when neither applies.
  The 3D Chamfer/Fillet tool is the case in the viewport; the Elements pane routes through the
  same rule (#963).
- **Pick priority (#959):** when several things crowd the cursor, one shared ranking decides
  which wins — `element_picker::default_pick_band`, by the candidate's element **kind**:
  vertex → the linear kinds (edge, line, circle, axis) → constraint → face → plane/image →
  sketch → body → operations, with the ground last. A tie *inside* a band goes to whichever is
  nearest in pixels, so a sketch line and a body edge under the same pixel compete on distance
  rather than one kind always outranking the other. A picker may override the ranking with
  `ElementPicker::with_priority` — it names only what it wants promoted (the design's "faces over
  edges"), and everything unnamed keeps the default order behind those. This is the single
  definition: `PickTarget::beats` reads it, replacing a `u8` hand-assigned at each of eight
  candidate construction sites plus a bolted-on vertex-beats-edge special case (#242), which the
  band ordering now states outright — as it does the origin beating its own axes (#240).
- **3D body sub-element hover (#144):** with the Select tool, hovering a 3D body highlights the
  **vertex, edge, or face** under the cursor — in that priority order (a corner beats an edge on
  it, which beats the face they lie on), so it is always clear what a pick would grab. Edges are
  the solid mesh's feature edges (`solid_mesh_unique_edges`, the same crease/boundary edges the
  wireframe draws, so this works for any body — extrusion-sourced, boolean-cut, or imported);
  vertices are the mesh corners; a face is the maximal edge-connected group of coplanar triangles
  (`solid_mesh_coplanar_faces`), so a whole box side or cylinder cap highlights as one face, with
  the nearer face winning when two project onto the cursor. The Chamfer/Fillet tool likewise
  hover-highlights the treatable analytic edge under the cursor before it is clicked.

### 11.5 3D interaction
- Orbit/pan/zoom the 3D rendering; select faces/edges/vertices; manipulate sketches and
  features directly in the viewport.
- **Default viewport bindings** (all rebindable per §11.3):

  | Input | Action |
  |---|---|
  | Right-drag | Orbit the camera |
  | **Middle-drag**, or **Shift + right-drag** | Pan the camera (slide the view target in the view plane). The web host `preventDefault`s the browser context menu so Shift+right-drag can pan (#1447). Firefox still forces its native menu on Shift+right-click regardless, so middle-drag remains the browser-safe fallback there (#195). |
  | Mouse wheel | Zoom (dolly in/out) |

- **Zoom to Fit (#164/#279/#1276):** available from the toolbar **Zoom** button (magnifying-glass
  icon, in both the Model and Drawing workbenches), the **`Z`** shortcut (plain `Z`; `Cmd/Ctrl+Z`
  stays Undo), the command palette ("Zoom to Fit"), and the View menu. Frames the **current
  selection** (union of the selected elements' world bounds) so it nearly fills the viewport;
  with nothing selected it frames all **non-construction** geometry (bodies plus solid sketch
  lines/circles — construction scaffolding and datum planes are ignored). Glides over
  `ZOOM_TO_FIT_DURATION` (half of Home/`VIEW_TRANSITION_DURATION`, #1303); Settings →
  **Animate zoom to fit** (on by default) turns the glide off for an instant snap. Scriptable via
  `bearcad.ui.zoom_fit()` and `bearcad.ui.animate_zoom_to_fit(bool)`.
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
  distinct accent color and follows the shape of the target (line stroke, face outline,
  ground crosshair, etc.).
- **Proximity picking:** thin or point-like geometry (lines, endpoints, vertices) must
  be pickable within a screen-space tolerance — the pointer need not land exactly on the
  stroke. Lines use a pixel-radius threshold around the segment and its endpoints; faces
  use a margin around their projected edges. Hover resolution and click picking share the
  same resolver so feedback matches what a click would select. **The per-body mesh analyses
  the pick path needs — coplanar face groups, feature edges, and edge chains — are memoized on
  the document's mesh fingerprint** (`extrude::body_face_groups` / `body_feature_edges` /
  `body_edge_chains`, #845), like the body meshes themselves. Re-deriving them per body per
  frame is what made a heavy document (engraved text is thousands of triangles) crawl while
  the cursor sat over the model — a ~20× frame-time difference on the reporter's part.
- **Shape edges:** when a tool accepts a line or axis reference (e.g. construction-plane
  creation), standalone sketch lines and individual edges of shapes (rectangle sides, etc.)
  are all valid picks. Shape edges take precedence over the shape's face when the cursor is
  near the edge. **Real geometry outranks a datum plane (#844):** when a click could mean a
  body/sketch face or a construction plane at the same screen distance, the face wins — even
  when the plane sits between it and the camera. Planes are translucent references, and they
  stay pickable everywhere a body isn't. Face depth is compared at the **point under the
  cursor** (`face::face_point_under_cursor`), not the face's centre, so overlapping faces
  order by what's actually in front there.
  Construction planes are the one exception (#124): they extend infinitely,
  so their rendered border is a display artifact, not real geometry — it isn't pickable as
  an edge/axis reference, only the plane's face is.
- **3D body faces (#465):** any planar face of any 3D body is a valid *face* reference for a
  construction plane — it hover-glows under the Plane tool and a click anchors the new plane
  on it (origin at the face centroid, normal the face normal, offset along the normal). A
  face wins over the ground/plane-quad fallback but loses to sharp targets (points, edges,
  axes) under the cursor (`construction::resolve_plane_pick_target`). Scriptable as
  `bearcad.plane{ offset?, origin = {x,y,z}, normal = {x,y,z} }`. An **axis** (world X/Y/Z
  or a sketch line) is `bearcad.plane{ axis = "x"|"y"|"z"|line, angle?, offset? }` (#1876).
- **3D body edges (#31):** any edge of any 3D body — not just 2D sketch geometry — is a valid
  axis reference for a construction plane, including STL/STEP-imported bodies. An edge here is
  a *feature* edge of the body's triangle mesh (a mesh boundary, or a crease where adjacent
  triangles' normals differ by more than ~15°, so flat-face triangulation diagonals *and* the
  small seams between facets approximating a smooth curved surface are both excluded, #82/#101)
  — the same extraction `ShadingMode::Wireframe` uses to draw a body's edges — so it works
  uniformly for any body regardless of how it was created, without needing an analytic profile.
- **Curve vertices (#474):** a sketch vertex on a line or curve is a valid *point* anchor for a
  construction plane: the new plane passes through the vertex with its **normal along the
  curve's tangent there** (a straight line contributes its own direction; a bezier the tangent
  at that endpoint), so the curve is normal to the plane at that point — with the usual offset
  gizmo + text field walking the plane along that direction. When two or more lines/curves
  meet at the vertex (via coincidence), the Plane tool's context section shows a single-select
  **Normal** picker to choose which incident line the plane is normal to (#612), the first by
  default; the context section also shows the picked **Anchor** (face, edge, or vertex) as an
  element-picker row whose ✕ clears it to re-pick (`construction::vertex_normal_candidates`).
- **The Anchor holds elements (#955):** the plane's `reference` is a *derived* frame — an
  origin and a direction — so the tool keeps `anchor_elements` (and each row's own frame in
  `anchor_refs`) beside it, and the Anchor input is a real picker registered in `tool_pickers`
  like every other (§11.1). Rows read `[point, line]` for a line+point set, so dropping one
  row leaves the surviving half as the anchor on its own rather than cancelling the plane;
  dropping the only row starts over. Re-opening a *committed* plane for edit shows an empty
  picker — its stored definition keeps the frame, not what was clicked to make it.
- **Plane tool context inputs + commit (#611/#613/#614):** once an anchor is picked, the Plane
  tool's context section shows an **Offset** value input, plus — for an edge/axis anchor — an
  **Angle** input, both mirroring the floating 3D fields (they edit the same in-progress plane,
  so pane and viewport stay in lock-step). A blue **Create plane** primary button (and Enter)
  is the *only* way to commit — a stray viewport click never creates the plane; clicks only
  grab/drag the gizmo or complete a line+point anchor.
- **Line + point (#483):** the Plane tool's **Anchor** accepts three complete sets: (1) one
  planar face, (2) one **straight** edge (line lies *in* the plane; angle spins around it), or
  (3) one line/curve **and** one point together — plane through the point with normal along
  the line. A **curve alone is not complete**: pick the curve first (held in Anchor), then a
  point; if that point is an endpoint of the curve, the normal is the curve's tangent there
  (same as #474). Straight edge then point upgrades the same way. After a vertex start, click
  a line/curve to set the normal. Anchor rows show both labels
  (`construction::complement_plane_anchor`, `pending_plane_line`).
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
  - **Ground** — how the ground plane renders (#159), one of three icon options:
    - *Ground grid*: the classic line grid (the default).
    - *Solid ground*: one filled plane in a dark grey-blue at exact z = 0, drawn as a
      no-depth-write shader pass (like the grid) so coplanar construction planes never
      z-fight it without geometric/pipeline bias (#1301); body faces on the ground re-draw
      after plane fills so coplanar pairs stay clean (#1215). The X/Y/Z axis lines still
      draw on top for orientation.
    - *Off* (#579): no grid and no solid fill — a clean background — though the world axes
      still draw for orientation. Used by the doc screenshots for an uncluttered body shot.
    Looking up from under z = 0 hides both the grid and solid fill; axes remain (#1300).
    Scriptable via `bearcad.ui.ground("grid" | "solid" | "off")`.
  - **Fill depth-biasing:** coplanar decals (sketch-shape fills, hover fills, stroke
    overlays) combine small world-space millimetre lifts with **slope-scaled pipeline
    depth bias** toward the camera (`wgpu::DepthBiasState`, sketch-fill and overlay
    pipelines): constant offsets alone collapse under glancing-angle depth interpolation
    on long thin faces (stippled z-fighting on e.g. an 8 ft board); the slope term grows
    the bias exactly where the depth gradient does. Construction-plane fills themselves
    take **no** bias (#1088/#1121); body faces that lie on a plane are re-drawn after the
    plane wash so coplanar solid/plane pairs stay clean (#1215).
  - **In-plane dimension-label text (#454):** committed dimension labels lie **in the
    dimension's plane**, flat with their dimension lines and arrows. Glyphs are laid out
    on orthonormal in-plane axes with **one uniform** world-per-pixel scale taken at the
    label center (`dimensions::planar_label_frame`), sized like screen text there and
    foreshortening naturally with the plane under perspective — but never shearing (the
    old per-axis scales plus bilinear screen warp did, badly, when zoomed out). The text
    reads along the dimension line, flipped to stay upright, and lifts slightly toward
    the eye so it never z-fights the face it annotates.
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
      of dropping to the ambient floor when the fixed light misses it. No materials/textures
      yet — every body renders with the same fixed gloss; per-body/per-face materials are
      future work. Bodies also cast **shadows** along the scene light: a planar
      contact shadow onto the ground (#1041) and a **directional shadow map** onto
      other bodies and themselves (#1461/#1535). The map is a depth test from the
      light — light-facing surfaces stay lit, occluded surfaces go to a dimmer
      headlight fill, and convex exteriors are not painted with hole-cavity decals.
      Ground-plane shadows stay at z = 0 (no camera-space lift) and stay hidden when
      looking up from below (#1464/#1476).
    - *Loose pencil / Colored pencil (#1805/#1812)*: the scene drawn by hand on warm paper —
      a laid-on fill so a near edge hides a far one, every feature edge and silhouette gone
      over twice with a repeatable wobble, and a hatched contact shadow instead of a smeared
      blot. Plain pencil is one graphite color; colored pencil keeps each body's material
      color and **scribbles it in** (#1818/#1825): the fill goes most of the way to the paper
      (`pencil::scribble_ground`) and every flat big enough to carry them
      (`PENCIL_FLAT_MIN_AREA_MM2`, at most `PENCIL_FLAT_LIMIT` per body — a tessellated sphere
      is hundreds of facets) is gone over with ruled strokes that `pencil::scribble` breaks
      into pieces: run a little past the outline, with gaps of bare paper between them, the way
      a hand fills a shape quickly. **One density and one tone on every side (#1825)** — no
      light-and-dark, which read as a render; a colored pencil drawing gets its form from its
      outlines, exactly as the plain pencil mode does. Solids do cast hatched shadows **on each
      other** (#1818), not only on the ground: every lit flat is a receiver, everything standing
      over it is projected onto its plane along the light, and the hatch is clipped to the flat.
      `crate::pencil` holds the strokes, tones, scribble and hatching, shared with the drawings
      workbench's pencil styles (#1809/#1821). **A cross-section's hatch and cut outline are
      drawn by the same hand (#1826/#1827)** — a perfectly ruled hatch in the middle of a
      hand-drawn view reads as a machine drawing pasted in. The hatch uses `stroke_within` with
      the wobble held to `RULED_WOBBLE_OF_SPACING` of the hatch spacing: the free-hand cap is
      nearly half that gap, so on it every line crossed the next and the cut face filled in
      solid. (The cap bounds the *resultant* — the wobble goes on along two axes at once.)
    - *Watercolor (#1829)*: the pencil drawing, painted. Same paper, same hand-drawn outlines,
      same hatched contact shadow; the color is laid on as a **wash** instead of a scribble —
      a ground that covers (`pencil::wash_tone`, far less paper showing than the pencil one),
      soft overlapping bands where the pigment **pooled** (`pencil::pooling`), and the deeper
      **rim** a drying wash leaves against every edge of a flat (`flat_boundary_edges`, the
      edges only one triangle owns). The rim goes down as several thin passes rather than one
      thick stroke: a thick round-capped line beads at every joint of its own wobble, which
      reads as bubbles rather than gathered pigment. `ShadingMode::is_drawn_by_hand` is what
      the paper palette and the popup's second row are keyed to, so the three stay one family.

  **Lighting runs per pixel, on smooth normals (#1037).** Solids carry a world-space normal
  and a lighting-model tag per vertex (`GpuVertex::normal`, whose `w` is a `ShadingModel`);
  `shader.wgsl` lights them in the fragment stage. Everything that is not a body — lines,
  fills, text, gizmos, the grid — is tagged `Unlit` and its color passes through untouched.
  The shader is the single source of truth for the lighting maths; the CPU `realistic_shade`
  is a test-only mirror, and `realistic_terms_match_the_shader` pins the shared constants to
  the WGSL source so the two cannot drift.

  **Lighting is done in linear space and tonemapped (#1038).** The viewport's render target
  is a plain UNORM format, so nothing encodes on our behalf: the shader decodes each base
  color from sRGB, does its arithmetic in linear light, applies a filmic tonemap, and
  re-encodes. The specular is *added* light rather than a lerp toward white, so a highlight
  keeps the material's color underneath and the tonemap's shoulder rolls off the overshoot
  instead of clipping to a flat white disc. The tonemap is Narkowicz's ACES fit normalized
  by `aces(1.0)` (`ACES_WHITE`), so adopting it does not darken the whole image.
  The ambient/diffuse weights are therefore **linear-space** values: `SOLID_AMBIENT`/
  `SOLID_DIFFUSE` and `REALISTIC_AMBIENT`/`REALISTIC_DIFFUSE` re-encode to roughly the
  sRGB-space weights the pre-#1038 maths used, so the change reads as better-lit rather than
  uniformly brighter.

  The normals themselves are **derived from the mesh**, not read off the kernel
  (`extrude::smooth_normals`): each corner averages the area-weighted normals of every
  triangle meeting at that position, but only those within `CREASE_ANGLE_DEG` (30°) of its
  own. Curved walls smooth; box corners, chamfers, and extrusion caps stay crisp. Deriving
  them this way means analytic primitive meshes, the hand-rolled fallbacks, and OCCT output
  all go through one code path. They are memoized per document state
  (`extrude::body_smooth_normals`, keyed off the pose fingerprint like the mesh caches) and
  shared by `Rc`, so a frame costs a refcount bump rather than a rebuild. A preview or ghost
  mesh with no normals falls back to its per-triangle geometric normal.

  Both rows are backed by `Camera` state (a viewport display preference, alongside
  projection mode — not saved model geometry) and are fully scriptable:
  `bearcad.ui.toggle_projection()` / `bearcad.ui.view("orthographic" | "natural")` for
  projection, and `bearcad.ui.shading("wireframe" | "transparent" | "solid" |
  "solid_wireframe" | "realistic")` for shading.

### 11.6 First-person (FPS) mode (#91)

A completely different control scheme for walking around (and inside) models like a
first-person game. **Experimental** — labeled as such in the command palette
("Toggle FPS Mode (experimental)"), the View menu ("FPS Mode (experimental)", checked
while active), status bar, shortcuts window, and docs. Toggled via those UI surfaces,
`Action::ToggleFpsMode`, or `bearcad.ui.first_person()`. The document is millimeters, so the
player is person-scale: eye height 1700&nbsp;mm, walking ~4.3&nbsp;m/s.

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
  smaller/larger person, not a world zoom); look sensitivity and `first_person_move`'s
  explicit mm offsets are unaffected.
- **Scripting:** `bearcad.ui.first_person(on?)`, `first_person_look(dx, dy)` (degrees;
  positive dx looks right, dy up), `first_person_move{ forward?, strafe? }` (mm along the
  ground), `first_person_jump()`, `first_person_fly(on?)`, `first_person_advance(seconds)`
  (integrates physics with no keys held, e.g. to land a jump), and
  `first_person_scale(value)` (sets the player scale directly, clamped as above).
  Outside first-person mode these raise catchable errors.

---

### 11.x Touch devices

- **Touch mode** latches on at the first touch event (`touch::detect`; scripts force it
  with `bearcad.ui.touch(true/false)`). It scales pick radii finger-sized
  (`touch::hit`, ×1.7 — sketch points/lines, gizmo handles, revolve handles) and grows
  widget interact sizes/padding (`theme::apply`).
- **Gestures** (viewport): two-finger drag pans, pinch zooms about the gesture centre
  (converted to the camera's scroll units by `touch::zoom_factor_to_scroll`, exact
  round-trip), and a **three-finger** drag orbits (`touch::ORBIT_FINGERS`) — fingers have
  no right button. **One finger never navigates (#754):** it belongs to the active tool, so
  drawing and dragging geometry in a sketch don't fight the camera. Trackpad pinches (`Event::Zoom`, incl. browser ctrl+wheel) zoom
  identically. Touch navigation owns the pointer (no hover/pick churn mid-gesture), and
  the status-bar tool hints swap their mouse wording for gesture wording at display
  time.
- **Gesture vs. drawing-tool disambiguation:** a two-finger gesture's first finger lands
  a beat early and reads as a primary press, which a drawing tool consumes as a
  placement click — when the second finger arrives within 0.5 s of that press while a
  rect/line/circle is in progress, the stray placement is cancelled. And since a touch
  user's next tap after finishing a shape is almost always a pick, **committing a
  rectangle or circle in touch mode returns to the Select tool** (desktop keeps the
  drawing tool armed; the Line tool keeps chaining on touch too).
- **One keyboard at a time (web)**: value fields (expression/dimension inputs) mark
  their focus (`touch::set_value_field_focused`, cleared per frame); on touch web
  builds the app stamps `inputmode="none"` onto eframe's hidden text agent while a
  value field is focused (re-focusing it so iOS re-evaluates), so the OS keyboard
  stays down where the app keypad serves — and free-text fields (names, notes,
  search) get the OS keyboard with the keypad standing down.
- **On-screen keypad**: in touch mode, focusing a **value** field floats a keypad above
  the status bar — digits, `.`, `=`, `+ - * /`, unit keys (mm/in/deg/ft), **parameter
  chips** (first five names), Back, Enter, Hide. Keys type by queueing **synthetic
  input events** flushed at the next frame start with focus handed back to the field
  (`tick_touch_extras`/`show_touch_keypad`), so every existing field — expressions,
  autocomplete, inline definitions — behaves exactly as if typed on a keyboard and the
  OS virtual keyboard never covers the viewport. The queue **waits for the field to actually
  hold focus** before firing (focus doesn't always land the same frame it's requested);
  firing regardless dropped keystrokes — a tapped digit would simply not appear (#831). It
  gives up after a few frames so a queue can't get stuck. The keypad follows the last focused
  field and tolerates the focus blip a key tap causes; it hides via Hide or ~30 frames
  with no focused field.
- **Drawing loupe (#755):** a fingertip covers the very point it's aiming at, so while a
  finger drags a sketch shape out (`creating_line`/`creating_rect`/`creating_circle`) or
  drags a vertex, a round **magnifier** floats beside it (`touch_loupe`, radius 64 px,
  ×2.6). It paints the open sketch's own geometry (lines, circles, their endpoints and the
  sketch origin), the shape in progress in the preview color, and the live snap ring —
  all projected, blown up about the fingertip, and clipped to the disc
  (`draw_touch_draw_loupe`, sharing the exploder's `clip_segment_to_disc`). A crosshair at
  the disc's centre marks the fingertip itself. It sits **above** the finger, and moves to
  the **side** (whichever has room) when the finger is too near the top of the view to fit
  — always fully inside the viewport and never over the fingertip (`touch_loupe::center`).
  It shows only while the finger is actually down: a touch pointer exists only then.
- **Long-press = right-click**: a touch press held ≥0.6 s within an 8 px slop
  (`touch::long_press_fires`, fires once per press) injects a synthetic secondary
  click at the finger, opening the same context menus.
- **Compact layout** below 700 logical px width (phones): the view-cube HUD drops to the
  **background** layer so an open pane covers it rather than the other way round (#830), and
  the three side panes render
  as **closable floating windows** over the viewport instead of docked panels
  (`show_pane_shell`), hidden by default (one-shot on first compact frame), toggled
  from always-visible **Elements/Context/Params** buttons in the status bar; the
  toolbar scrolls horizontally; the status text truncates into the space the right
  cluster leaves. `BEARCAD_WINDOW=390x760` opens a fixed-size window for testing it.

### 11.x Interactive tutorials

- **Tutorial mode**: guided, in-app walkthroughs narrated by **Bear** (the view cube — the
  narration calls him by name, #766). A
  **Tutorial** button in the bottom-right status bar (beside the update badge) opens the
  **Tutorials pane** (also View ▸ Panes ▸ Tutorials, #1291) listing every registered
  tutorial (`tutorial::TUTORIALS`); the whole row (status circle + title) is clickable and
  highlights on hover (#1252). The button is **bright blue** while any walkthrough is
  unfinished. **Mark all complete** in the pane finishes every walkthrough and drops the
  highlight and any tutorial prompting; **Mark all unstarted** clears those checks.
  Starting one opens a fresh document
  and resets the camera to the home view (#1261). Finishing a walkthrough reopens the pane
  when other tutorials are still incomplete (#1289). For the first 30 days after a
  **fresh install** (not an upgrade), if walkthroughs remain unfinished at launch, a
  standout tooltip points at the button: *Want to try some tutorials?* It fades a few
  seconds after the user starts working on the document.
- **One action per step** when authoring (#1253): every click is its own step, every bit of
  typing is its own step — never combine a click with typing (or two clicks / two typed
  values) in one step. See the module docs on `tutorial`.
- **First tutorial (`cube`)** (#1256–#1259/#1262): ultra-short beginner copy, no typing and
  no parameters — click a rectangle on the ground, extrude, Enter. After a tool is selected,
  placement steps use **world** orbs on the ground/face (not the toolbar button again).
  **Dimensions** (`dimensioned_box`, #1315–#1318) draws a free rectangle, then uses the
  Dimension tool to set sizes — not the rectangle tool's width/height fields. Reopening a
  sketch is Elements double-click or right-click → Edit sketch; changing a committed dim is
  double-click the label, then type. After the edit: Esc finishes the sketch, Z zooms to
  fit. Titles: **Sketch & Extrude**, **3D Bodies**, **Dimensions** (#1317). No parameters
  in that walkthrough (#1316).
- **Parameters** (`parameters`, #1347): add `width` in the Parameters pane, type `width` and
  `width*2` into the rectangle fields, change `width`, extrude with inline `height=30mm`,
  then change `height` in the pane. Title: **Parameters**.
- **Chamfer** (`chamfer`, #1555): rectangle on the ground, chamfer one corner, extrude, then
  click the top of the box so Chamfer takes every edge of that side. Title: **Chamfer**.
- **Constraints** (`constraints`, #1591): draw a four-sided polygon with the Line tool, then
  pin a side horizontal (parallel to X), a neighbour vertical (parallel to Y), and two sides
  Equal. Title: **Constraints**. Sits after Chamfer and before Combine.
- **Combine** (`combine`, #1556): place a cuboid and an overlapping sphere, switch Combine to
  Cut, pick the cube then the sphere, Enter. Title: **Combine**.
- **Raised text** (`raised_text`, #1557): place a cuboid, sketch on its top, drop text, type
  `BEAR`, extrude the letters so they stand proud. Title: **Raised text**.
  3D Bodies, Combine, and Raised text all start with a cuboid: if the Shape tool's last-used
  kind isn't cuboid, a step points at Cuboid in the Context pane (#1569).
- **Numbered list** (#1558): the Tutorials pane prefixes each walkthrough with its catalog
  number (`1. Sketch & Extrude`, …). Scripting exposes the same 1-based `number` on
  `bearcad.ui.tutorials()`.
- **Second tutorial (`navigate`)** (#1269): pan / orbit / zoom, Zoom to Fit (toolbar
  button + `Z`, #1583), and the view-bear HUD (snap a view + Home). Starts with a few
  cubes already in the document; camera steps auto-advance on motion. Orbit, pan, zoom,
  Zoom to Fit, and Home have no "…for me" assist — the user does those. After orbit and
  after pan, a Next-only "Good job" step makes the next action obvious. The bear-snap
  step still offers an assist. Orbs can point at `UiAnchor::ViewCube` / `ViewHome` /
  `ZoomToFit`. No Selection Exploder step (#1330): its tooltip covered the loupes.
- Each tutorial is a list of **steps**: Bear's narration in a cartoon **speech
  bubble** tucked under the view cube (with a tail pointing at Bear), a **pulsing
  gold ring** on what to click next (toolbar buttons, the Parameters `+`, or a projected
  viewport point — anchors recorded per frame in `AppState::tutorial_anchor_rects`), and
  either a **done predicate** on the app state (the step auto-advances the moment any
  action satisfies it — worked-ahead users skip ahead, `AppState::advance_tutorial`) or a
  manual **Next** button, and a **Back** button reviews earlier steps (auto-advance
  stands down while reviewing and resumes when Next reaches unfinished work,
  `TutorialRun::hold`). The bubble's buttons sit where they lead: **Back** on the left,
  **Next/Finish** on the right, and a **✕ in the title row's top right** (the bundled
  `IconId::Close` SVG, never a font glyph) ends the tutorial any time (#756). Narration is
  drawn as a `LayoutJob`: runs wrapped in **backticks** — parameter names, values, the
  exact letters to type — come out **monospace in their own blue** so they stand out from
  the prose (`tutorial::narration_spans`, #757). The ring is a **pulsing blue
  orb that always glides** between anchors — never teleports when the target
  changes, including while the camera is still zooming (#1346). The tooltip
  waits until zoom lands (#1332). A step that wants a **drag**
  rather than a click names the
  button in the pointer badge below and animates it (`Step::drag_hint`, #819/#882) — a
  pointer blown sideways by cartoon wind gusts under the ring. Anything to be **held with the click** — the Shift of a second pick, the right button
  of a drag — reads as the orb's own select-arrow, a `+`, and the thing to hold, in big bold
  blue on a pill sitting **clear of the ring** (#759/#851/#877/#882,
  `Step::needs_shift`/`Step::drag_hint`, `draw_orb_pointer_combo`), above it when there's
  room and below it otherwise, so it never covers the spot being pointed at. Steps whose target sits **under other geometry**
  say **"Press space if it's too crowded to
  pick"** in the same blue just above the ring (`Step::key_hint`, #777/#853), which is how the
  navigate tutorial introduces the Selection Exploder. None of them is a keycap: a key-shaped badge
  reads as something to click, and these are things to hold or press on the keyboard. A step whose work
  is a short **sequence** — click this, Shift+click that, press the button — shows **every mark
  at once, numbered** (`Step::marks`, `tutorial::GuideMark`, #854): the active one is the orb
  with its pointer, the rest are quieter rings, and each turns **green** as its part lands, so
  the whole move is visible from the start. A step can also name the **words to type**
  (`Step::type_hint`, #778/#781/#848). Once the field **the orb marks** has the keyboard the
  **guide itself becomes the instruction** (`typing_guide_takes_over`, #874): the ring gives way to a single box just above the field reading
  *Use the keyboard to type* in white with the words in the monospace blue the narration gives
  code (`draw_orb_typing_guide`). There's nothing left to click once you're in the box, so the
  click guide steps aside rather than sitting there looking like a button.
  The box hangs off the **focused field's own rect** (`typing_guide_rect`, #868) — just above it,
  or below when the top of the window is in the way — so it never covers the floating dimension
  and diameter inputs, which open right where the orb is.
  It's either fixed text or **computed from the state** (`TypeHint::Dynamic`), and a step holds
  its words back until the field that takes them exists (#786/#787/#789): a dimension's
  value input after it's placed. The extrude
  step's orb likewise moves from the toolbar button to the **profile face** once the tool is
  up, so the face to click is shown rather than described (#790).
  All three badges are bounded by the **window**, not the viewport, since an orb can be
  pointing into a side pane. In the dimensioning steps the orb moves **off** the
  line once it's picked, onto the spot where the dimension will drop (#779) — the same side
  the committed label takes, so "click away from the line" is shown rather than described —
  and then onto the **value field** it opened (`UiAnchor::DimensionValue`, the floating input
  recording its own rect, #814), where the typing goes. The extrude steps do the same with
  the tool's floating **distance** field (`UiAnchor::ExtrudeDistance`, #816). Steps can
  carry an **`on_enter` hook** that runs once when the tutorial lands on them going
  forward (never while reviewing with Back) — and again when a sketch opens (#875), since a
  sketch's own entry transition aims at the plane's origin and would throw the step's framing
  away.
  On **phone-width layouts** the default spot (a narration step, no orb) is along the
  **bottom** of the viewport, above the status bar, with **no tail** — nothing to point at,
  and the top of a phone screen is where the model is (#827). Otherwise the
  bubble **follows the orb** (#825): below it by preference, above it when the bottom of the
  window is in the way, else to whichever side has room, with its tail on the edge facing the
  orb — so what to read and where to look are the same place, and the orb's glide carries the
  bubble along. With no orb (narration-only steps, including the **first step**) it
  hangs off the **left side** of the view-cube **HUD panel** (the cube rect grown by
  `view_cube::HUD_PANEL_PAD`, so the gear/home buttons count too), tail pointing right at
  Bear — the words come out of the bear's mouth (#1577). A leftover orb from an earlier
  step does not count; under the HUD the bubble covered the Context pane controls a step
  was pointing at (#760). It's positioned from the bubble's **measured** width (frame
  margins make it wider than its content) plus the tail and a gap, so it never clips the
  HUD (#767, `tutorial_bubble_layout`).
- **Phone steps (#828):** on the compact layout the side panes are floating windows toggled
  from the status bar. A step can be `only_on_phone`: its predicate is already satisfied on a
  desktop (where the panes are docked), so it auto-advances and only ever shows on a phone;
  its orb can point at the status-bar toggle (`UiAnchor::PaneButton`). Steps whose wording
  assumes a desktop can carry
  `Step::phone_narration`, used when `AppState::compact_layout` is set (mirrored each frame
  from `touch::compact`).
- The bubble's header is just **"Step N of M"** (#847) — the tutorial's own name is on the
  button that started it, and repeating it on every step is noise.
- **Steps that need the keyboard offer to do themselves** (#810/#843): a **"do it for me"
  button** in the bubble (`tutorial::StepAssist { label, run: fn(&mut AppState) }`, applied by
  `Action::TutorialAssist`) makes the same document changes the user's typing would, so
  the step's own predicate advances the tutorial exactly as if they'd done it. Steps where
  **clicking the thing the orb points at is the whole job** carry no button (#843): tool
  buttons, pane taps, tapping into a field, and clicking a face. An assist never clobbers
  work already done, and a step whose work exists is a no-op.
- **Linkable (#765):** the web build reads **`?tutorial=<name>`** from the page URL at boot
  (`tutorial::tutorial_from_query`) and starts that walkthrough — `…/app/?tutorial=cube`.
  The desktop twin is
  **`bearcad --tutorial <name>`** (`ScriptOptions::tutorial`). An unknown name just opens
  the app normally.
- **`?open=<url>`**: the web build also fetches a document URL (percent-decoded,
  `main::open_url_from_query`) and opens it at boot through the same queue the browser
  open dialog feeds — so a docs page can link a screenshot straight into the live model.
  **Every model screenshot on the tool pages is such a link**: each screenshot script saves
  its scene as `<name>.bearcad.json` beside the PNG, and the page wraps the image in an
  anchor to `/app/?open=<that url>`. The convention is stated once on the Modeling Tools
  index rather than captioned under each picture. Annotated `pane-*` shots are excluded —
  their subject is the callouts, not the model. The document is the web JSON codec
  (`storage::to_json_bytes`); `bearcad.save("….json")` writes it, which is how a
  screenshot scene publishes the model beside its PNG. A failed fetch lands as a status
  line, not a broken app.
- Scriptable: `bearcad.ui.tutorial("cube")`, `bearcad.ui.tutorial_next()`,
  `bearcad.ui.tutorial_assist()` (press the current step's "do it for me" button),
  `bearcad.ui.tutorial_end()`, `bearcad.ui.tutorial_step()` (current step index or nil),
  `bearcad.ui.tutorial_narration()` (current step text, or nil),
  `bearcad.ui.tutorial_orb()` (`{x, y}` window px of the guide ring, or nil; pass it to `click`),
  `bearcad.ui.tutorial_bubble()` (`{x, y, w, h}` speech-bubble screen rect, or nil).
  Prompting (#1434): `bearcad.ui.complete_all_tutorials()`, `bearcad.ui.unstart_all_tutorials()`,
  `bearcad.ui.install_age([days|false])`,
  `bearcad.ui.tutorial_highlight()`, `bearcad.ui.tutorial_prompt()` / `"launch"` / `"work"` /
  `"tick"` / `"dismiss"`, `bearcad.ui.complete_tutorial(name)`.

### 11.x Auto-update (#427)
- **Build identity (#460, #1129):** `build.rs` bakes `git describe --tags` and the short
  SHA into the binary. `full_version()` reports the release tag verbatim when built from
  an exactly-tagged checkout (`v0.1.0-build.<short-sha>`), else `v0.1.0 (<sha>)`; the About
  dialog (native menu metadata and the in-app Help → About status line, web included)
  shows it. Release build numbers are the abbreviated commit SHA of the released commit
  (`vX.Y.Z-build.<short-sha>`; #1788). The update check compares the latest release tag against
  the baked describe — so a release build knows its own build number and never offers
  itself as an update. A **dev build** (debug assertions, or a describe carrying commits
  past the tag — `updater::is_dev_build`) is treated as **ahead of every release**: it
  holds unreleased work, so the check never reports an update and no badge appears,
  however far the published build number has marched on (#764).

Native builds check GitHub once at startup in a background thread
(`updater::spawn_check`, ureq/rustls against the releases API; the
check is best-effort and silent on failure; `BEARCAD_NO_UPDATE_CHECK` disables it, and
the doc-screenshot harness sets it). The **update channel** setting (#1288,
`AppSettings::update_channel`, default **Release**) chooses which stream is watched:
**Release** hits `/releases/latest` (published non-prerelease only); **Pre-release**
lists recent releases and takes the newest non-draft (prereleases included). Changing the
channel in Settings re-runs the check. When a strictly newer version exists
(`updater::is_newer`, dotted numeric compare), a **bright green badge** appears in the
status bar's bottom-right corner — no popup, no interruption. Clicking it stages the
update **in place on every desktop OS**, downloading the artifact for the reported tag
(so a pre-release install does not silently pull `/latest`): **Windows** (bare exe
artifact) and **Linux** (tar.gz) download to a temp dir and swap the running executable
via the rename trick (old binary moves to `.old`); **macOS** (.dmg) uses the Squirrel.Mac
trick — a running `.app` bundle can be renamed — so it mounts the dmg (`hdiutil attach`),
`ditto`-copies the new bundle beside the installed one (same volume, so the final rename
is atomic), and rename-swaps (`BearCAD-old.app` aside; roll back on failure; dmg detached
either way). Once staged the badge becomes a **⟳ Restart BearCAD** button
(`updater::restart_into`: `open -n` for a bundle, plain spawn otherwise, then exit).
Leftover `.old` binaries/bundles are cleaned on the next startup. Fallbacks: a
non-bundle macOS run (dev build) auto-downloads the artifact in the browser; a failed
stage rolls back and opens the release page for that version.

### 11.x2 Auto-zoom (#438)

A toolbar **toggle** beside Zoom-to-fit (`AppState::auto_zoom`, off by default;
`bearcad.ui.auto_zoom(bool)`). While on and a rectangle or extrusion is in progress,
each frame checks the **live bounds** (document ∪ in-progress rect corners ∪ extrusion
profile swept by its live distance) and how they sit in the view (any corner off-screen /
behind the camera; whole thing < ⅓ of the viewport — `auto_zoom_screen_state`). Framing is
**direction- and intent-gated** (#463): the camera zooms **out** only when the bounds
actually *grew* off-screen, and zooms **in** only when they actually *shrank* under-⅓
**and** the size is deliberate — a typed rect dimension or an in-progress extrusion —
so mouse-dragging a fresh small rectangle never yanks the camera in. Growth/shrinkage is
tracked frame-to-frame via the bounds diagonal (±2 % hysteresis, reset when nothing is
live). When it fires, the camera **glides** to frame the bounds
(`Camera::frame_bounds_animated`, 0.22 s, same destination math as the instant
zoom-to-fit; orientation untouched). Triggers only between animations (never fights an
in-flight glide) and stands down in FPS mode and the Drawing workbench. Decision logic is
the pure `auto_zoom_should_frame` (unit-tested).

With **no live preview** in progress, the same watch runs on the **committed document
bounds** (#624): a commit that lands geometry off-screen — e.g. a 20 m extrusion
confirmed straight from the context pane before any preview tick ran — still glides the
view out to fit, and an undo that shrinks the model well inside glides back in.
Committed geometry always counts as deliberately sized; growth/shrinkage is tracked via
a separate document-bounds diagonal so panning away never snaps the camera back.

**Selection watch:** the moment the selection *changes* to something that pokes
off-screen — e.g. a body face picked while half of it sits outside the view — the camera
glides out to take the whole selection in (`selection_world_bounds`, which resolves a
selected body face to its full coplanar triangle group via `body_face_triangles`, not
just its stored centroid). Framing is **zoom-out only**
(`Camera::frame_bounds_zoom_out_animated`: the destination distance never drops below
the current one), so picking something small pans over without diving in, and a fully
visible selection never moves the camera at all. "Changed" is judged by an
order-independent fingerprint of the selection set (`scene_selection_fingerprint`;
empty → never frames), so orbiting or panning away from a still-selected face doesn't
snap the view back.

**Stands down for a joint drag (#905):** while a part is being dragged through its joint
the whole watch is skipped (its growth baselines and selection fingerprint are refreshed
instead), so the camera can't swing out from under the drag or snap when it lands.

### 11.x Help mode (#672)

**Directive:** A beginner should be able to ask the app what a control is for, without leaving
it.

- Help mode is a toggle, off by default, reached from the command palette (*Turn On Help Mode*
  / *Turn Off Help Mode*), from **Help → Help Mode** in the OS menu (a checked item with the
  **Cmd/Ctrl+/** accelerator — the "?" binding without also pressing Shift; platforms without
  the native menu handle the key in the egui layer), and from scripts as
  `bearcad.ui.help([on])` (no argument toggles). It is session state, never persisted.
- With it on, every row of the Context pane that has help text grows a floating note beside
  the pane — outside it, so the pane itself stays controls and values only (§the context pane's
  no-prose rule) — joined to its row by a leader line. Notes that would overlap slide apart.
- Every toolbar tool that has a keyboard shortcut also grows a small badge with that
  key (e.g. **B** under Shape), from the same `tool_shortcut()` table the Keyboard
  Shortcuts window uses. Scripts read the badges as `bearcad.ui.toolbar_shortcuts()`.
  The tools on the bar (and the only ones a letter key will arm) are
  `bearcad.ui.toolbar_tools()`.
- The help text is per (tool, row label), so the same label reads correctly under different
  tools ("Bodies" means one thing to Move and another to Combine). Rows that mean the same
  thing everywhere (default units, snapping) are matched on the label alone.
- A scripted Context-pane capture widens to include the notes, which is how the documentation's
  annotated pane pictures are made (§9.3).

### 11.x Viewport tool hints (#1509)

The usage line painted at the bottom of the 3D viewport (tool-specific: click to select,
orbit/zoom, Enter to commit, …). On by default, session-only.

- **View → Tool Hints** is a checked OS-menu item (web: View → Tool hints). The command
  palette offers *Hide Tool Hints* / *Show Tool Hints*. Scripts: `bearcad.ui.tool_hints([on])`
  (no argument toggles).
- A pending scripted screenshot suppresses the line for that frame, the same way a
  viewport shot hides the view bear. Docs scripts also call `bearcad.ui.tool_hints(false)`
  so window shots stay clean.

### 11.y Keyboard Shortcuts window (#434)

**View → Keyboard Shortcuts** / **Help → Keyboard Shortcuts** (and the palette entry
"Keyboard Shortcuts") opens a closable window listing **every** binding in the app,
grouped by scope: Everywhere, Tools (3D modeling workbench), Sketch mode, Constraints
(Constraint tool), Expression fields, First-person mode (experimental), and Technical drawings —
sections whose shortcuts only apply in a certain state carry a scope note.

The single source is **`shortcuts::all_shortcuts()`**. Maintenance contract: any new or
changed key binding MUST be reflected there in the same change. Two sections are derived
so they cannot go stale — Tools from `tool_shortcut()` (the same table the toolbar
labels use) and Constraints from `GeometricConstraintType::ALL` — with tests
(`shortcut_list_covers_every_tool_shortcut`,
`shortcut_list_covers_every_constraint_mnemonic`) enforcing the coverage; everything
else is listed explicitly in that function.

### 11.y1 Changelog window (#1328)

**Help → Changelog** (and the palette entry "Changelog") opens a window with the changelog
baked into this build — the full `changelog.md` `changer bump` produced when the binary
was built. Scriptable as `bearcad.ui.changelog("show"|"hide"|"toggle")`;
`bearcad.ui.changelog("text")` returns the markdown.

GitHub draft releases take their version from `changer next-version` plus the abbreviated
commit SHA as the build number (`vX.Y.Z-build.<short-sha>`; #1788), and their notes from
`changer bump -n`. Publishing a draft runs `changer bump` so the repo CHANGELOG matches
that release, leaves later snippets for the next draft, tags the released commit
`vX.Y.Z`, and dispatches a website rebuild so the landing page shows the new version.

### 11.z Settings window & app settings store (#720)

**Cmd/Ctrl+comma** toggles the **Settings** window; it is also reachable from the
command palette ("Settings") and the native menu (app menu on macOS, File menu on
Windows). Like the Context pane, it shows controls and values only — what each row
means lives in help mode (#672), keyed on the row's label.

Settings are **per-machine app state**, not document state: JSON at the platform config
path (`~/Library/Application Support/BearCAD/settings.json` on macOS,
`%APPDATA%\BearCAD` on Windows, `$XDG_CONFIG_HOME`/`~/.config` on Linux), loaded at
startup, saved on change. A missing or malformed file silently means defaults. Every
field is `#[serde(default)]` so files round-trip across versions (`src/settings.rs`).

Settings:
- **Library directory** (`library_directory`) — the folder `Library(...)` unit-import
  sources resolve against (imported units, #719): **Choose…** (folder picker) sets it,
  **✕** clears it.
- **Animate zoom to fit** (`animate_zoom_to_fit`, default on) — Zoom to Fit glides at half
  Home duration; off snaps instantly (#1276/#1303).
- **Update channel** (`update_channel`, default **Release**) — **Release** or
  **Pre-release**; the auto-updater only checks the chosen stream (#1288). Scriptable as
  `bearcad.ui.update_channel("release"|"pre_release")` (no arg returns the current).

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
4. ~~Full assembly joint catalog (§2.3).~~ **Resolved:** the Joint tool (§3.3, #891) —
   eight kinds, limits, rest poses, and drag-through-joint; path/cam/gear/belt couplings
   remain future work.
5. OCCT binding strategy and the exact C++ shim surface (§10).
6. Lua API module layout and function signatures (§8).
7. Per-feature `payload` encoding in the SQLite schema (§7.3).
8. GD&T symbol coverage and standard for technical drawings (§12.2).
9. DXF/SVG/PDF writer library selection and licensing for drawing export (§12.3–12.4).
10. Geometry cache granularity — per-feature (floor) vs. per-body and/or tessellation-LOD
    entries, and the BREP/mesh blob encoding (§4.4, §7.3).
