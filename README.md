# BearCAD 

<p align="left">
  <img src="src/assets/appicon.png" alt="BearCAD app icon" width="128" height="128">
</p>

Local-first, parametric CAD. Built by robots to see what AI can do.

[Docs](https://iffy.github.io/BearCAD/)

## Download

| Platform | Download |
|----------|----------|
| macOS (Apple Silicon) | [bearcad.dmg](https://github.com/iffy/BearCAD/releases/latest/download/bearcad.dmg) |
| Windows (x86_64) | [bearcad.exe](https://github.com/iffy/BearCAD/releases/latest/download/bearcad.exe) |
| Linux (x86_64) | [bearcad-linux-x86_64.tar.gz](https://github.com/iffy/BearCAD/releases/latest/download/bearcad-linux-x86_64.tar.gz) |

## Run

```sh
cargo run
```

- Pick a face with the **Sketch** tool (or start on the default XY construction plane),
  then draw with **Rectangle**, **Line**, or **Circle**.
- Type dimensions while drawing; **Tab** cycles fields; **Enter** commits.
- **Right-drag** to orbit; **Shift+right-drag** to pan; **mouse wheel** to zoom.
- **Escape** cancels an in-progress draw; press again to exit sketch mode or return to
  Select.
- **Save / Save As…** writes a `.bearcad` SQLite file; **Open…** loads one back.
- **Clear** resets the document; **Undo last** reverts the most recent action as a whole
  (e.g. an entire rectangle — its lines and constraints — in one step).

```sh
cargo run -- --help    # usage and exit
cargo test
```

## Building with the OCCT kernel

BearCAD's real BREP geometry kernel is [OpenCASCADE (OCCT)](https://dev.opencascade.org/),
behind the **`occt`** Cargo feature, which is **on by default** — solid
booleans/cut, true BREP fillets/chamfers, and curved-surface STEP all come from the
kernel. So the default `cargo build` / `cargo run` needs a C++ toolchain and a built
OCCT; set that up once:

```sh
# 1. Fetch the pinned OCCT source (once):
git submodule update --init --depth 1 third_party/OCCT

# 2. Build OCCT as static libraries (needs cmake + a C++17 compiler; takes a while):
scripts/build-occt.sh

# 3. Build/run BearCAD (the default build links the kernel):
cargo run
```

`scripts/build-occt.sh` builds the modeling toolkits plus DataExchange (for STEP
read/write) — no visualization, application-framework, or Draw modules — into
`third_party/OCCT/occt-install`, which `build.rs` statically links against.

### Building without the kernel

To build the lean fallback — **no C++ toolchain, no OCCT** — disable the default
feature:

```sh
cargo run --no-default-features
```

This is what the Windows release and the fast CI check build. The kernel-only
features fall back to hand-rolled mesh geometry (or are hidden, e.g. extrude Cut),
but the app is otherwise fully functional.

### Recompiling against a different OCCT version

BearCAD links OCCT **statically**. The LGPL 2.1 permits this on the condition that
you can relink the app against a different (e.g. modified or newer) OCCT. To do
so, point the **`OCCT_DIR`** environment variable at any OCCT install prefix — one
containing `include/opencascade/*.hxx` and `lib/libTK*.a` — and rebuild:

```sh
OCCT_DIR=/path/to/your/occt-install cargo build
```

When `OCCT_DIR` is set it takes precedence over the bundled submodule build, so
you can swap in your own OCCT (from Homebrew, a distro package, or a custom build)
without touching BearCAD's source. See
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) for the full licensing story.

### Kernel in CI and releases

CI (`.github/workflows/ci.yml`) has a dedicated `occt` job that builds OCCT once
(cached on the pinned submodule + build script, so it's restored rather than
rebuilt on later runs) and runs the kernel test suite, so kernel regressions are
caught on every push/PR. The `ci` job separately builds/tests the
`--no-default-features` (no-kernel fallback) configuration — fast, no OCCT — so
both code paths stay green.

The **macOS and Linux release binaries ship with the kernel** (the default build).
**Windows currently ships the no-kernel fallback build** (`--no-default-features`)
— a static OCCT/MSVC build is being scaffolded via the experimental, non-blocking
`windows-occt` CI job (see issue #96), so on Windows the
kernel-only features (real BREP fillets/chamfers, solid booleans/cut,
curved-surface STEP) fall back to hand-rolled mesh geometry (or are hidden) until
that lands.

## Scripting

Scripts are **Lua** files (`.lua`) that call the global `bearcad` API — the same actions and
synthetic input as the GUI, useful for automation and regression tests. Full docs, including
the complete API reference, are at **[iffy.github.io/BearCAD/docs/scripting](https://iffy.github.io/BearCAD/docs/scripting)**.

**Run a script:**

```sh
cargo run -- --script examples/rectangle.lua --exit
```

```lua
-- examples/rectangle.lua
bearcad.new()
bearcad.rect{ width = 80, height = 50, name = "Main box" }
```

**Interactive REPL** — drive the live app from your terminal, entry by entry (the GUI stays
fully usable while it runs):

```sh
cargo run -- --repl
```

```
bearcad> x = 15
bearcad> bearcad.rect{ width = x * 2, height = x }
bearcad> 1 + 2
3
bearcad> bearcad.save("drawing.bearcad")
```