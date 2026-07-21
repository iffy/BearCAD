# BearCAD 

<p align="left">
  <img src="src/assets/appicon.png" alt="BearCAD app icon" width="128" height="128">
</p>

Local-first, parametric CAD. Built by robots to see what AI can do.

Solid geometry by [OpenCASCADE (OCCT)](https://dev.opencascade.org/), the industry-grade
BREP kernel; sketch constraints solved by [SolveSpace](https://solvespace.com/)'s solver
(libslvs) — on desktop and in the browser alike.

[Docs](https://iffy.github.io/BearCAD/)

## Download

Or skip the download: **[run BearCAD in your browser](https://www.iffycan.com/BearCAD/app/)**
(full geometry engine and constraint solver; documents save as downloads).

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
kernel. So the default `cargo build` / `cargo run` needs a built OCCT; set that up
once:

```sh
# 1. Install OCCT (downloads a prebuilt when one matches; Windows: scripts/build-occt.ps1):
scripts/build-occt.sh

# 2. Build/run BearCAD (the default build links the kernel):
cargo run
```

`scripts/build-occt.sh` first tries the **prebuilt** static libraries published by
the `occt-prebuilt` workflow — keyed to the pinned OCCT submodule commit and the
build script itself, checksum-verified — so a fresh clone is building BearCAD in
minutes with no OCCT compile. When no prebuilt matches your platform (or with
`BEARCAD_OCCT_FROM_SOURCE=1`), it compiles from source instead: fetch the
submodule (`git submodule update --init --depth 1 third_party/OCCT`) and have
cmake + a C++17 toolchain on PATH. Either way the result lands in
`third_party/OCCT/occt-install` — the modeling toolkits plus DataExchange (for
STEP read/write), no visualization/application-framework/Draw — which `build.rs`
statically links against.

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

CI (`.github/workflows/ci.yml`) has dedicated `occt` (Linux) and `windows-occt`
(Windows/MSVC) jobs that build OCCT once (cached on the pinned submodule + build
script, so it's restored rather than rebuilt on later runs) and run the full test
suite — plus the launch smoke, example scripts, and interaction tests — against
the kernel build, so kernel regressions are caught on every push/PR.

**All release binaries — macOS, Linux, and Windows — ship with the kernel** (the
default build), so real BREP fillets/chamfers, solid booleans/cut, and
curved-surface STEP work the same on every platform.

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