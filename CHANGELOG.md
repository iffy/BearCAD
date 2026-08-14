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
