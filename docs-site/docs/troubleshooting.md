---
sidebar_position: 12
title: Troubleshooting
---

# Troubleshooting

## The log

Every run writes a log. Its path is printed when the app starts:

```text
bearcad: logging to /tmp/bearcad/bearcad.log
```

The previous run is kept beside it as `bearcad.prev.log`, so a crash on startup is still
readable after you restart to look.

The log holds everything: what was launched, the GPU in use, every action and every refused
one, imports, and any panic. Warnings and notable events also print to the terminal, so
running BearCAD from a shell narrates the session as it goes.

```bash
BEARCAD_LOG=1 bearcad            # the full trace in the terminal too
BEARCAD_LOG_FILE=./run.log bearcad   # write the log somewhere else
```

## An action didn't do what you expected

Look for the line naming it. A refused action says why:

```text
[   4.117] info  CreateExtrusion refused: Select a closed profile to extrude
```

That reason appears in the status bar as well, but the log keeps it after the next action
has replaced it.

## A McMaster-Carr download didn't arrive

The catalog window keeps its own log, `bearcad-mcmaster.log`, next to the app's. It records
every page it opened and every download it saw:

```text
[  12.004] info  catalog: download started — https://… → /tmp/bearcad-mcmaster/91290A115.STEP
[  12.310] info  catalog: download finished — /tmp/bearcad-mcmaster/91290A115.STEP
```

The lines to look for:

- **`popup handed to the browser`** — the click opened a new window that didn't look like a
  file, so it went to your normal browser. If that was the CAD download, say so: the rule for
  telling a download from a help popup is a judgement about the URL, and it can be corrected.
- **no download line at all** — the click never reached the window's download machinery.
- **`did not finish`** — it started and failed part way.
- **`nothing recorded where it went`** — it finished, but the destination was lost. The file
  is probably in `/tmp/bearcad-mcmaster/`; **File → Import → STEP…** will take it from there.

## A blank window

If the window comes up grey and empty, the log distinguishes the two causes: no frames drawn
at all (the app never painted) versus a handful and then nothing (it painted, then stopped
being asked to). Either way the log says which, a few seconds after launch.
