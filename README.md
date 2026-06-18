# LE3 — Local CAD

On-device parametric CAD. See [SPEC.md](SPEC.md) for the full design.

## Status

Very early prototype. Currently implemented:

- An egui GUI with a 2D sketch viewport.
- Draw **rectangles** by dragging in the viewport.
- **Save / Open** documents as `.le3` files (SQLite, per SPEC §7).

Not yet implemented: the 3D viewport, OCCT kernel, action DAG, parameters,
constraints, scripting, and everything else in the spec.

## Run

```sh
cargo run
```

- **Drag** in the viewport to create a rectangle.
- **Save / Save As…** writes a `.le3` SQLite file.
- **Open…** loads one back.
- **Clear** removes all rectangles; **Undo last** drops the most recent.

## Test

```sh
cargo test
```
