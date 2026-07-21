fn main() {
    // Bake the build identity in (About dialog, update check): `git describe` names
    // release builds by their tag (v0.1.0-build.N); the short SHA identifies any build.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    // CI release builds pass the exact tag the release job will publish under
    // (BEARCAD_RELEASE_TAG, todoer #466): the tag doesn't exist yet at build time, and
    // the CI checkout is shallow (no tags), so `git describe` alone can't name it —
    // leaving the update check comparing against the bare crate version and offering
    // the very build being run.
    println!("cargo:rerun-if-env-changed=BEARCAD_RELEASE_TAG");
    let describe = std::env::var("BEARCAD_RELEASE_TAG")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["describe", "--tags", "--always"]))
        .unwrap_or_default();
    let sha = git(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=BEARCAD_GIT_DESCRIBE={describe}");
    println!("cargo:rustc-env=BEARCAD_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed=.git/HEAD");

    // OCCT kernel (#86, unconditional since todoer #471). On wasm32 the kernel
    // ships as a separate Emscripten-built module (see scripts/build-occt-wasm.sh)
    // — nothing links into the app binary there.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        build_occt_shim();
    }
    // libslvs (the sketch constraint solver) links into every native build; the wasm32
    // app instead reaches it inside the emscripten-built kernel module
    // (scripts/build-occt-wasm.sh + web/kernel-bridge.js).
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        build_slvs();
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon_path = std::path::Path::new("target/generated/appicon.ico");
    if let Some(parent) = icon_path.parent() {
        std::fs::create_dir_all(parent).expect("create generated icon directory");
    }
    png_to_ico("src/assets/appicon.png", icon_path);

    let icon_path = icon_path
        .to_str()
        .expect("generated icon path should be valid UTF-8");
    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_path);
    res.compile().expect("compile Windows icon resources");
}

/// Compile the C++ FFI shim (cpp/bearcad_kernel.cpp) and link it against a static
/// OpenCASCADE build (#86).
///
/// The OCCT install prefix is resolved from, in order:
///   1. the `OCCT_DIR` env var (point this at *your own* OCCT to rebuild against a
///      different version — see README.md), or
///   2. the default location produced by `scripts/build-occt.sh` (or
///      `scripts/build-occt.ps1` on Windows): `third_party/OCCT/occt-install`.
///
/// The prefix must contain `include/opencascade/*.hxx` and `lib/libTK*.a`
/// (`lib/TK*.lib` on Windows — `cargo:rustc-link-lib=static=TK...` resolves
/// either naming per platform).
fn build_occt_shim() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-changed=cpp/bearcad_kernel.cpp");
    println!("cargo:rerun-if-changed=cpp/bearcad_kernel.hpp");
    println!("cargo:rerun-if-env-changed=OCCT_DIR");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let occt_dir = std::env::var_os("OCCT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("third_party/OCCT/occt-install"));

    let include = occt_dir.join("include/opencascade");
    let libdir = occt_dir.join("lib");
    if !include.is_dir() || !libdir.is_dir() {
        panic!(
            "OCCT not found under {}\n(expected {} and {}).\n\
             Build it first: `scripts/build-occt.sh` (or `scripts/build-occt.ps1` on \
             Windows), or set OCCT_DIR to your own OCCT install prefix. See README.md \
             \"Building with the OCCT kernel\".",
            occt_dir.display(),
            include.display(),
            libdir.display(),
        );
    }

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("cpp/bearcad_kernel.cpp")
        .include("cpp")
        .include(&include)
        // Split every function/data item into its own section so the linker's
        // dead-code stripping (below) can drop the ones this binary never calls.
        // GCC/Clang spell it -ffunction-sections/-fdata-sections; MSVC spells it
        // /Gy. flag_if_supported drops whichever the active compiler rejects.
        .flag_if_supported("-ffunction-sections")
        .flag_if_supported("-fdata-sections")
        .flag_if_supported("/Gy")
        // The shim catches OCCT C++ exceptions at the FFI boundary, so MSVC needs
        // /EHsc exception semantics. cc enables /EHsc for C++ by default; keep it
        // explicit here so that behavior is load-bearing, not incidental.
        .flag_if_supported("/EHsc")
        .compile("bearcad_kernel");

    println!("cargo:rustc-link-search=native={}", libdir.display());

    // OCCT toolkits, listed high-level -> low-level so a single-pass linker
    // resolves the (layered, acyclic) inter-toolkit dependencies. Only the
    // modeling toolkits are needed for the current kernel surface; visualization
    // / data-exchange are not linked.
    //
    // Static-archive linking already pulls in only the object files that are
    // referenced (unused .cxx compilation units never enter the binary). The
    // link-time dead-strip below goes finer-grained — dropping unreferenced
    // *functions/data* within the object files that do get pulled in — provided
    // OCCT itself was compiled with -ffunction-sections/-fdata-sections
    // (scripts/build-occt.sh sets that).
    for tk in OCCT_TOOLKITS {
        println!("cargo:rustc-link-lib=static={tk}");
    }

    // The C++ standard library the shim and OCCT need.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=dylib=c++");
            // ld64: strip unreferenced code/data from the final binary.
            println!("cargo:rustc-link-arg=-Wl,-dead_strip");
        }
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            // GNU ld / lld: garbage-collect unreferenced sections.
            println!("cargo:rustc-link-arg=-Wl,--gc-sections");
        }
        "windows" => {
            // MSVC link.exe: drop unreferenced functions/data (/OPT:REF) and fold
            // identical COMDATs (/OPT:ICF) — the /Gy analogue of --gc-sections.
            // No explicit C++ stdlib line: MSVC pulls in the CRT automatically.
            println!("cargo:rustc-link-arg=/OPT:REF");
            println!("cargo:rustc-link-arg=/OPT:ICF");
            // Windows system libs OCCT's toolkits reference (TKernel uses Win32
            // registry/session and Winsock APIs). First-pass list (#96) — tune it
            // from real linker errors once the windows-occt CI job produces logs.
            println!("cargo:rustc-link-lib=dylib=user32");
            println!("cargo:rustc-link-lib=dylib=advapi32");
            println!("cargo:rustc-link-lib=dylib=ws2_32");
        }
        _ => {}
    }
}

/// OCCT modeling toolkits, high-level first so a single-pass linker resolves the
/// layered (acyclic) inter-toolkit dependencies (see `build_occt_shim`). Covers
/// solids + booleans (TKBO/TKBool), shape healing they rely on (TKShHealing), and
/// triangulation (TKMesh).
const OCCT_TOOLKITS: &[&str] = &[
    // STEP data exchange (STEPControl_Reader/Writer, #65/#71). Highest-level:
    // depends on the modeling + foundation toolkits below, so links first.
    "TKDESTEP",
    "TKDE",
    "TKXSBase",
    "TKMesh",
    // Edge fillets/chamfers (BRepFilletAPI_*, #77). High-level: depends on
    // TKTopAlgo/TKBRep/TKBO below, so it links first.
    "TKFillet",
    "TKOffset",
    "TKBool",
    "TKBO",
    "TKShHealing",
    "TKPrim",
    "TKTopAlgo",
    "TKGeomAlgo",
    "TKBRep",
    "TKGeomBase",
    "TKG3d",
    "TKG2d",
    "TKMath",
    "TKernel",
];

fn png_to_ico(png_path: &str, out_path: &std::path::Path) {
    use ico::{IconDir, IconImage};
    use image::imageops::FilterType;
    use std::fs::File;
    use std::io::BufWriter;

    let image = image::ImageReader::open(png_path)
        .expect("open app icon png")
        .decode()
        .expect("decode app icon png")
        .into_rgba8();

    let mut icon_dir = IconDir::new(ico::ResourceType::Icon);
    for size in [256u32, 48, 32, 16] {
        let resized = image::imageops::resize(&image, size, size, FilterType::Lanczos3);
        let (width, height) = resized.dimensions();
        let icon = IconImage::from_rgba_data(width, height, resized.into_raw());
        let entry = ico::IconDirEntry::encode(&icon).expect("encode icon size");
        icon_dir.add_entry(entry);
    }

    let file = File::create(out_path).expect("create ico file");
    icon_dir
        .write(BufWriter::new(file))
        .expect("write ico file");
}
/// Compile SolveSpace's constraint-solver library (libslvs) straight from the submodule
/// sources — the same six translation units its own `slvs-solver`/`slvs-interface` CMake
/// targets build, plus the flat-array shim (cpp/bearcad_slvs.cpp) and mimalloc
/// (single-file amalgamation) for the solver's temporary arena. Header-only Eigen comes
/// from the vendored extlib. Linked into every native build; the wasm32 app gets
/// libslvs from the emscripten kernel module instead (scripts/build-occt-wasm.sh).
fn build_slvs() {
    use std::path::PathBuf;
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ss = manifest.join("third_party/solvespace");
    if !ss.join("include/slvs.h").is_file() {
        panic!(
            "solvespace submodule missing: run\n  git submodule update --init --depth 1 third_party/solvespace\n  (cd third_party/solvespace && git submodule update --init --depth 1 extlib/eigen extlib/mimalloc)"
        );
    }

    // mimalloc is compiled as C++ (its own MI_USE_CXX mode): the MSVC-in-C branch of
    // its atomics header is missing mi_atomic_void_addi64_relaxed (upstream bug), which
    // breaks stats.c under cl/clang-cl; the C++/C11 branch is complete on every target.
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file(ss.join("extlib/mimalloc/src/static.c"))
        .include(ss.join("extlib/mimalloc/include"))
        .flag_if_supported("-Wno-unused-function")
        .flag_if_supported("-Wno-deprecated")
        // Force C++ despite the .c extension (gnu-style and MSVC-style spellings).
        .flag_if_supported("-xc++")
        .flag_if_supported("/TP")
        .compile("slvs_mimalloc");

    let mut b = cc::Build::new();
    b.cpp(true)
        .std("c++17")
        .define("LIBRARY", None)
        .include(ss.join("include"))
        .include(ss.join("src"))
        .include(ss.join("extlib/eigen"))
        .include(ss.join("extlib/mimalloc/include"))
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        // Vendored solvespace/Eigen headers trip these harmlessly; silence to keep the build
        // output clean (they're third-party, not our code).
        .flag_if_supported("-Wno-missing-field-initializers")
        .flag_if_supported("-Wno-unused-but-set-variable");
    // MSVC: the define set solvespace's own CMake uses on Windows. _USE_MATH_DEFINES is
    // load-bearing (M_PI etc.); /bigobj covers the Eigen-heavy system.cpp object.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        b.define("_USE_MATH_DEFINES", None)
            .define("NOMINMAX", None)
            .define("WIN32_LEAN_AND_MEAN", None)
            .define("_CRT_SECURE_NO_WARNINGS", None)
            .define("_SCL_SECURE_NO_WARNINGS", None)
            .define("UNICODE", None)
            .define("_UNICODE", None)
            .flag_if_supported("/bigobj")
            .flag_if_supported("/EHsc");
    }
    b.file("cpp/bearcad_slvs.cpp");
    println!("cargo:rerun-if-changed=cpp/bearcad_slvs.cpp");
    for f in [
        "src/slvs/lib.cpp",
        "src/constrainteq.cpp",
        "src/entity.cpp",
        "src/expr.cpp",
        "src/platform/platformbase.cpp",
        "src/system.cpp",
        "src/util.cpp",
    ] {
        b.file(ss.join(f));
        println!("cargo:rerun-if-changed={}", ss.join(f).display());
    }
    b.compile("slvs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
