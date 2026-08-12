---
sidebar_position: 2.5
title: Why use BearCAD?
---

# Why use BearCAD?

**Is BearCAD better than Fusion, SolidWorks, FreeCAD, or TinkerCAD?** No.

BearCAD is pre-alpha. The others have years of features, polish, and users. Need CAM,
simulation, sheet metal, or a production assembly? Use one of them.

Use BearCAD when a tiny local CAD app that launches instantly and needs no account is
worth the missing features.

Update the tables below when a BearCAD feature lands.

## Size, start, cost

As of August 2026. Sizes are the download/installer, not the installed footprint. Start
times are typical cold launches, not benchmarks.

| | BearCAD | Autodesk Fusion | SolidWorks | FreeCAD | TinkerCAD |
|---|---|---|---|---|---|
| **Download** | 21–50 MB | 8.5 GB to install | ~8–20 GB | 500–800 MB | none (browser) |
| **Start time** | ~0.5 s | tens of seconds + sign-in | tens of seconds | several–15 s | page load + account |
| **Cost** | Free | $680/yr; personal-use free (limited) | ~$2,820/yr Standard | Free | Free |
| **Account** | No | Yes | License | No | Yes |
| **Offline** | Yes | Partial | Yes | Yes | No |
| **Platforms** | macOS, Windows, Linux, browser | Windows, macOS, browser | Windows | Windows, macOS, Linux | Browser |

BearCAD sizes from the latest GitHub release (macOS 21 MB, Linux 25 MB, Windows 50 MB).
Fusion install size is Autodesk's published requirement. SolidWorks varies with which
products you pick. FreeCAD 1.1.3 installers are ~495 MB (Windows) to ~780 MB (Linux
AppImage). Fusion personal-use is free for non-commercial work under $1,000/yr, with
feature and document-count limits.

## Features

| | BearCAD | Fusion | SolidWorks | FreeCAD | TinkerCAD |
|---|---|---|---|---|---|
| Parametric sketches | Yes | Yes | Yes | Yes | No |
| Constraint solver | Yes | Yes | Yes | Yes | No |
| Extrude / revolve / loft / sweep | Yes | Yes | Yes | Yes | CSG primitives |
| Fillet / chamfer / shell | Yes | Yes | Yes | Yes | Limited |
| Assemblies | Joints only | Yes | Yes | Yes | Groups |
| Technical drawings | Basic | Yes | Yes | Yes | No |
| STEP / STL import & export | Yes | Yes | Yes | Yes | STL |
| CAM | No | Yes | Extra | Yes | No |
| Simulation / FEA | No | Yes | Extra | Yes | No |
| Sheet metal | No | Yes | Yes | Addon | No |
| Surface modeling | No | Yes | Yes | Yes | No |
| PCB / electronics | No | Yes | Extra | Addon | Circuits |
| Rendering | No | Yes | Yes | Addon | Basic |
| Scripting | Lua | API | API | Python | Codeblocks |
| Local files | Yes | Cloud-first | Yes | Yes | Cloud |
| Open source | Yes | No | No | Yes | No |

BearCAD's kernel is OpenCASCADE (same family as FreeCAD) and sketches are solved by
SolveSpace. Joints pose parts; components are still just folders. Drawings project
views to PDF/SVG. No BOM, no GD&T. The app is MIT/Apache-2.0; the kernel is LGPL.

## Why use it anyway

- **Small.** Tens of megabytes, not gigabytes.
- **Fast.** Cold launch is about half a second. No splash, no sign-in.
- **Yours.** Files stay on your computer.
- **Scriptable.** Anything you click, you can script in Lua.

[Quickstart](/docs/quickstart) is the fastest way to see whether that trade is worth it.
