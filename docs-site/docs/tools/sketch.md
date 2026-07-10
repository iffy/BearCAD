---
sidebar_position: 3
title: Sketch
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/sketch.svg")} width="30" /> Sketch

**Shortcut:** `S`

Everything in BearCAD starts as a 2D **sketch** drawn on a flat surface. The Sketch tool
picks that surface: click the ground plane, a construction plane, or any flat face of a
solid body, and the camera turns to face it head-on. The Line, Rectangle, and Circle tools
then draw onto it.

While a sketch is open, the viewport has a bright **orange border** so you always know
you're in sketch mode. Press **Esc** (with nothing in progress) to leave the sketch.

## Notes

- Sketching on a body's own face ties the sketch to that body: it moves with it, and the
  face's corners and edges can be dimensioned against directly.
- When faces overlap under the cursor, the one nearest the camera wins — you never pick a
  hidden face by accident.
- Click an existing sketch's face with the Sketch tool to reopen it for editing.
