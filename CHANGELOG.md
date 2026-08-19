# v0.5.1 - 2026-08-19

- **FIX:** Shape Enter now advances through each dimension instead of creating a body on the first press, Extrude's Up to picker disarms after a pick, and a line-and-point construction plane stays through the point.
- **FIX:** Chamfer and Fillet now highlight and pick only the remaining visible edges of a sliced or cut body, not the original body's vanished edges.
- **FIX:** macOS no longer aborts at launch when attaching Finder file-open hooks.

# v0.5.0 - 2026-08-18

- **NEW:** The rotational Repeat Angle field now toggles where the last copy sits, so 360° and 5 items can space equally instead of stacking the last on the first.
- **NEW:** View → Tool Hints hides the usage line at the bottom of the viewport, and scripts can toggle it with bearcad.ui.tool_hints.
- **NEW:** Unfinished tutorials light the Tutorials button bright blue, Skip all tutorials turns that prompting off, and a fresh install shows a Want to try some tutorials? tip for the first 30 days that fades once you start modeling.
- **NEW:** The macOS disk image opens with a honey-toned background that shows you to drag BearCAD into Applications.
- **NEW:** In realistic shading, bodies now cast shadows on each other and themselves, not only on the ground.
- **NEW:** On the web, phones, tablets, and small screens show a banner that BearCAD doesn't work very well there.
- **FIX:** Double-clicking a sketch repeat, slice, or chamfer/fillet in the Elements pane reopens it for editing.
- **FIX:** Value gizmos now click-to-stick: the second click releases the handle, and Enter or the context button commits.
- **FIX:** Face Snap hover preview now sits the moving face against the fixed face, matching the placement a click commits.
- **FIX:** Letter shortcuts only arm a tool the current workbench toolbar would show, so E, R, and L stay on drawing tools while a drawing is open.
- **FIX:** Lua export files a loft into a component with move_to_component.
- **FIX:** Lua export can put a loft or drawing in a component via move_to_component{ kind = "loft"|"drawing" }.
- **FIX:** After a successful commit, Revolve, Sweep, Loft, and Shape stay on the tool with an empty draft, the same as Extrude.
- **FIX:** Sketch Mirror and Sketch Slice now commit with the same blue confirm button as Offset and Repeat, and Combine's button is labeled Combine rather than Create.
- **FIX:** Sketches and extrudes drawn on a moved or operated-on body stay in the Elements pane after later features consume that body.
- **FIX:** Free Move gizmos on a small object now keep a minimum viewport-pixel spacing from each other instead of shrinking into a clump (#1478).
- **FIX:** Zoom to Fit frames every live tool ghost, including slice, shell, shape, joint, 3D chamfer/fillet, and sketch offset/mirror/repeat/slice.
- **FIX:** Lua export writes sketch geometry before the ops that consume it, so mirror, fillet, offset, and repeat scripts replay.
- **FIX:** Last-used tool options — Output mode, Extrude and Revolve symmetric, Combine kind, Joint kind, chamfer and fillet amount, Offset, Shell, and Repeat — are remembered for the session.
- **FIX:** The rotation gizmo's fading tail stays on the original position line and disappears once the turn is wider than the tail (#1433).
- **FIX:** Realistic mode no longer paints a hole-cavity shadow onto the outside of a cuboid or cylinder.
- **FIX:** Lua export keeps rectangle edge names, drawing titles, projected lines, components, and body shadow flags.
- **FIX:** body_mesh_face treats face as world millimetres from body_faces and centroid as already-quantized integers, so a reported face can be fed back without hand-quantizing.
- **FIX:** The Move inspector packs each mode's fields at the top instead of leaving empty gaps for hidden rows.
- **FIX:** Fillet and Chamfer no longer pick an edge through a body, and a moved body's edges pick the same way any other body's do.
- **FIX:** On the web, Shift+right-drag pans the camera instead of opening the browser context menu.
- **FIX:** Sketch mirrors nest under their sketch, so filtering out sketch components hides the mirror and its children.
- **FIX:** Clicking a material's colour swatch in the Context pane dropdown selects that material, the same as clicking its name.
- **FIX:** Extrude, Revolve, Sweep, Loft and Mirror Output (new / add / cut) is scriptable as bearcad.ui.tool_mode.
- **FIX:** Repeating a standalone extrude makes a separate body for each instance, and each instance can take its own material.
- **FIX:** Dragging the Face Snap Turn gizmo a short way now stores the short signed angle, so a clockwise pull reads as −61° instead of wrapping to 299° (#1432).
- **FIX:** The 2D Mirror tool accepts the sketch origin axes and in-plane world axes as a mirror line.
- **FIX:** Extrude, Revolve, and Sweep arm their options as soon as you pick the tool, so Y and the Output row work before the first pick.
- **FIX:** The Y shortcut now cycles the Combine tool's mode (combine / cut / intersect / difference), the same way it cycles Extrude output.
- **FIX:** Loft can start from a bare body face by building an implicit sketch, matching Extrude, Revolve, and Sweep.
- **FIX:** Cylinders sitting on the ground no longer shimmer where they meet the floor in realistic shading.
- **FIX:** Sketch Offset and Repeat gizmos no longer overwrite a value you have already typed.
- **FIX:** Hovering and orbiting no longer hitch on models with sketches on moved or fused bodies.
- **FIX:** Add to body and Cut now shadow the host and produce a new body, the same way Merge already did.
- **FIX:** Switching the Move tool's mode no longer flashes its input fields red.
- **FIX:** Cut-extruding through a moved body keeps the body, and Extrude's Up to picker now accepts any 3D body face.
- **FIX:** Tool switching, Enter-to-commit, Esc, and picker focus now follow one table of per-tool rules, so a new tool cannot skip those behaviors.
- **FIX:** The Parameters pane no longer leaves a large empty gutter to the left of the Name column.
- **FIX:** Realistic-mode body shadows use a directional shadow map, so light-facing surfaces stay lit and holes no longer paint onto convex exteriors.
- **FIX:** Lua export no longer double-applies a construction plane's offset when the script is replayed.
- **FIX:** Realistic-mode self-shadows no longer speckle, soften like sunlight, and put the shade side of a hole in shadow.
- **FIX:** Esc after committing a Shape or any other empty 3D tool returns to Select on the first press.
- **FIX:** Ground-plane shadows no longer show when the camera looks up from below the plane.
- **FIX:** Dragging a Move push/pull gizmo snaps values to 0.1 of the document unit so the typed value and computed subtitle no longer flicker.
- **FIX:** On the Combine tool, changing the mode after Side A is filled focuses the Side B picker.
- **FIX:** Chamfer and Fillet now toggle vertices and edges on click in both 2D and 3D, and the HUD describes the dual-mode pick.
- **FIX:** The Dimension tool can now measure how far a circle sits from the sketch origin, so a hole on a circular face can be located from the centre.
- **FIX:** Clicking to place a dimension label no longer highlights or selects the face under the cursor
- **FIX:** fillet_vertex and chamfer_vertex accept points = { ... } so one call is one multi-corner operation, and Lua export emits that form.
- **FIX:** On macOS, Command+` cycles through every app window, including the McMaster-Carr catalog.
- **FIX:** Switching tools now carries the current picks into the new tool's primary picker for every tool, including Extrude, Revolve, Sweep, Chamfer, Fillet and Loft.
- **FIX:** Clicking a hover-highlighted sketch face during Mirror now picks that face's edges as the shapes to reflect.
- **FIX:** Extrudes sketched on a moved body are real solids: laser-cut and shell commit on them the same as any other body.
- **FIX:** Extrude Add and Cut onto a Revolve, Sweep, or Loft result now fuse into that body instead of dropping the new extrusion.
- **FIX:** Loft operations can be selected, hidden, renamed, deleted, and picked in the Elements pane like other operations.
- **FIX:** Revolve angle, helical pitch, 3D fillet and chamfer amount, and construction-plane offset and angle now store the typed expression so they follow parameters and re-edit restores the original text.
- **FIX:** Lua export of a constrained sketch uses the coordinates you drew, so replaying the script lands on the same geometry.
- **FIX:** Which tools start a sketch from a face, leave a sketch, or stay inside one now comes from the tool table, so Constraint and Project click a face, Combine, Joint, and Shell leave, and Move, Mirror, Repeat, and Slice survive opening a sketch.
- **FIX:** Selecting the Repeat tool in a sketch now shows the Entities picker immediately so you can highlight and pick lines and circles.
- **FIX:** The Move tool no longer hover-highlights construction planes or offers them in the exploder.

# v0.4.0 - 2026-08-17

- **NEW:** Unit parameter values in the Parameters pane now commit when you press Tab, matching Enter, while letting variable-name autocomplete complete first (#1403).
- **NEW:** Repeated bodies (Repeat tool) are now named with incrementing numbers off their source body's name — 'Jim' yields 'Jim1', 'Jim2', …, and a base already ending in a number ('Jim1') yields 'Jim1-1', 'Jim1-2', ….
- **NEW:** macOS release builds are Developer ID signed, notarized, and stapled.
- **NEW:** Script calls can now hold the platform primary modifier with bearcad.ui.key("c", { cmd = true }), which the Copy/Paste shortcuts read as ⌘/Ctrl (the normal Command/Ctrl+C and +V keys)
- **NEW:** Right-clicking an imported unit in the Elements pane now offers 'Import another [name]', adding another instance of the same unit with identical parameter overrides (#1404).
- **NEW:** The Elements graph draws a dashed skip-edge from a hidden shadow's parent to the node that depended on that shadow.
- **NEW:** The Y shortcut now cycles the active tool's Output choice (new body / add to body / cut) on the Extrude, Revolve, Sweep, Loft, and Mirror tools (#1397).
- **NEW:** In the Parameters pane, the gear/options icon now sits to the left of the name and the delete button is pushed to the far right of each row, so the delete is harder to click by accident.
- **FIX:** Import Unit (File → Import → BearCAD…) now checks for an unsaved document before opening the file picker, prompting you to save first, instead of waiting until a file is already chosen (#1402).
- **FIX:** Rotation gizmos grab only at their handle discs (with a yellow hover ring) so dragging the red handle no longer turns the blue ring (#1418).
- **FIX:** Sketch lines selected in edit mode render in the depth-disabled Wireframe layer so extrusion geometry never occludes them (#1409); body edges and vertices of the sketch\u2019s host face are accepted by the InSketch pick filter, making them selectable and visible in the explosion selector (#1410, #1411).
- **FIX:** The Offset tool's default distance is now always 5 mm regardless of the document's default length unit, instead of scaling a bare '5' (e.g. 5 in = 127 mm) into an inches document (#1412).
- **FIX:** Rotation gizmo radials from the body centre are thinner, with the original position dashed and the live handle position solid (#1419).
- **FIX:** Rotation gizmo turns stay signed end to end — a -5° pull reads as -5°, never a wrapped 355° — across the ring labels, the typed rx/ry/rz fields, and face-spin turns (#1415).
- **FIX:** Snapping a sketch point to a body face's boundary edge now pins it to the edge line (a point-on-edge coincident) rather than to the face's nearest corner (a point-on-vertex coincident), so a sketch drawn beside a base cuboid follows the edge when the base's dimensions change (#1395).
- **FIX:** Face Snap move previews now use a yellow, axis-aligned spin gizmo and a smooth bezier connector that leaves both faces along their normals, even with no extra turn.
- **FIX:** Rotation gizmo handles float clear of the moved body: each of Free Move's three rings starts on its own fixed, non-overlapping spot around the selection so multiple gizmos never pile up (#1413).
- **FIX:** Rotation gizmo circles stay on the object's current equators after a multi-axis turn, instead of shrinking off-plane around a world axis (#1422).
- **FIX:** Free Move no longer hover-highlights or selects construction planes and other scene objects; only the rotation and translation gizmo handles are pickable.
- **FIX:** ValueInput bare numbers are now interpreted in the document's default length unit (e.g. 1.5 in an inches doc = 1.5 in) instead of always millimetres; repeat counts remain unit-independent
- **FIX:** Free Move's three rotation rings follow the preview: turning one ring swings the other handles along with the moving object's composed rotation instead of each staying on its own fixed axis (#1414).
- **FIX:** Min, max and step fields in the Parameters gear-options panel now use ValueInput widgets, accepting expressions, units, and computed previews like every other value input in the app (#1399 #1400).
- **FIX:** Imported STL bodies are now moveable in both free and point-snap modes (regression tests added)
- **FIX:** Switching Move translate modes no longer flashes inspector inputs red when rows remount (#1416).
- **FIX:** Free Move preview body now snaps to gizmo drags instead of easing toward them; typed inspector values still animate.
- **FIX:** Moving an imported unit now moves the instance itself (not its materialized bodies), so the geometry stays nested under the unit in the Elements pane instead of spawning a detached Moved output body; this also holds when a selected unit is handed into the Move tool and when re-editing a committed unit move.
- **FIX:** Pulling a rotation gizmo off its start drops the fade on the unused side, and a thin full circle of rotation appears only while the handle is held (#1420).
- **FIX:** Rotation gizmo direction arrows stand off from the handle as far as the Move tool's translation arrows (#1421).
- **FIX:** The Move tool's rotation gizmos now draw two fading 30° arcs out from the handle instead of a full circle, keep the handle floating on a deterministic reference with a direction arrow on each side of it, and paint the fade arcs underneath the live rotation sweep; a live turn stays signed (negative angles no longer read as 355°).
- **FIX:** Auto-zoom now frames both the original position and the previewed destination of a moving body, matching Zoom to Fit.
- The Move tool inspector labels the mode dropdown Move mode instead of Translate.
- Doubled the file-preview thumbnail resolution from 512 px to 1024 px (#1407)
- Speed up orbit/pan by keying the fully-constrained-lines memo on the document's integer mesh revision instead of re-serializing the whole sketch model to JSON every frame.
- Point all website payment actions at the Stripe payment link (landing-page action and downloads note) instead of the dashboard-configured buy button.

# v0.3.0 - 2026-08-14

- **BREAKING CHANGE:** Remove the Viewport-styles documentation page and its CI build process (style-swatch generator, screenshot scene, and workflow steps).
- **NEW:** Add a dedicated /docs/downloads page with just the per-OS download links, linkable from the site.
- **NEW:** Release CI now publishes a wasm web build as a release asset and the website CI downloads the latest published non-draft release instead of rebuilding the webapp on every docs push, cutting website build time.
- **NEW:** Add Help ▸ Report Problem…, which opens the user's browser at a new-issue form on the GitHub repo.
- **FIX:** Move: fix three drag/hover/display bugs (#1365, #1366, #1367)
- CI no longer rebuilds doc screenshots on every push: push/merge website builds fetch the existing screenshots from the deployed GitHub Pages site, and only the nightly re-captures them (and only when the repo changed since the last nightly) (#1389)
- Website payment verbiage now reads "Name Your Price": the landing-page action and the downloads-page note explain that BearCAD is free (pay nothing), and that supporters can name their own price via the existing Stripe button.
- Landing page now offers four main actions (Run in your browser, Read the docs, Pay to support, Download), with Pay and Download also mirrored into the top navbar next to the GitHub link.
- Auto-zoom now performs a zoom-to-fit 500 ms after the user stops interacting (mouse movement, clicks, or keyboard input), debounced so it never fires mid-gesture, and pauses while a drag is in progress.

# v0.2.0 - 2026-08-14

- **NEW:** Pathed documents write only changed rows in an open transaction; Save COMMITs
- **NEW:** .bearcad files are a typed SQLite schema: one table per entity, blobs for preview/fonts/meshes
- **NEW:** Persist tessellation in geometry_cache so files open without a full OCCT rebuild
- **NEW:** Imported units store the embedded copy as a nested .bearcad blob
- **NEW:** Add a Parameters tutorial: width in the pane, a width x width*2 rectangle, then extrude with inline height=30mm.
- **NEW:** Add a declarative bearcad.project API so the Project tool can be driven from scripts like the other modeling tools.
- **NEW:** Help → Changelog shows this build's changelog. GitHub releases take version and notes from changer bump; publishing a draft updates CHANGELOG and tags vX.Y.Z.
- **FIX:** Windows tests drop SQLite handles before deleting temp files
- **FIX:** Cut-extrude into a combined body applies, and further cuts work
- **FIX:** Tutorial dim-label tooltip waits for zoom, sits below the ring, and drops the arrow callout (#1332 #1333)
- **FIX:** Fillet/chamfer Shape-tool cuboid edges (and cylinder rims)
- **FIX:** Declarative rect/circle sizes can be changed with add_constraint or edit_dim (#1353)
- **FIX:** Clamp excessive extrude taper to 89° (and a 10 m size cap) with a ValueInput warning (#1352)
- **FIX:** Boolean-call test counter is per-thread so #1337 cut-preview tests survive cargo test parallelism
- **FIX:** Cut preview no longer rebuilds the target body every frame (#1337)
- **FIX:** wasm web app compiles; file association no longer calls the native installer (#1335)
- **FIX:** Drawing PDF/SVG dimension labels sit beside their lines like the editor (#1350)
- **FIX:** Move destination picks click through the moving body (#1336)
- **FIX:** Tutorial guide orb always glides to a new target instead of teleporting (#1346)
- **FIX:** Save no longer crashes when attaching the Finder preview icon (#1339)
- **FIX:** Empty boolean results (cut that leaves nothing, disjoint intersect) error instead of inserting a phantom body; an enclosed cut still creates a cavity
- **FIX:** Cylinder Height field sits below Radius (#1331)
- **FIX:** Remove the Selection Exploder from the navigate tutorial so its tooltip no longer covers the loupes
- **FIX:** Cut-extrude into moved/sliced/mirrored/repeated/filleted bodies applies
- **FIX:** mirror_bodies accepts plane = 0 as a construction-plane ordinal instead of a cryptic type error
- Remove the Build an angle bracket tutorial.
- Why page: drop editorial reminder; garish yes/no in Features table

# v0.1.0 - 2026-08-12

- Initial version. It kinda works :)
