---
sidebar_position: 3
title: Parameters & units
---

# Parameters & units

Parameters are named values like `leg = 50mm` that any value field can reference. Change
the value and everything built on it rebuilds.

## The Parameters pane

The **Parameters** pane lists every parameter. While a row's field is focused, the
Elements pane highlights everything that uses that parameter.

## Expressions

**Every value input accepts an expression**, not just a number:

- Arithmetic: `+ - * /` and parentheses — `leg / 2 + 5`.
- Functions: `max`, `min` (any number of arguments, or one `[a, b, c]` array), `abs`,
  `floor`, `ceil`/`ceiling`, and `round` — `max(w, 20)`, `min([leg, arm, 40])`,
  `ceil(span / step)`.
- Parameter names, including inside other parameters' expressions: `A + 5in`.
- **Mixed units**: `3mm + 2in` evaluates correctly. Lengths take `mm`, `cm`, `m`, `in`,
  `ft`; angles take `deg`, `rad`. A bare number is millimetres (degrees in angle fields).

The text you type is stored verbatim — reopen the field and `3mm + 2in` is still there.
Whenever what you typed isn't literally the resulting value, the field shows the computed
result beside it — `1in` shows `= 25.4 mm`, a bare `10` shows `= 10.0 mm`.

While typing a name, autocomplete offers matching parameters: **Space**/**Tab** completes,
**Enter** completes *and* commits.

## Creating parameters inline

Typing `name=value` in any value field — `width=20mm` in an extrude-distance field, say —
creates that parameter on the spot and binds the field to it. A bare `name=` reuses an
existing parameter; `name=value` redefines it.

## Derived parameters

A **derived** parameter's value comes from measuring geometry. The
[Dimension tool](/docs/tools/dimension#in-3d-mode) in 3D mode captures one with a click,
or make a selection and the Parameters pane shows the measured value next to a **Derive
from selection** button. Valid selections:

- **One line or edge** — its length (also on right-click: **Create parameter from
  length**).
- **Two points** — the distance between them (2D or 3D).
- **Two parallel lines** — the distance between them.
- **Two non-parallel lines in the same plane** — the angle between them.

Derived values are read-only in the pane (the name stays editable) and re-measure as the
geometry changes. Focusing a derived parameter's row highlights the geometry that defines
it; clicking into its **name** field draws that source geometry in **green** in the 3D view.

```lua
bearcad.derive_parameter{ kind = "line_length", a = 0, name = "leg" }
bearcad.derive_parameter{ kind = "line_distance", a = 0, b = 1 }
bearcad.derive_parameter{ kind = "line_angle", a = 0, b = 2 }
bearcad.derive_parameter{ kind = "point_distance",
  a = { kind = "line", index = 0, ["end"] = "start" },
  b = { kind = "line", index = 0, ["end"] = "end" } }
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
bearcad.parameter("delete", 0)
assert(bearcad.parameter("get", "A") == 5)   -- evaluated (mm / radians)
bearcad.parameter("get_expression", "A")     -- "5mm", as typed

bearcad.set_units{ length = "in", angle = "deg" }          -- document defaults
bearcad.set_units{ sketch = 0, length = "mm" }             -- per-sketch override
```

Sizes in scripting calls accept expression strings too — see
[Declarative modeling](/docs/scripting/declarative-modeling).
