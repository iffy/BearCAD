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

A rectangle is four lines (bottom, right, top, left). Drawing verbs open a ground-plane
sketch when none is active. An operation that consumes a body produces a new one, so
the index moves: chain off `bearcad.count("body") - 1` or use names.

## Shapes (no sketch)

A cube/box/block is `cuboid`, not `box` or `cube`. It sits on the plane at `at` (the
base centre, default origin) and grows along `normal` (default +Z, so on the ground).
A 10 mm cube sitting on the ground, centred on the origin in XY:

```lua
bearcad.cuboid{ width = 10, depth = 10, height = 10 }
```

```
bearcad.cuboid{ width, depth, height, at = {x,y,z}?, normal?, u_axis?, name? }
bearcad.cylinder{ radius, height, at?, normal?, name? }
bearcad.sphere{ radius, at?, name? }
bearcad.edit_shape{ index, shape = "cuboid"|"cylinder"|"sphere"?, width?, depth?, height?, radius?, at?, normal?, u_axis?, name? }
```

Every dimension takes a number or an expression string.

## Sketching

```
bearcad.rect{ width, height, x = 0?, y = 0?, name? }
bearcad.line{ x, y, x1, y1, name?, dimension? }          -- or length + angle (degrees)
bearcad.circle{ x, y, r | radius | diameter, name? }
bearcad.text{ text, x, y, size?, font?, bold?, italic?, underline?, rotation?, wrap?, flip?, name? }
bearcad.begin_sketch("construction_plane", i)
bearcad.begin_sketch{ kind = "extrude_cap"|"extrude_side"|…, … }
bearcad.open_sketch(i)
bearcad.exit_sketch()
bearcad.plane{ offset?, from = 0?, origin = {x,y,z}?, normal = {x,y,z}?, name? }
bearcad.project{ body?, bodies?, plane?, planes?, entities? }
```

`dimension` on a line locks its length (number, expression, or `true` for as-drawn).

## Solids

```
bearcad.extrude{ polygon = {line, …} | circle = i | circles = {i, …} | text = i | boolean = {…}, distance?, to?, body = "new"|"add"|"cut"|"join"?, name?, symmetric?, taper?, taper_mode = "distance"|"angle"? }
bearcad.edit_extrusion{ extrusion, distance? | by? | to? }
bearcad.extrude_face{ face = {…}, distance, body? }
bearcad.revolve{ polygon = {…} | circle = i | circles = {i, …}, axis = "x"|"y"|"z"|{ line = i }, angle? | revolutions?, pitch?, body = "new"|"add"|"cut"?, bodies?, symmetric?, name? }
bearcad.sweep{ polygon = {…} | circle = i | circles = {i, …}, path = {line, …}, body = "add"|"cut"?, bodies? }
bearcad.loft{ circles = {i, …}?, polygons = { {line, …}, … }?, body? }
bearcad.combine{ op = "union"|"cut"|"intersect"|"xor", a = {i, …}, b = {i, …}, keep_b?, bake?, name? }   -- `difference` means cut; bake = true consumes the inputs and leaves one standalone body
bearcad.slice{ bodies = {i, …}, cutters = {…}, extend?, name? }
bearcad.shell{ bodies = {i, …}, faces = {…}?, thickness, name? }
bearcad.move_bodies{ bodies = {i, …}, x?, y?, z?, rx?, ry?, rz?, name? }
bearcad.mirror_bodies{ plane = i, bodies = {i, …}, output = "new"|"add"|"cut"?, name? }
bearcad.repeat_bodies{ bodies = {i, …}, axis = "x"|"y"|"z", mode?, count?, spacing? | gap?, length?, around?, flip?, to?, name? }
```

To cut a hole: sketch on a face, then `extrude{ …, body = "cut" }`. A cut pointing away
from the body is flipped inward.

Rounding is one call per operation — a set of edges in a single call, never one call per
edge (four calls would make four bodies):

```
bearcad.fillet_edge{ extrusion = i, edges = { { kind = "vertical"|"top"|"bottom", face = i, edge = i }, … }, radius }
bearcad.chamfer_edge{ extrusion = i, edges = { … }, distance }
bearcad.extrude_edges(i)               -- the edge refs fillet/chamfer accept on extrusion i
bearcad.fillet_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, radius }
bearcad.chamfer_vertex{ point = { kind = "line", index = i, endpoint = "start"|"end" }, distance }
```

Shape-tool cuboids use the same edge calls with `kind = "vertical"` etc. on the primitive.

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
bearcad.get{ kind, index }         --   construction_plane, extrusion, revolution, sweep,
                                   --   loft, combine, move, mirror, repeat, slice, shell,
                                   --   edge_treatment, sketch_offset, sketch_mirror,
                                   --   sketch_repeat, sketch_slice, sketch_chamfer, shape,
                                   --   body, drawing, cross_section, section_plane, parameter,
                                   --   sketch_text, component, image, joint, unit_instance.
                                   --   aliases: plane, revolve, boolean, primitive, text,
                                   --   tracing_image, sketch_fillet, unit, offset.
                                   --   not chamfer/fillet (use edge_treatment or sketch_chamfer).
                                   --   `count` and `get` take the same set.
bearcad.find("name")
bearcad.set_name(el, "name")
bearcad.element("line", i)         -- or bearcad.element(id) / bearcad.element(name)
bearcad.id(el)                     -- el:id(): a stable id, unique and never reused
bearcad.line_endpoints(i)          -- x0, y0, x1, y1
bearcad.image_corners(i)           -- tracing image quad in world mm, live Move included
bearcad.body_stats(i)              -- volume, triangles, bbox = { min = {x,y,z}, max = {x,y,z} }
bearcad.body_faces(i)
bearcad.drawing_views(i)           -- a drawing's page: orientation, style, dimensions
bearcad.body_edges(i)
bearcad.body_cylinders(i)
bearcad.selection()
bearcad.pickers()                  -- active tool pickers; bearcad.picker("Targets") for one
bearcad.gizmos()                   -- live gizmo rows; bearcad.gizmo("move_rz") for one
bearcad.visible(el)                -- effective visibility, component chain included
bearcad.sketch_dof()
bearcad.sketch_conflicts()
bearcad.status()
```

Never assume a call did what you meant: read it back and assert.

## Handles

A creation call hands back what it made: one element, or a list of them.

```
local sides = bearcad.rect{ x = 0, y = 0, width = 20, height = 10 }   -- four lines
local box   = bearcad.extrude{ polygon = sides, distance = 5 }        -- the new body
box:kind()  box:index()  box:id()  box:name()  box:exists()  box:delete()  tostring(box)
bearcad.delete(box)                -- or a list; does not replace the scene selection
bearcad.delete_selection()         -- whatever is selected (the GUI Delete)
```

Ordinals shift when elements are deleted, and a solid op consumes the body it acts on.
A handle does not: it names the same element until that element is gone, and then says so.
Anywhere an index is accepted — `bodies`, `polygon`, `extrusion`, `{ kind, index }`, … —
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
bearcad.ui.double_click(x, y)               -- opens a sketch / plane / dimension for editing
bearcad.ui.viewport()                       -- { width, height, x, y } of the area clicks address
bearcad.ui.right_click_ground(x, y)         -- opens a context menu
bearcad.ui.context_menu()                   -- { kind, index } of the open menu, or nil
bearcad.ui.key("enter")
bearcad.ui.palette("Export STEP")
```

## Every function

Every function BearCAD exposes, with the arguments it takes — a name that is
not in this list is not a function. `{ … }` is one options table; `?` marks an
optional argument or key. The sections above carry the detail for the calls they
cover.

```
bearcad.add_parameter(name, expression)
bearcad.add_unit_instance{ unit, name }
bearcad.apply_construction(construction)
bearcad.apply_visibility(visible)
bearcad.bake(index)   -- freeze a boolean's result into standalone bodies and consume its inputs
bearcad.begin_combine{ op, a, b, keep_b }
bearcad.begin_edit_section_plane{ view, cut }
bearcad.begin_joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }   -- face = { moving, fixed, flip?, offset?, spin? }
bearcad.begin_move{ bodies, images, x, y, z, rx, ry, rz, roll, flip, spin, gap, from, to, from_b, to_b, from_c, to_c }   -- from/to are { body = i, vertex = {x,y,z} } | { body = i, edge = {{x,y,z},{x,y,z}} } | { origin = true }
bearcad.begin_sketch(…)
bearcad.body_cylinders(index)
bearcad.body_edges(index)
bearcad.body_faces(index)
bearcad.body_stats(index)
bearcad.calibrate_image{ image, from, to, length }
bearcad.calibration_point{ image, index, x, y }
bearcad.chamfer_edge{ edges, edge, extrusion, primitive, distance }
bearcad.chamfer_vertex{ points, point, distance }
bearcad.circle{ x, y, r, radius, diameter, name }
bearcad.clear()
bearcad.clear_selection()
bearcad.clone_unit_instance{ instance }
bearcad.combine{ op, a, b, keep_b, name }
bearcad.commit_dim()
bearcad.commit_plane()
bearcad.component{ name, parent }?
bearcad.constrain(kind, a, b, …)
bearcad.constraint_shortcut(key)
bearcad.copy()
bearcad.count(kind)
bearcad.count_saved(kind)
bearcad.cross_section{ name }
bearcad.cuboid{ width, depth, height, at?, normal?, u_axis?, name? }
bearcad.cylinder{ radius, height, at?, normal?, u_axis?, name? }
bearcad.debug.mesh_cache()
bearcad.debug.session_writes()
bearcad.debug.sqlite_scalar(sql)
bearcad.debug.tool_row()
bearcad.debug.tool_table()
bearcad.debug.widget_id_warnings()
bearcad.delete(element_or_list)
bearcad.delete_drawing_loupe{ drawing, view, index }
bearcad.delete_parameter(name)
bearcad.delete_section_plane{ view, cut }
bearcad.delete_selection()
bearcad.derive_parameter{ kind, a, b, body, body_b, name, instance, face, edge }
bearcad.dimension{ kind, value, index, a, b, sign, point, line, anchor, mover }
bearcad.drag_gizmo{ name, by }
bearcad.drag_line({ … }, anchor_u?, anchor_v?, u?, v?)
bearcad.drag_vertex({ … }, u?, v?)
bearcad.drawing{ name }?
bearcad.drawing_align_view{ drawing, parent, dir, pos }
bearcad.drawing_angle{ drawing, view, edge1, edge2 }
bearcad.drawing_circle_dim_offset{ drawing, view, center, offset }
bearcad.drawing_circle_dimension{ drawing, view, center }
bearcad.drawing_curve_dimension{ drawing, view, points }
bearcad.drawing_dim_offset{ drawing, view, offset, a, b }
bearcad.drawing_dimension{ drawing, view, a, b }
bearcad.drawing_loupe{ drawing, view, at, radius, to, to_radius }   -- at/to are {u, v} in the view's projected mm
bearcad.drawing_loupe_dimension{ drawing, view, index, a, b }   -- show/hide one edge's length dimension on a loupe; a/b are its world endpoints
bearcad.drawing_loupes{ drawing, view }   -- { at, radius, to, to_radius, zoom, style, dimensions } each
bearcad.drawing_move_view{ drawing, view, x, y }
bearcad.drawing_page{ drawing, width, height, margin }
bearcad.drawing_paper{ drawing, paper }   -- white | dark (the editor's sheet; exports are always white)
bearcad.drawing_point_dim_offset{ drawing, view, index, offset }
bearcad.drawing_point_dimension{ drawing, view, a, b, axis }
bearcad.drawing_point_dimension_axis{ drawing, view, index, axis }
bearcad.drawing_style{ drawing, style }              -- the style projections added to this page start in
bearcad.drawing_text{ drawing, text, x, y, wrap? }   -- x/y are page fractions (0–1)
bearcad.drawing_view{ drawing, body, bodies, component, sketch, cross_section, orientation }
bearcad.drawing_view_add{ drawing, view, body, bodies, component }
bearcad.drawing_view_align_lines{ drawing, view, show }
bearcad.drawing_view_label{ drawing, view, hidden, pos, text }
bearcad.drawing_view_lines{ drawing, view }          -- the lines the view draws: { x1, y1, x2, y2 } in view mm, hidden lines already removed
bearcad.drawing_view_orientation{ drawing, view, orientation }
bearcad.drawing_view_section{ drawing, view, cross_section }
bearcad.drawing_view_size{ drawing, view, width, height, size_x, size_y }
bearcad.drawing_view_style{ drawing, view, style }   -- visible, wireframe, shaded, colorful, loose_pencil, color_pencil, watercolor
bearcad.drawing_views(index)
bearcad.edit_boolean{ index, op, a, b, keep_b }
bearcad.edit_dim(axis)
bearcad.edit_drawing_loupe{ drawing, view, index, at?, radius?, to?, to_radius?, style? }   -- style is a drawing view style, or "view" to follow the projection's
bearcad.edit_extrusion{ extrusion, distance, by, to }
bearcad.edit_joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }   -- face = { moving, fixed, flip?, offset?, spin? }
bearcad.edit_mirror{ index, plane, bodies, output }
bearcad.edit_move{ index, bodies, images, x, y, z, rx, ry, rz, roll, flip, spin, gap, from, to, from_b, to_b, from_c, to_c }   -- from/to are { body = i, vertex = {x,y,z} } | { body = i, edge = {{x,y,z},{x,y,z}} } | { origin = true }
bearcad.edit_parameter{ name, private, min, max, step, rename }
bearcad.edit_plane(index)
bearcad.edit_repeat{ index, bodies, axis, around, flip, mode, count, spacing, gap, length, to }
bearcad.edit_section_plane{ view, cut, offset, roll, depth, flip, bodies, exclude_bodies }
bearcad.edit_shape{ index, shape, at, normal, u_axis, width, depth, height, radius, name }
bearcad.edit_shell{ index, bodies, faces, thickness }
bearcad.edit_sketch_mirror{ index, sketch, line, lines, circles }
bearcad.edit_sketch_offset{ index, sketch, lines, circles, distance, construction }
bearcad.edit_sketch_repeat{ index, sketch, lines, circles, angle, dir, mode, count, spacing, gap, length }
bearcad.edit_sketch_slice{ index, lines, circles, faces, cutters }
bearcad.edit_slice{ index, bodies, cutters, extend }
bearcad.element(kind, index)   -- or bearcad.element(id_or_name)
bearcad.exit_sketch()
bearcad.exploder()
bearcad.export_3mf(path, body?)
bearcad.export_drawing_pdf{ drawing, path }
bearcad.export_drawing_svg{ drawing, path }
bearcad.export_preview(path)
bearcad.export_step(path, body?)
bearcad.export_stl(path, body?)
bearcad.extrude{ distance, to, circle, circles, polygon, text, boolean, body, name, symmetric, taper, taper_mode }
bearcad.extrude_edges(index)   -- edge refs fillet_edge/chamfer_edge accept: kind vertical|top|bottom
bearcad.extrude_face{ to, distance, body, name }
bearcad.fillet_edge{ edges, edge, extrusion, primitive, radius }
bearcad.fillet_vertex{ points, point, radius }
bearcad.find(name)
bearcad.get{ kind, index }
bearcad.gizmo(name)
bearcad.gizmos()
bearcad.globals()
bearcad.hovered()
bearcad.id(element)   -- the element's stable, never-reused id
bearcad.image_corners(index)
bearcad.image_opacity{ image, opacity }
bearcad.import_image("path" | { path, plane? })
bearcad.import_lua(value)
bearcad.import_step(path)
bearcad.import_stl(path)
bearcad.import_unit(value)
bearcad.joint{ index, a, b, parts, kind, lead, base, face, line_up, frame_origin, frame_axis, frame_axis2, position, position2, position3, slide_min, slide_max, slide_min_to, slide_max_to, turn_min, turn_max, name }   -- face = { moving, fixed, flip?, offset?, spin? }
bearcad.line{ x, y, x1, y1, length, angle, bezier, dimension, name }
bearcad.line_endpoints(index)
bearcad.loft{ circle, circles, polygon, polygons, bodies, body, name }
bearcad.material{ name, color, bodies }   -- no color + an existing name applies that material
bearcad.mirror_bodies{ plane, bodies, output, name }
bearcad.mirror_sketch{ sketch, line, lines, circles }
bearcad.move_bodies{ bodies, images, x, y, z, rx, ry, rz, roll, flip, spin, gap, from, to, from_b, to_b, from_c, to_c, name }   -- from/to are { body = i, vertex = {x,y,z} } | { body = i, edge = {{x,y,z},{x,y,z}} } | { origin = true }
bearcad.move_preview()
bearcad.move_to_component{ kind, index, component }
bearcad.new()
bearcad.offset_sketch{ sketch, lines, circles, distance, construction }
bearcad.open(path)
bearcad.open_sketch(sketch)
bearcad.parameter_edit(name, field)
bearcad.parameter_editing()
bearcad.parameter_expression(name)
bearcad.parameter_from_line_length(line, name?)
bearcad.parameter_options(name, open?)
bearcad.parameter_slider(name, value?)
bearcad.parameter_value(name)
bearcad.paste{ linked, x, y, z }?
bearcad.picker(name)
bearcad.pickers()
bearcad.plane{ offset, from, origin, normal, name }
bearcad.project{ entities, body, bodies, plane, planes, kind, index, name, type }?
bearcad.quit()
bearcad.rebuild_geometry()
bearcad.rect{ x, y, width, height, name }
bearcad.remove_calibration_point{ image, index }
bearcad.repeat_bodies{ bodies, axis, around, flip, mode, count, spacing, gap, length, to, name }
bearcad.repeat_cut{ cuts, axis, around, flip, mode, count, spacing, gap, length, to }
bearcad.repeat_sketch{ sketch, lines, circles, angle, dir, mode, count, spacing, gap, length }
bearcad.repeat_sketches{ sketches, axis, around, flip, mode, count, spacing, gap, length, to }
bearcad.revert_joint(op)
bearcad.revert_joints()
bearcad.revolve{ circle, circles, polygon, axis, symmetric, bodies, body, line, revolutions, angle, pitch, offset, gap, name }
bearcad.save(path?)
bearcad.section_plane{ view, plane, origin, normal, offset, roll, depth, flip, bodies, exclude_bodies }   -- depth: how far the cut reaches; omitted or false cuts through
bearcad.section_planes(view?)   -- view: cross-section index or name; omitted = the view being edited
bearcad.section_stats(index)   -- body as the open cross-section shows it: { volume, triangles, bbox }
bearcad.select(…)
bearcad.selection()
bearcad.session_log()
bearcad.set_body_shadow{ body, shadow }
bearcad.set_construction(element, construction)
bearcad.set_dim(axis, value)
bearcad.set_dim_label_offset(axis, offset)
bearcad.set_gizmo{ name, value }
bearcad.set_joint_rest(op)
bearcad.set_material{ body, material }
bearcad.set_name(element, name)
bearcad.set_parameter(name, expression)
bearcad.set_unit_parameter{ instance, name, value, expression }
bearcad.set_units{ length, angle, component, sketch }
bearcad.set_visible(element, visible)   -- element = { kind = "plane" } (no index): every element of that kind
bearcad.shell{ bodies, faces, thickness, name }
bearcad.sketch_conflicts(sketch?)
bearcad.sketch_dof(sketch?)
bearcad.slice{ bodies, cutters, extend, name }
bearcad.slice_sketch{ sketch, lines, circles, faces, cutters }
bearcad.sphere{ radius, at?, name? }
bearcad.status()
bearcad.sweep{ circle, circles, polygon, path, bodies, body, name }
bearcad.sync_unit(value)
bearcad.text{ text, x, y, size, font, bold, italic, underline, rotation, wrap, flip, name }
bearcad.toggle_construction()
bearcad.toggle_visibility()
bearcad.ui.add_geometric_constraint(name)
bearcad.ui.ai_mcp(how)
bearcad.ui.ai_pane_sections()
bearcad.ui.ai_sections(how)
bearcad.ui.angle_snap(degrees)
bearcad.ui.animate_joints(on?)
bearcad.ui.animate_zoom_to_fit(on?)
bearcad.ui.auto_zoom(on?)
bearcad.ui.camera{ yaw, pitch, distance, target, projection, shading, ground }?   -- no fields = read
bearcad.ui.changelog(verb?)
bearcad.ui.click(x, y, { shift?, ctrl?, cmd? }?)
bearcad.ui.click_ground(x, y, { shift?, ctrl?, cmd? }?)
bearcad.ui.click_world(x, y, z, { shift?, ctrl?, cmd? }?)
bearcad.ui.close_tab(index?)
bearcad.ui.complete_all_tutorials()
bearcad.ui.complete_tutorial(name)
bearcad.ui.context_menu()
bearcad.ui.context_row_rect(label)
bearcad.ui.detach_tab(index?)
bearcad.ui.double_click(x, y)
bearcad.ui.drag(x0, y0, x1, y1)
bearcad.ui.drag_ground(x0, y0, x1, y1)
bearcad.ui.drag_line({ … }, anchor_u?, anchor_v?, u?, v?)
bearcad.ui.drag_vertex({ … }, u?, v?)
bearcad.ui.drag_world(x0, y0, z0, x1, y1, z1)
bearcad.ui.drawing_loupe_rect{ view, index, magnified? }   -- { x, y, w, h, band, handle } — the circle's box in window coords, the px of rim that resizes it, and the centre dot that moves it
bearcad.ui.drawing_view_rect(view)
bearcad.ui.elements_graph{ shadow_bodies? }
bearcad.ui.elements_row_rect(label)
bearcad.ui.elements_view(name)
bearcad.ui.first_person(on?)
bearcad.ui.first_person_advance(seconds)
bearcad.ui.first_person_fly(on?)
bearcad.ui.first_person_jump()
bearcad.ui.first_person_look(dx, dy)
bearcad.ui.first_person_move{ forward, strafe }
bearcad.ui.first_person_scale(scale)
bearcad.ui.focus_calibrate()
bearcad.ui.focus_dim(axis)
bearcad.ui.focus_name()
bearcad.ui.focused_window()
bearcad.ui.ground(name)
bearcad.ui.headless()   -- true when the run has no OS window
bearcad.ui.help(on?)
bearcad.ui.install_age(days?)
bearcad.ui.key(name, { shift?, ctrl?, cmd? }?)
bearcad.ui.keydown(name, { shift?, ctrl?, cmd? }?)
bearcad.ui.keyup(name, { shift?, ctrl?, cmd? }?)
bearcad.ui.mcmaster(verb?, part?)
bearcad.ui.menu_item_rect(label)   -- { x, y, w, h } of that item, window coords
bearcad.ui.menu_items()   -- labels of the open context menu's items, in order
bearcad.ui.menu_structure()
bearcad.ui.move(x, y)
bearcad.ui.move_ground(x, y)
bearcad.ui.move_world(x, y, z)
bearcad.ui.new_tab{ same }?
bearcad.ui.orbit(dx, dy)
bearcad.ui.os_open(path)
bearcad.ui.palette(…)
bearcad.ui.pan(dx, dy)
bearcad.ui.pane(pane, visible)
bearcad.ui.pane_rect(pane)
bearcad.ui.pane_scroll(pane)
bearcad.ui.picker_focus(name)
bearcad.ui.reorder_tab(from, to)
bearcad.ui.repeat_tool{ axis?, count?, gap?, distance?, offset?, to_end?, computed?, around?, flip? }
bearcad.ui.report_issue(verb?)
bearcad.ui.right_click(x, y, { shift?, ctrl?, cmd? }?)
bearcad.ui.right_click_ground(x, y, { shift?, ctrl?, cmd? }?)
bearcad.ui.right_drag(dx, dy)
bearcad.ui.right_drag_pan(dx, dy)
bearcad.ui.screenshot(path?, region?)
bearcad.ui.scroll_pane(pane, dy)
bearcad.ui.set_home_view()
bearcad.ui.settings(verb?)
bearcad.ui.shading(name)   -- wireframe, transparent, solid, solid_wireframe, realistic, loose_pencil, dark_pencil, color_pencil, watercolor
bearcad.ui.snapping(on?)
bearcad.ui.tab(index?)
bearcad.ui.tab_count()
bearcad.ui.tabs()
bearcad.ui.toggle_projection()
bearcad.ui.tool(name?) -- no name reads the armed tool
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
bearcad.ui.view_home()
bearcad.ui.viewport() -- { width, height, x, y }
bearcad.ui.wait(frames)
bearcad.ui.wait_ms(ms)
bearcad.ui.wheel(scroll)
bearcad.ui.window_count()
bearcad.ui.windows()
bearcad.ui.workbench(name)
bearcad.ui.zoom_fit()
bearcad.undo()
bearcad.unit_link(unit, mode)
bearcad.unit_override{ instance, name, value, expression }
bearcad.version()
bearcad.visible(element)   -- effective visibility, component chain included
```
