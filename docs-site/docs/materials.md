---
sidebar_position: 3.5
title: Materials
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# Materials

A material is a name and a color. Bodies render in their material's color.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/materials.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/materials.png")} alt="Nine cubes in a 2×2×2 with a centre cube, each a different material color, some with a hole, sphere bite, chamfer or fillet" />
</a>

## Assigning

Select one or more bodies. The context pane's **Material** dropdown lists every material
in the document.

- A new document already has a palette: **Unobtainium** first (the grey-blue every new
  body starts as), then Blue, Green, Red, Yellow, Purple, Orange, Cyan, Pink, Grey.
  Consecutive entries contrast, so two picks in a row never look alike.
- **New material…** adds one (`Material N`, next color in that rotation) and assigns it
  to the selection.
- **Name** and **Color** edit the chosen material in place; every body using it
  re-renders.
- Selecting bodies of different materials reads *Mixed*.

A body extruded off another body's face is made of that body's material. A sketch on a
plane has no source body, so its extrusion starts as Unobtainium.

## Export

[3MF export](/docs/files#export) writes each body's color, so a multi-color model opens
in Bambu Studio with one filament slot per color.
