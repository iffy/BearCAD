---
sidebar_position: 3
title: Parameters & units
---

# Parameters & units

Parameters are named values like `leg = 50mm` that any value field can reference. Change
the value and everything built on it rebuilds.

## The Parameters pane

The **Parameters** pane lists every parameter. While a row's field is focused, the
Elements pane highlights everything that uses that parameter — and hovering a row (or
focusing its fields) also **glows those users green in the 3D view**: the dimensions
referencing it, the geometry they drive, and any body whose extrude distance uses it.

Each row's **gear** opens that parameter's options under its name (multiple can be open
at once):

- **Min**, **Max**, **Step** — optional expressions. Unit kind (length vs angle) comes
  from the default value. Importers must stay within these. Tab walks Min → Max → Step.
  With both min and max set, a **slider** sits on the row below, spanning the name and
  value columns.
- **Private** — checked = internal (hidden from
  [import](/docs/files#importing-bearcad-files) by default); unchecked = a knob importers
  are meant to change. Advisory only. A new parameter starts unchecked when its value is
  a plain number and checked when it's an expression.

## An imported unit's parameters

Select a [unit instance](/docs/files#importing-bearcad-files) and its parameters lead the
pane under the instance's name — public knobs first, with an **Internals** eye that
reveals the private ones. You can't edit the unit's min/max/step/private; you only
**override** the value on that instance (snapped to step, clamped to min/max). With both
min and max set, a **slider** sits on the row below (same as this document's parameters).
Overridden values read gold; **✕** restores the part's own value.

```lua
bearcad.unit_override{ instance = 0, name = "width", value = "20" }
bearcad.unit_override{ instance = 0, name = "width" }   -- back to the part's value
```

## Expressions

**Every value input accepts an expression**, not just a number:

- Arithmetic: `+ - * /` and parentheses — `leg / 2 + 5`.
- Functions: `max`, `min` (any number of arguments, or one `[a, b, c]` array), `abs`,
  `floor`, `ceil`/`ceiling`, and `round` — `max(w, 20)`, `min([leg, arm, 40])`,
  `ceil(span / step)`.
- **A unit instance's parameters**, qualified by the instance name: `bracket.width`.
  Backticks wrap a single name where it has spaces — `` `left bracket`.width ``. The
  value is the instance's override where one is set, otherwise the part's own. One level
  only: a nested unit's internals aren't reachable.
- Parameter names, including inside other parameters' expressions: `A + 5in`.
- **Mixed units**: `3mm + 2in` evaluates correctly. Lengths take `mm`, `cm`, `m`, `in`,
  `ft`; angles take `deg`, `rad`. A bare number is millimetres (degrees in angle fields).

The text you type is stored verbatim — reopen the field and `3mm + 2in` is still there.
Whenever what you typed isn't literally the resulting value, the field shows the computed
result beside it — `1in` shows `= 25.4 mm`, a bare `10` shows `= 10.0 mm`. Trailing zeros
do not count: `10mm` and `10.00 mm` do not preview `10.0 mm`.

While typing a name, autocomplete offers matching parameters: **Space**/**Tab** completes,
**Enter** completes *and* commits. Unit instances complete too: typing `fo` offers `foo`
(backticked when the name has spaces), and `foo.` offers that instance's parameters —
primary knobs first.

## Creating parameters inline

Typing `name=value` in any value field — `width=20mm` in an extrude-distance field, say —
creates that parameter on the spot and binds the field to it. A bare `name=` reuses an
existing parameter; `name=value` redefines it.

## Derived parameters

A **derived** parameter's value comes from measuring geometry. The
[Dimension tool](/docs/tools/dimension#measuring) records one from the selection with its
**Derive parameter** button, in a sketch or in 3D. Valid selections:

- **One line or edge** — its length (also on right-click: **Create parameter from
  length**).
- **Two points** — the distance between them (2D or 3D).
- **Two parallel lines** — the distance between them.
- **Two non-parallel lines in the same plane** — the angle between them.
- **One body edge** — its length.
- **Two body corners** — the distance between them, on one body or across two.

Derived values are read-only in the pane — a **lock** icon sits left of the name (hover
it to see what's measured), while the name itself stays editable — and re-measure as the
geometry changes. Focusing a derived parameter's row highlights the geometry that defines
it; clicking into its **name** field draws that source geometry in **green** in the 3D view.

```lua
bearcad.derive_parameter{ kind = "line_length", a = 0, name = "leg" }
bearcad.derive_parameter{ kind = "line_distance", a = 0, b = 1 }
bearcad.derive_parameter{ kind = "line_angle", a = 0, b = 2 }
bearcad.derive_parameter{ kind = "point_distance",
  a = { kind = "line", index = 0, endpoint = "start" },
  b = { kind = "line", index = 0, endpoint = "end" } }
-- Body geometry: a/b are mm points anywhere on the picked edge's ends or the corners.
bearcad.derive_parameter{ kind = "body_edge_length", body = 0, a = {0, 0, 0}, b = {30, 0, 0} }
bearcad.derive_parameter{ kind = "body_vertex_distance", body = 0,
  a = {0, 0, 0}, b = {30, 40, 0} }
```

## Display units

The Context pane's **Default units** section (Select tool, nothing selected) sets the
document-wide length and angle units used for dimension labels and the Elements pane. With
exactly one **sketch** selected it becomes **Sketch units** — a per-sketch override, with a
**Follow document** entry per axis to inherit the default again.

## Scripting

```lua
bearcad.parameter("add", "A", "5mm")
bearcad.parameter("value", 0, "A + 5in")     -- edit parameter 0's expression
bearcad.parameter("name", 0, "Len")
bearcad.parameter("private", 0, true)        -- hide from import (internal)
bearcad.parameter("min", 0, "1mm")           -- optional bounds; min+max ⇒ slider
bearcad.parameter("max", 0, "100mm")
bearcad.parameter("step", 0, "0.5mm")
bearcad.parameter("options", 0, true)        -- open the row's gear-options
bearcad.parameter("edit", 0, "min")          -- focus a bound field (Tab → max → step)
bearcad.parameter("editing")                 -- {index=, field="min"|"max"|"step"} or nil
bearcad.parameter("slider", 0)               -- {min=, max=, value=, step?} or nil
bearcad.parameter("slider", 0, 15)           -- set via the slider (mm / rad, snapped)
bearcad.parameter("min", 0)                  -- clear a bound
bearcad.parameter("delete", 0)
assert(bearcad.parameter("get", "A") == 5)   -- evaluated (mm / degrees)
bearcad.parameter("get_expression", "A")     -- "5mm", as typed

bearcad.set_units{ length = "in", angle = "deg" }          -- document defaults
bearcad.set_units{ sketch = 0, length = "mm" }             -- per-sketch override
```

Sizes in scripting calls accept expression strings too — see
[Declarative modeling](/docs/scripting/declarative-modeling).
