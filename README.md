# LE3 — Local CAD

On-device parametric CAD. See [SPEC.md](SPEC.md) for the full design.

## Status

Very early prototype. Currently implemented:

- An egui GUI with a 3D viewport (orbit camera, projected with egui's painter).
- A **Rectangle** tool: draw rectangles on the ground plane (XY, z = 0).
- **Save / Open** documents as `.le3` files (SQLite, per SPEC §7).

Not yet implemented: the 3D viewport, OCCT kernel, action DAG, parameters,
constraints, scripting, and everything else in the spec.

## Run

```sh
cargo run
```

- Select the **Rectangle** tool, then **left-drag** on the ground plane to draw.
- **Right-drag** to orbit; **Shift+right-drag** to pan; **mouse wheel** to zoom.
- **Escape** cancels an in-progress draw; press again to return to the Select tool.
- **Save / Save As…** writes a `.le3` SQLite file; **Open…** loads one back.
- **Clear** removes all rectangles; **Undo last** drops the most recent.

## Test

```sh
cargo test
```
