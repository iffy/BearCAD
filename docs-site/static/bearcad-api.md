# BearCAD Lua API

Use only the functions listed here. Unknown option keys fail the call and name the
accepted ones. Prefer `bearcad.*` (declarative modeling) over `bearcad.ui.*`.

Every dimension is a ValueInput, the same field the app gives you: pass a number, or an
expression string that may name parameters, units and arithmetic — `"leg"`, `"leg / 2 + 3mm"`,
`"45deg"`, `"1.2rad"`, `"5in"`. An expression stays live, so editing the parameter moves the
geometry. A bare number is millimetres (degrees for an angle) — the canonical units every
read-back reports; a bare number *inside* a string follows the document's default unit, so
`"1.5"` is 1.5 in when `bearcad.set_units{ length = "in" }` is in force.

Indices are creation-order ordinals and shift when things are deleted — prefer names
(`bearcad.find`) for anything you will refer to twice. One operation per call
(especially fillets, chamfers, booleans).

A rectangle is four lines (bottom, right, top, left); `rect` returns those lines as a
profile. Drawing verbs open a ground-plane sketch when none is active. An operation that
consumes a body produces a new one: chain off the handle the call returned, or use names.

## Shapes (no sketch)

A cube/box/block is `cuboid` (`cube`/`box` alias it). `cube{ size = 10 }` is equal
sides. It sits on the plane at `at` (the base centre, default origin) and grows along
`normal` (default +Z, so on the ground).

```lua
bearcad.cube{ size = 10 }
bearcad.cuboid{ width = 10, depth = 10, height = 10 }
```

`r` / `radius` / `diameter` are accepted on every radial size (circle, cylinder,
sphere, fillet). Circle `get` returns `r`, `radius`, and `diameter`.

```
bearcad.cuboid{ width, depth, height, at = {x,y,z}?, normal?, u_axis?, name? }
bearcad.cube{ size, width?, depth?, height?, at?, normal?, u_axis?, name? }
bearcad.box{ width, depth, height, at?, normal?, u_axis?, name? }
bearcad.cylinder{ r | radius | diameter, height, at?, normal?, name? }
bearcad.sphere{ r | radius | diameter, at?, name? }
bearcad.edit_shape{ index, shape = "cuboid"|"cylinder"|"sphere"?, width?, depth?, height?, size?, radius?, at?, normal?, u_axis?, name? }
```

Every dimension takes a number or an expression string.

## Sketching

```
bearcad.rect{ width, height, x = 0?, y = 0?, name? }
bearcad.line{ x, y, x1, y1, name?, dimension? }          -- or length + angle (degrees)
bearcad.circle{ x, y, r | radius | diameter, name? }
bearcad.edit_circle{ index, r | radius | diameter, name? }
bearcad.text{ text, x, y, size?, font?, bold?, italic?, underline?, rotation?, wrap?, flip?, name? }
bearcad.begin_sketch{ kind = "plane", index = i }
bearcad.begin_sketch(box:face("top"))            -- or a body_faces entry
bearcad.begin_sketch{ kind = "extrude_cap"|"extrude_side"|…, … }
bearcad.open_sketch(i)
bearcad.exit_sketch()
bearcad.plane{ offset?, from = 0?, origin = {x,y,z}?, normal = {x,y,z}?, axis = "x"|"y"|"z"|line?, angle?, name? }
bearcad.project{ body?, bodies?, plane?, planes?, entities? }
```

`dimension` on a line locks its length (number, expression, or `true` for as-drawn).

## Solids

```
bearcad.extrude{ profiles = circle | {line, …} | {…}, distance?, to?, body = "new"|"add"|"cut"|"join"?, name?, symmetric?, taper?, taper_mode = "distance"|"angle"? }
bearcad.sketch_faces(sketch?)            -- closed loops/circles/text/regions for `profiles`
bearcad.edit_extrusion{ index | extrusion, distance? | by? | to? }
bearcad.extrude_face{ face = {…}, distance, body? }
bearcad.revolve{ profiles = circle | {line, …} | {…}, axis = "x"|"y"|"z"|{ line = i }, angle? | revolutions?, pitch?, body = "new"|"add"|"cut"?, bodies?, symmetric?, name? }
bearcad.edit_revolve{ index, angle? | revolutions?, pitch?, axis?, … }
bearcad.sweep{ profiles = circle | {line, …} | {…}, path = {line, …}, body = "add"|"cut"?, bodies? }
bearcad.edit_sweep{ index, path?, … }
bearcad.loft{ profiles = { circle | {line, …}, … }, body? }
bearcad.edit_loft{ index, … }
bearcad.combine{ op = "union"|"cut"|"intersect"|"xor", a = {i, …}, b = {i, …}, keep_b?, bake?, name? }   -- `difference` means cut; bake = true consumes the inputs and leaves one standalone body
bearcad.edit_combine{ index, op, a, b, keep_b? }
bearcad.slice{ bodies = {i, …}, cutters = {…}, extend?, name? }
bearcad.shell{ bodies = {i, …}, faces = {…}?, thickness, name? }
bearcad.move_bodies{ bodies = {i, …}, x?, y?, z?, rx?, ry?, rz?, name? }
bearcad.mirror_bodies{ plane = i, bodies = {i, …}, output = "new"|"add"|"cut"?, name? }
bearcad.repeat_bodies{ bodies = {i, …}, axis = "x"|"y"|"z", mode?, count?, spacing?, length?, around?, flip?, to?, name? }
```

To cut a hole: sketch on a face, then `extrude{ …, body = "cut" }`. A cut pointing away
from the body is flipped inward.

Rounding is one call per operation — a set of edges in a single call, never one call per
edge (four calls would make four bodies):

```
bearcad.fillet{ body = h, edges = bearcad.body_edges(h) | { { kind = "vertical"|"top"|"bottom", face = i, edge = i }, … }, r | radius | diameter }
bearcad.chamfer{ body = h, edges = …, distance }
bearcad.edit_fillet{ index, radius? }
bearcad.edit_chamfer{ index, distance? }
bearcad.fillet_edge / chamfer_edge     -- aliases; `extrusion=` / `shape=` still accepted
bearcad.extrude_edges(i)               -- analytic edge refs on extrusion i
bearcad.fillet_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, r | radius | diameter }
bearcad.chamfer_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, distance }
```

## Parameters and constraints

```
bearcad.add_parameter("w", "24")
bearcad.set_parameter("w", "30")
bearcad.select{ kind, index, endpoint? }               -- second arg true = add
                                                       -- line vertex: endpoint = "start"|"end"
                                                       -- or line:start() / line:endpoint("end")
                                                       -- drawing page items: kind "projection"|"annotation"|"dimension"
                                                       --   + drawing (+ view / a,b / index); selecting opens the drawing
bearcad.constrain("parallel"|"perpendicular"|"equal"|"coincident"|"midpoint"|"horizontal"|"vertical"|"tangent", a, b, …)
bearcad.dimension{ kind = "line"|"circle"|"point_point"|"point_line"|"line_line"|"angle", value, … }
bearcad.ui.add_geometric_constraint(name)              -- current selection; UI tests only
```

Anywhere a size is accepted, an expression string is too.

## Inspect

```
bearcad.count(kind)                -- canonical: line, circle, sketch, constraint,
bearcad.get(handle|{ kind, index }|kind, index)
                                   --   plane, extrusion, revolution, sweep,
                                   --   loft, combine, move, mirror, repeat, slice, shell,
                                   --   edge_treatment, sketch_offset, sketch_mirror,
                                   --   sketch_repeat, sketch_slice, sketch_chamfer, shape,
                                   --   body, drawing, cross_section, section_plane, parameter,
                                   --   sketch_text, component, image, joint, unit_instance.
                                   --   aliases: construction_plane, revolve, boolean, primitive, text,
                                   --   tracing_image, sketch_fillet, unit, offset.
                                   --   not chamfer/fillet (use edge_treatment or sketch_chamfer).
                                   --   `count` and `get` take the same set.
                                   --   get returns create/edit keys plus evaluated numbers;
                                   --   missing identity is nil (unknown kind errors).
bearcad.find("name")               -- sugar for element-by-name; nil if missing
bearcad.set_name(el, "name")
bearcad.element("line", i)         -- the lookup; also element(id) / element(name)
bearcad.id(el)                     -- el:id(): a stable id, unique and never reused
bearcad.line_endpoints(i)          -- x0, y0, x1, y1; missing → nil
bearcad.image_corners(i)           -- tracing image quad in world mm, live Move included
bearcad.body_stats(i)              -- volume, triangles, bbox = { min = {x,y,z}, max = {x,y,z} }
                                   -- missing body → nil; body with no mesh → error
bearcad.body_faces(i)              -- pass an entry to begin_sketch / extrude_face / fillet
bearcad.drawing_views(i)           -- a drawing's page: orientation, style, bodies, dimensions
bearcad.body_edges(i)              -- pass entries to fillet{ body, edges } / chamfer{ body, edges }
bearcad.body_cylinders(i)
bearcad.selection()                -- { kind, index, … } tables that work as handles;
                                   -- point selections include index + endpoint
bearcad.visible(el)                -- effective visibility, component chain included
bearcad.set_visible(el, false)     -- handle, list, or { kind = "plane" }; boolean only
bearcad.set_construction(el, true) -- same targets; selection forms are bearcad.ui.*
bearcad.sketch_dof()
bearcad.sketch_conflicts()
bearcad.sketch_faces()

bearcad.status()
```

Never assume a call did what you meant: read it back and assert.

## Handles

A creation call hands back what it made: one element, or a list of them.

```
local sides = bearcad.rect{ x = 0, y = 0, width = 20, height = 10 }   -- four lines
local box   = bearcad.extrude{ profiles = sides, distance = 5 }        -- the new body
box:kind()  box:index()  box:id()  box:name()  box:exists()  box:delete()
box:get()   box:stats()  box:select()  tostring(box)
bearcad.delete(box)                -- or a list; does not replace the scene selection
bearcad.delete_selection()         -- whatever is selected (the GUI Delete)
```

Ordinals shift when elements are deleted, and a solid op consumes the body it acts on.
A handle does not: it names the same element until that element is gone, and then says so.
Anywhere an index is accepted — `bodies`, `profiles`, `extrusion`, `{ kind, index }`, … —
a handle, its `id` string, or a name works too.

## Files

```
bearcad.new()
bearcad.open("part.bearcad")
bearcad.save()                     -- or save("other.bearcad")
bearcad.undo()
bearcad.import_step("part.step")
bearcad.import_stl("part.stl")
bearcad.export_step("out.step")            -- or (path, body) with a handle/id/name/ordinal
bearcad.export_stl("out.stl")
bearcad.export_3mf("out.3mf")
```

## Drawings

`bearcad.drawing_text{ drawing, text, x, y }` — x/y are page fractions (0–1), not millimetres.

## GUI (`bearcad.ui.*`)

Reach for this only when the interaction itself is the point.

```
bearcad.ui.tool("select"|"rectangle"|…)
bearcad.ui.view("front"|"top"|"iso"|…)
bearcad.ui.zoom_fit()
bearcad.ui.screenshot("shot.png")            -- viewport; "window" / a pane name for others
bearcad.ui.camera{ yaw?, pitch?, distance?, target?, projection?, shading?, ground? }
bearcad.ui.camera{}                          -- read it back, shading and ground included
bearcad.ui.shading("loose_pencil")           -- …|realistic|loose_pencil|dark_pencil|color_pencil|watercolor
bearcad.ui.pane("ai"|"hierarchy"|"context"|"parameters"|…, "show"|"hide"|"toggle")
bearcad.ui.click_ground(x, y)               -- sketch-plane millimetres
bearcad.ui.click_world(x, y, z)             -- any world point: a body's side wall, say
bearcad.ui.click(x, y) / click(rect)        -- viewport px, or a window-space rect/orb
bearcad.ui.double_click(x, y) / (rect)      -- waits out egui's click counter
bearcad.ui.viewport()                       -- { width, height, x, y } of the area clicks address
bearcad.ui.right_click_ground(x, y)         -- opens a context menu
bearcad.ui.context_menu()                   -- { kind, index } of the open menu, or nil
bearcad.ui.key("enter")
bearcad.ui.palette("Export STEP")
bearcad.ui.begin_move{ … } / begin_combine / begin_joint / begin_edit_shape   -- arm a tool; do not commit
bearcad.ui.pickers() / picker("Targets")                  -- armed tool pickers
bearcad.ui.gizmos() / gizmo("move_rz")                    -- live gizmo rows
bearcad.ui.hovered() / exploder()                         -- viewport hover / Selection Exploder
bearcad.ui.set_dim / edit_dim / commit_dim                -- dimension widget
```

## Every function

Every function BearCAD exposes, with the arguments it takes — a name that is
not in this list is not a function. Built from the live Lua table:
accepted option keys plus positional args. `{ … }` is one options table; `?` marks an
optional argument or table. The sections above carry the detail for the calls they
cover.

```
bearcad.add_parameter(name, expression)
bearcad.add_unit_instance{ unit, name }
bearcad.bake(index)
bearcad.begin_sketch(…)
bearcad.body_cylinders(index)
bearcad.body_edges(index)
bearcad.body_faces(index)
bearcad.body_stats(index)
bearcad.box{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.calibrate_image{ image, from, to, length }
bearcad.calibration_point{ image, index, x, y }
bearcad.chamfer{ body, edges, edge, extrusion, shape, primitive, distance }
bearcad.chamfer_edge{ body, edges, edge, extrusion, shape, primitive, distance }
bearcad.chamfer_vertex{ … }
bearcad.circle{ x, y, r, radius, diameter, name }
bearcad.clear()
bearcad.clear_selection()
bearcad.clone_unit_instance{ instance }
bearcad.combine{ op, a, b, keep_b, bake, name }
bearcad.component{ name, parent }?
bearcad.constrain(name, …)
bearcad.copy()
bearcad.count(kind)
bearcad.count_saved(kind)
bearcad.cross_section{ name }?
bearcad.cube{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.cuboid{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.cylinder{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.debug.mesh_cache()
bearcad.debug.session_writes()
bearcad.debug.sqlite_scalar(sql)
bearcad.debug.tool_row()
bearcad.debug.tool_table()
bearcad.debug.widget_id_warnings(…)
bearcad.delete(…)
bearcad.delete_drawing_loupe{ drawing, view, index }
bearcad.delete_parameter(target)
bearcad.delete_section_plane{ view, cut }
bearcad.delete_selection()
bearcad.derive_parameter{ kind, from, a, b, body, body_b, name, instance, face, edge }
bearcad.dimension{ kind, type, value, index, a, b, sign, point, line, anchor, mover }
bearcad.drag_line(first, anchor_u?, anchor_v?, u?, v?)
bearcad.drag_vertex(first, u?, v?)
bearcad.drawing{ … }?
bearcad.drawing_align_view{ drawing, parent, dir, pos }
bearcad.drawing_angle{ … }
bearcad.drawing_circle_dim_offset{ … }
bearcad.drawing_circle_dimension{ drawing, view, center }
bearcad.drawing_curve_dimension{ drawing, view, points }
bearcad.drawing_dim_offset{ drawing, view, a, b, offset, angle, side }
bearcad.drawing_dimension{ drawing, view, a, b }
bearcad.drawing_loupe{ drawing, view, at, radius, to, to_radius }
bearcad.drawing_loupe_dimension{ drawing, view, index, a, b }
bearcad.drawing_loupes{ drawing, view }
bearcad.drawing_move_view{ drawing, view, x, y }
bearcad.drawing_page{ drawing, width, height, margin }
bearcad.drawing_paper{ drawing, paper }
bearcad.drawing_point_dim_offset{ drawing, view, index, offset }
bearcad.drawing_point_dimension{ drawing, view, a, b, axis }
bearcad.drawing_point_dimension_axis{ drawing, view, index, axis }
bearcad.drawing_style{ … }
bearcad.drawing_text{ … }
bearcad.drawing_view{ drawing, body, bodies, component, sketch, cross_section, orientation }
bearcad.drawing_view_add{ drawing, view, body, bodies, component }
bearcad.drawing_view_align_lines{ … }
bearcad.drawing_view_label{ drawing, view, hidden, pos, text }
bearcad.drawing_view_lines{ … }
bearcad.drawing_view_orientation{ … }
bearcad.drawing_view_scale{ drawing, view, scale }
bearcad.drawing_view_section{ drawing, view, cross_section }
bearcad.drawing_view_size{ drawing, view, width, height, size_x, size_y }
bearcad.drawing_view_style{ … }
bearcad.drawing_views(index)
bearcad.edit_chamfer{ index, edge, edges, body, extrusion, shape, primitive, distance }
bearcad.edit_circle{ index, r, radius, diameter, name }
bearcad.edit_combine{ index, op, a, b, keep_b }
bearcad.edit_drawing_loupe{ drawing, view, index, at, radius, to, to_radius, style }
bearcad.edit_extrusion{ index, extrusion, distance, by, to }
bearcad.edit_fillet{ index, edge, edges, body, extrusion, shape, primitive, radius }
bearcad.edit_joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }
bearcad.edit_loft{ index, circle, circles, polygon, polygons, body, bodies, name }
bearcad.edit_mirror{ index, plane, bodies, output }
bearcad.edit_move{ bodies, images, x, y, z, rotate, rx, ry, rz, roll, flip, spin, gap, from, to, index }
bearcad.edit_parameter{ name, private, min, max, step, rename }
bearcad.edit_repeat{ index, bodies, axis, around, flip, mode, count, spacing, length, to }
bearcad.edit_revolve{ index, circle, circles, polygon, axis, angle, revolutions, pitch, offset, gap, symmetric, body, bodies, name }
bearcad.edit_section_plane{ view, cut, offset, roll, depth, flip, bodies, exclude_bodies }
bearcad.edit_shape{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.edit_shell{ index, bodies, faces, thickness }
bearcad.edit_sketch_mirror{ index, sketch, line, lines, circles }
bearcad.edit_sketch_offset{ index, sketch, lines, circles, distance, construction }
bearcad.edit_sketch_repeat{ index, sketch, lines, circles, angle, dir, mode, count, spacing, length }
bearcad.edit_sketch_slice{ index, lines, circles, faces, cutters }
bearcad.edit_slice{ index, bodies, cutters, extend }
bearcad.edit_sweep{ index, circle, circles, polygon, path, body, bodies, name }
bearcad.element(kind, index?)
bearcad.exit_sketch()
bearcad.export_3mf(path, body?)
bearcad.export_drawing_pdf{ drawing, path }
bearcad.export_drawing_svg{ … }
bearcad.export_lua(path)
bearcad.export_preview(path, opts? | { yaw, pitch })
bearcad.export_step(path, body?)
bearcad.export_stl(path, body?)
bearcad.extrude{ distance, to, profiles, circle, circles, polygon, polygons, text, boolean, body, name, symmetric, taper, taper_mode }
bearcad.extrude_edges(index)
bearcad.extrude_face{ … }
bearcad.fillet{ body, edges, edge, extrusion, shape, primitive, r, radius, diameter }
bearcad.fillet_edge{ body, edges, edge, extrusion, shape, primitive, r, radius, diameter }
bearcad.fillet_vertex{ points, point, r, radius, diameter }
bearcad.find(name)
bearcad.get(…)
bearcad.globals()
bearcad.id(element)
bearcad.image_corners(index)
bearcad.image_opacity{ image, opacity }
bearcad.import_image(value)
bearcad.import_lua(value | { path, force })
bearcad.import_step(path)
bearcad.import_stl(path)
bearcad.import_unit(value | { path, link, name })
bearcad.joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }
bearcad.line{ x, y, x1, y1, length, angle, bezier, dimension, name }
bearcad.line_endpoints(index)
bearcad.loft{ … }
bearcad.material{ name, color, bodies }
bearcad.materials()
bearcad.mirror_bodies{ plane, bodies, output, name }
bearcad.mirror_sketch{ sketch, line, lines, circles }
bearcad.move_bodies{ bodies, images, x, y, z, rotate, rx, ry, rz, roll, flip, spin, gap, from, to, name }
bearcad.move_to_component{ kind, index, component }
bearcad.new()
bearcad.offset_sketch{ sketch, lines, circles, distance, construction }
bearcad.open(path)
bearcad.open_sketch(sketch)
bearcad.parameter_edit(target, field)
bearcad.parameter_editing()
bearcad.parameter_expression(target)
bearcad.parameter_from_line_length(line, name?)
bearcad.parameter_options(target, open?)
bearcad.parameter_slider(target, value?)
bearcad.parameter_value(target)
bearcad.paste{ linked, x, y, z }?
bearcad.plane{ offset, from, origin, normal, axis, angle, name }
bearcad.project{ entities, body, bodies, plane, planes, kind, index, name, type }?
bearcad.quit()
bearcad.rebuild_geometry()
bearcad.rect{ x, y, width, height, name }
bearcad.remove_calibration_point{ image, index }
bearcad.repeat_bodies{ bodies, axis, around, flip, mode, count, spacing, length, to, name }
bearcad.repeat_cut{ cuts, axis, around, flip, mode, count, spacing, length, to }
bearcad.repeat_sketch{ sketch, lines, circles, angle, dir, mode, count, spacing, length }
bearcad.repeat_sketches{ sketches, axis, around, flip, mode, count, spacing, length, to }
bearcad.revert_joint(op)
bearcad.revert_joints()
bearcad.revolve{ profiles, circle, circles, polygon, polygons, axis, symmetric, bodies, body, revolutions, angle, pitch, name }
bearcad.save(path?)
bearcad.section_plane{ view, plane, origin, normal, offset, roll, depth, flip, bodies, exclude_bodies }
bearcad.section_planes(view?)
bearcad.section_stats(index)
bearcad.select(…)
bearcad.selection()
bearcad.session_log(…)
bearcad.set_body_shadow{ body, shadow }
bearcad.set_construction(element, construction)
bearcad.set_joint_rest(op)
bearcad.set_material{ body, material }
bearcad.set_name(element, name)
bearcad.set_parameter(target, expression)
bearcad.set_unit_parameter{ instance, name, value, expression }
bearcad.set_units{ length, angle, component, sketch }
bearcad.set_visible(element, visible)
bearcad.shell{ bodies, faces, thickness, name }
bearcad.sketch_conflicts(sketch?)
bearcad.sketch_dof(sketch?)
bearcad.sketch_faces(sketch?)
bearcad.slice{ bodies, cutters, extend, name }
bearcad.slice_sketch{ sketch, lines, circles, faces, cutters }
bearcad.sphere{ index, shape, at, normal, u_axis, width, depth, height, size, r, radius, diameter, name }
bearcad.status()
bearcad.sweep{ … }
bearcad.sync_unit(value)
bearcad.text{ … }
bearcad.ui.add_geometric_constraint(name)
bearcad.ui.ai_mcp(how)
bearcad.ui.ai_pane_sections()
bearcad.ui.ai_sections(how)
bearcad.ui.angle_snap(degrees)
bearcad.ui.animate_joints(on?)
bearcad.ui.animate_zoom_to_fit(on?)
bearcad.ui.apply_construction(construction)
bearcad.ui.apply_visibility(visible)
bearcad.ui.auto_zoom(on?)
bearcad.ui.begin_combine{ op, a, b, keep_b }
bearcad.ui.begin_edit_section_plane{ view, cut }
bearcad.ui.begin_edit_shape{ index }
bearcad.ui.begin_joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }
bearcad.ui.begin_move{ bodies, images, x, y, z, rotate, rx, ry, rz, roll, flip, spin, gap, from, to }
bearcad.ui.camera{ yaw, pitch, distance, target, projection, shading, ground }?
bearcad.ui.changelog(verb?)
bearcad.ui.click(…)
bearcad.ui.click_ground(x, y, opts?)
bearcad.ui.click_world(x, y, z, opts?)
bearcad.ui.close_tab(index?)
bearcad.ui.commit_dim()
bearcad.ui.commit_plane()
bearcad.ui.complete_all_tutorials()
bearcad.ui.complete_tutorial(name)
bearcad.ui.constraint_shortcut(key)
bearcad.ui.context_menu()
bearcad.ui.context_row_rect(label)
bearcad.ui.detach_tab(index?)
bearcad.ui.double_click(…)
bearcad.ui.drag(…)
bearcad.ui.drag_gizmo{ … }
bearcad.ui.drag_ground(x0, y0, x1, y1)
bearcad.ui.drag_world(x0, y0, z0, x1, y1, z1)
bearcad.ui.drawing_loupe_rect{ view, index, magnified }
bearcad.ui.drawing_view_rect(view)
bearcad.ui.edit_dim(axis)
bearcad.ui.edit_plane(index)
bearcad.ui.elements_graph(opts?)
bearcad.ui.elements_row_rect(label)
bearcad.ui.elements_view(name)
bearcad.ui.exploder()
bearcad.ui.first_person(on?)
bearcad.ui.first_person_advance(seconds)
bearcad.ui.first_person_fly(on?)
bearcad.ui.first_person_jump()
bearcad.ui.first_person_look(dx, dy)
bearcad.ui.first_person_move{ … }
bearcad.ui.first_person_scale(scale)
bearcad.ui.focus_calibrate()
bearcad.ui.focus_dim(axis)
bearcad.ui.focus_name()
bearcad.ui.focused_window()
bearcad.ui.gizmo(name)
bearcad.ui.gizmos()
bearcad.ui.ground(value)
bearcad.ui.headless()
bearcad.ui.help(on?)
bearcad.ui.hovered()
bearcad.ui.install_age(days?)
bearcad.ui.key(name, opts?)
bearcad.ui.keydown(name)
bearcad.ui.keyup(name)
bearcad.ui.mcmaster(verb?, part?)
bearcad.ui.menu_item_rect(label)
bearcad.ui.menu_items()
bearcad.ui.menu_structure()
bearcad.ui.move(…)
bearcad.ui.move_ground(x, y)
bearcad.ui.move_preview()
bearcad.ui.move_world(x, y, z)
bearcad.ui.new_tab{ … }?
bearcad.ui.orbit(dx, dy)
bearcad.ui.os_open(path)
bearcad.ui.palette(… | { open })
bearcad.ui.pan(dx, dy)
bearcad.ui.pane(pane, visible)
bearcad.ui.pane_rect(pane)
bearcad.ui.pane_scroll(pane)
bearcad.ui.picker(name)
bearcad.ui.picker_focus(name)
bearcad.ui.pickers()
bearcad.ui.press(x, y)
bearcad.ui.press_world(x, y, z)
bearcad.ui.release()
bearcad.ui.reorder_tab(from, to)
bearcad.ui.repeat_tool{ axis, count, gap, spacing, distance, length, offset, to_end, computed, around, flip }
bearcad.ui.report_issue(verb?)
bearcad.ui.right_click(…)
bearcad.ui.right_click_ground(x, y)
bearcad.ui.right_drag(dx, dy)
bearcad.ui.right_drag_pan(dx, dy)
bearcad.ui.screenshot(…)
bearcad.ui.scroll_pane(pane, dy)
bearcad.ui.set_dim(axis, value)
bearcad.ui.set_dim_label_offset(axis, offset)
bearcad.ui.set_gizmo{ … }
bearcad.ui.set_home_view()
bearcad.ui.settings(verb?)
bearcad.ui.shading(name)
bearcad.ui.snapping(on?)
bearcad.ui.tab(index?)
bearcad.ui.tab_count()
bearcad.ui.tabs()
bearcad.ui.toggle_construction()
bearcad.ui.toggle_projection()
bearcad.ui.toggle_visibility()
bearcad.ui.tool(name?)
bearcad.ui.tool_hints(on?)
bearcad.ui.tool_mode(mode?)
bearcad.ui.toolbar_shortcuts()
bearcad.ui.toolbar_tools()
bearcad.ui.touch(on?)
bearcad.ui.tutorial(name)
bearcad.ui.tutorial_assist()
bearcad.ui.tutorial_bubble()
bearcad.ui.tutorial_end()
bearcad.ui.tutorial_highlight()
bearcad.ui.tutorial_narration()
bearcad.ui.tutorial_next()
bearcad.ui.tutorial_orb()
bearcad.ui.tutorial_pane(verb?)
bearcad.ui.tutorial_prompt(verb?, arg?)
bearcad.ui.tutorial_step()
bearcad.ui.tutorials()
bearcad.ui.type(text)
bearcad.ui.unstart_all_tutorials()
bearcad.ui.update_channel(channel?)
bearcad.ui.view(…)
bearcad.ui.view_home(…)
bearcad.ui.viewport(…)
bearcad.ui.wait(…)
bearcad.ui.wait_ms(…)
bearcad.ui.wheel(scroll)
bearcad.ui.window_count()
bearcad.ui.windows()
bearcad.ui.workbench(name?)
bearcad.ui.zoom_fit(…)
bearcad.undo()
bearcad.unit_link(unit, mode)
bearcad.version(…)
bearcad.visible(element)
```
