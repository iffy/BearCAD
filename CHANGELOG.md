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
