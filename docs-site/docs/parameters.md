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
- Functions: `max`, `min` (any number of arguments, or one `[a, b, c]` array), and
  `abs` — `max(w, 20)`, `min([leg, arm, 40])`, `abs(off - 5)`.
- Parameter names, including inside other parameters' expressions: `A + 5in`.
- **Mixed units**: `3mm + 2in` evaluates correctly. Lengths take `mm`, `cm`, `m`, `in`,
  `ft`; angles take `deg`, `rad`. A bare number is millimetres (degrees in angle fields).

The text you type is stored verbatim — reopen the field and `3mm + 2in` is still there.

While typing a name, autocomplete offers matching parameters: **Space**/**Tab** completes,
**Enter** completes *and* commits.

## Creating parameters inline

Typing `name=value` in any value field — `width=20mm` in an extrude-distance field, say —
creates that parameter on the spot and binds the field to it. A bare `name=` reuses an
existing parameter; `name=value` redefines it.

## Measuring geometry into a parameter

Right-click in the viewport with a single undimensioned line selected and choose **Create
parameter from length**: a read-only parameter appears that always equals that line's current
length, so other features can reference a measured size.

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
