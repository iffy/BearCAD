---
sidebar_position: 3.5
title: Materials
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# Materials

A material is a name and a colour. Bodies render in their material's colour.

<a
  href={useBaseUrl("/app/") + "?open=" + encodeURIComponent(useBaseUrl("/img/screenshots/materials.bearcad.json"))}
  target="_blank"
  rel="noopener noreferrer"
  title="Open this model in BearCAD"
>
  <img src={useBaseUrl("/img/screenshots/materials.png")} alt="Nine cubes in a 2×2×2 with a centre cube, each a different material colour, some with a hole, sphere bite, chamfer or fillet" />
</a>

## Assigning

Select one or more bodies. The context pane's **Material** dropdown lists every material
in the document.

- A new document already has a palette: **Unobtainium** first (the grey-blue every new
  body starts as), then Blue, Green, Red, Yellow, Purple, Orange, Cyan, Pink, Grey.
  Consecutive entries contrast, so two picks in a row never look alike.
- **New material…** adds one (`Material N`, next colour in that rotation) and assigns it
  to the selection.
- **Name** and **Colour** edit the chosen material in place; every body using it
  re-renders.
- Selecting bodies of different materials reads *Mixed*.

A body extruded off another body's face is made of that body's material. A sketch on a
plane has no source body, so its extrusion starts as Unobtainium.

## Export

[3MF export](/docs/files#export) writes each body's colour, so a multi-colour model opens
in Bambu Studio with one filament slot per colour.

## Scripting

```lua
bearcad.material{ name = "Brass", color = "#c88a4a", bodies = {0} }
bearcad.set_material{ body = 1, material = 0 }
bearcad.set_material{ body = 1 }        -- back to the default material
```

Scripts name a material by its order in the document. Unobtainium is `0`.
