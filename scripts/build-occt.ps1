# Build the OpenCASCADE (OCCT) geometry kernel as static libraries for BearCAD (#86/#96).
#
# Windows/MSVC counterpart of scripts/build-occt.sh — keep the module ON/OFF set
# and overall structure in sync with that script so they stay maintainable side
# by side.
#
# BearCAD links OCCT statically (LGPL 2.1 permits this provided we ship the means
# to relink against a different OCCT — that's what OCCT_DIR + this script provide;
# see README.md "Building with the OCCT kernel"). OCCT source comes from the
# `third_party/OCCT` git submodule.
#
# Usage:
#   pwsh scripts/build-occt.ps1        # build into third_party/OCCT/occt-install
#   cargo build                        # then build BearCAD against it
#
# To build BearCAD against your *own* OCCT instead of this script's output, set
# OCCT_DIR to an install prefix containing include/opencascade and lib/TK*.lib
# and skip this script entirely.
#
# Modeling toolkits (FoundationClasses, ModelingData, ModelingAlgorithms) plus
# DataExchange (STEP read/write for #65/#71). Visualization, ApplicationFramework,
# Draw and the FreeType/TCL/TK/VTK dependencies stay disabled — not needed for the
# current kernel surface (solids, booleans, mass properties, STEP I/O).

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$occtSrc = Join-Path $repoRoot 'third_party/OCCT'
$occtBuild = Join-Path $occtSrc 'occt-build'
$occtInstall = Join-Path $occtSrc 'occt-install'

# Prebuilt key (todoer #469): the pinned OCCT submodule commit (from the gitlink, no
# submodule init needed) plus this script's own hash — matching the occt-prebuilt
# workflow's asset naming, so a fetch is only accepted when it matches exactly what
# a local build would produce.
$gitlink = ((git -C $repoRoot ls-tree HEAD third_party/OCCT) -split '\s+')[2].Substring(0, 12)
$scriptHash = (Get-FileHash -Algorithm SHA256 $PSCommandPath).Hash.ToLower().Substring(0, 12)
$prebuiltSlug = 'windows-x86_64'
$prebuiltKey = "$gitlink-$scriptHash"
$prebuiltAsset = "occt-install-$prebuiltSlug-$prebuiltKey.tar.gz"
$prebuiltUrl = "https://github.com/iffy/BearCAD/releases/download/occt-prebuilt/$prebuiltAsset"
$keyFile = Join-Path $occtInstall '.prebuilt-key'

# A previously fetched prebuilt that still matches the key needs no work at all.
if ($env:BEARCAD_OCCT_FROM_SOURCE -ne '1' -and (Test-Path $keyFile) -and
    ((Get-Content $keyFile -Raw).Trim() -eq "$prebuiltSlug-$prebuiltKey")) {
    Write-Host ">> Prebuilt OCCT already installed and up to date ($prebuiltSlug-$prebuiltKey)."
    Write-Host '>> Now build BearCAD with: cargo build'
    exit 0
}

# Try the prebuilt before compiling from source (skipped with
# BEARCAD_OCCT_FROM_SOURCE=1, or when an install is already present).
if ($env:BEARCAD_OCCT_FROM_SOURCE -ne '1' -and -not (Test-Path (Join-Path $occtInstall 'lib'))) {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) "bearcad-occt-prebuilt-$PID"
    New-Item -ItemType Directory -Force $tmp | Out-Null
    try {
        Write-Host ">> Trying prebuilt OCCT: $prebuiltUrl"
        $archive = Join-Path $tmp $prebuiltAsset
        Invoke-WebRequest -Uri $prebuiltUrl -OutFile $archive
        Invoke-WebRequest -Uri "$prebuiltUrl.sha256" -OutFile "$archive.sha256"
        $want = ((Get-Content "$archive.sha256" -Raw).Trim() -split '\s+')[0].ToLower()
        $got = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
        if ($want -and ($want -eq $got)) {
            New-Item -ItemType Directory -Force $occtSrc | Out-Null
            tar xzf $archive -C $occtSrc
            if ($LASTEXITCODE -ne 0) { throw "tar extraction failed (exit $LASTEXITCODE)" }
            Set-Content -Path $keyFile -Value "$prebuiltSlug-$prebuiltKey"
            Write-Host ">> Installed prebuilt OCCT into $occtInstall (checksum verified)."
            Write-Host '>> Now build BearCAD with: cargo build'
            exit 0
        }
        Write-Warning ">> Prebuilt checksum mismatch (want $want, got $got); building from source."
    } catch {
        Write-Host ">> No prebuilt for $prebuiltSlug-$prebuiltKey (or download failed); building from source."
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

if (-not (Test-Path (Join-Path $occtSrc 'CMakeLists.txt'))) {
    Write-Error ("OCCT submodule missing at $occtSrc`n" +
        '       run: git submodule update --init --depth 1 third_party/OCCT')
}

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    Write-Error 'cmake not found on PATH'
}

$jobs = [Environment]::ProcessorCount
if (-not $jobs -or $jobs -lt 1) { $jobs = 4 }

Write-Host '>> Configuring OCCT (static, modeling-only, MSVC) ...'
# Windows/MSVC specifics vs the .sh:
#   * /Gy is MSVC's function-level-sections analogue of -ffunction-sections, so
#     BearCAD's link-time dead-strip (build.rs: /OPT:REF,ICF) can drop every OCCT
#     function the final binary never calls. Passing CMAKE_CXX_FLAGS here replaces
#     only CMake's *base* flags — the per-config Release flags (/O2 etc.) still
#     apply — so /EHsc (MSVC C++ exception semantics, normally part of the base
#     flags) must be re-supplied alongside /Gy.
#   * CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL selects the dynamic CRT (/MD),
#     matching Rust's x86_64-pc-windows-msvc default so the static OCCT archives
#     link cleanly into the cargo build.
#   * The generator is CMake's default (Visual Studio on windows-latest), which is
#     multi-config — hence `-A x64` at configure time and `--config Release` on
#     both the build and install steps below.
#   * INSTALL_DIR_LAYOUT=Unix (#96): OCCT's own CMakeLists.txt defaults this to
#     "Windows" on WIN32, which installs libs under
#     `<prefix>/<os>/<compiler>/lib` and headers under `<prefix>/inc` — a
#     different layout than build.rs's cross-platform `include/opencascade` +
#     `lib` expectation (which macOS/Linux already get, since OCCT's default
#     layout there is "Unix"). Forcing "Unix" here gives Windows the same flat
#     layout build.rs looks for, without needing a Windows-specific path branch
#     there.
cmake -S "$occtSrc" -B "$occtBuild" `
    -A x64 `
    -DCMAKE_INSTALL_PREFIX="$occtInstall" `
    -DINSTALL_DIR_LAYOUT=Unix `
    -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL `
    -DCMAKE_CXX_FLAGS="/Gy /EHsc" `
    -DCMAKE_C_FLAGS="/Gy" `
    -DBUILD_LIBRARY_TYPE=Static `
    -DBUILD_MODULE_FoundationClasses=ON `
    -DBUILD_MODULE_ModelingData=ON `
    -DBUILD_MODULE_ModelingAlgorithms=ON `
    -DBUILD_MODULE_Visualization=OFF `
    -DBUILD_MODULE_ApplicationFramework=OFF `
    -DBUILD_MODULE_DataExchange=ON `
    -DBUILD_MODULE_Draw=OFF `
    -DBUILD_MODULE_DETools=OFF `
    -DUSE_FREETYPE=OFF `
    -DUSE_TK=OFF `
    -DUSE_TCL=OFF `
    -DUSE_VTK=OFF `
    -DUSE_FREEIMAGE=OFF `
    -DUSE_RAPIDJSON=OFF `
    -DUSE_OPENGL=OFF `
    -DUSE_GLES2=OFF `
    -DBUILD_DOC_Overview=OFF
if ($LASTEXITCODE -ne 0) { Write-Error "cmake configure failed (exit $LASTEXITCODE)" }

Write-Host ">> Building OCCT with $jobs jobs (this takes a while) ..."
cmake --build "$occtBuild" --config Release --parallel $jobs
if ($LASTEXITCODE -ne 0) { Write-Error "cmake build failed (exit $LASTEXITCODE)" }

Write-Host ">> Installing OCCT into $occtInstall ..."
cmake --install "$occtBuild" --config Release
if ($LASTEXITCODE -ne 0) { Write-Error "cmake install failed (exit $LASTEXITCODE)" }

Write-Host ">> Done. OCCT static libs are in $occtInstall/lib"
Write-Host '>> Now build BearCAD with: cargo build'
