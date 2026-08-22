# BearCAD Lua API

Use only the functions listed here. Unknown option keys fail the call and name the
accepted ones. Prefer `bearcad.*` (declarative modeling) over `bearcad.ui.*`.

Lengths are millimetres. API angles are radians unless a call names a unit (`"45deg"`,
`"5in"` work anywhere an expression is accepted). Indices are creation-order ordinals
and shift when things are deleted — prefer names (`bearcad.find`) for anything you will
refer to twice. One operation per call (especially fillets, chamfers, booleans).

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
bearcad.line{ x, y, x1, y1, name?, dimension? }          -- or length + angle
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
bearcad.extrude{ polygon = {line, …} | circle = i | circles = {i, …} | text = i | boolean = {…}, distance?, to?, body = "new"|"merge"|"cut"|"join"?, name?, symmetric?, taper?, taper_mode = "distance"|"angle"? }
bearcad.edit_extrusion{ extrusion, distance? | by? | to? }
bearcad.extrude_face{ face = {…}, distance, body? }
bearcad.revolve{ polygon = {…} | circle = i | circles = {i, …}, axis = "x"|"y"|"z"|{ line = i }, angle? | revolutions?, pitch?, body = "new"|"add"|"cut"?, bodies?, symmetric?, name? }
bearcad.sweep{ polygon = {…} | circle = i | circles = {i, …}, path = {line, …}, body = "add"|"cut"?, bodies? }
bearcad.loft{ circles = {i, …}?, polygons = { {line, …}, … }?, body? }
bearcad.combine{ op = "union"|"difference"|"intersect"|"cut"|"join", a = {i, …}, b = {i, …}, keep_b?, keep_leftovers?, name? }
bearcad.slice{ bodies = {i, …}, cutters = {…}, extend?, name? }
bearcad.shell{ bodies = {i, …}, faces = {…}?, thickness, name? }
bearcad.move_bodies{ bodies = {i, …}, x?, y?, z?, rx?, ry?, rz?, name? }
bearcad.mirror_bodies{ plane = i, bodies = {i, …}, output = "new"|"join"|"cut"?, name? }
bearcad.repeat_bodies{ bodies = {i, …}, axis = "x"|"y"|"z", mode?, count?, spacing? | gap?, length?, around?, flip?, to?, name? }
```

To cut a hole: sketch on a face, then `extrude{ …, body = "cut" }`. A cut pointing away
from the body is flipped inward.

Rounding is one call per operation — a set of edges in a single call, never one call per
edge (four calls would make four bodies):

```
bearcad.fillet_edge{ extrusion = i, edges = { { kind = "vertical"|"top"|"bottom", face = i, edge = i }, … }, radius }
bearcad.chamfer_edge{ extrusion = i, edges = { … }, distance }
bearcad.fillet_vertex{ point = { kind = "line", index = i, ["end"] = "start"|"end" }, radius }
bearcad.chamfer_vertex{ point = { kind = "line", index = i, ["end"] = "start"|"end" }, distance }
```

Shape-tool cuboids use the same edge calls with `kind = "vertical"` etc. on the primitive.

## Parameters and constraints

```
bearcad.parameter("add", "w", "24")
bearcad.parameter("value", i, "30")
bearcad.select{ kind, index }                          -- second arg true = add
bearcad.add_constraint({ kind = "line", index = i }, "25mm")
bearcad.add_geometric_constraint("parallel"|"perpendicular"|"equal"|"coincident"|"midpoint"|"horizontal"|"vertical")
bearcad.add_angle_constraint{ a = i, b = i, value }
```

Anywhere a size is accepted, an expression string is too.

## Inspect

```
bearcad.count("body"|"line"|"circle"|"sketch"|"constraint"|"parameter"|…)
bearcad.get{ kind, index }
bearcad.find("name")
bearcad.set_name(el, "name")
bearcad.element("line", i)
bearcad.line_endpoints(i)          -- x0, y0, x1, y1
bearcad.image_corners(i)           -- tracing image quad in world mm, live Move included
bearcad.body_stats(i)              -- volume, triangles, bbox
bearcad.body_faces(i)
bearcad.body_edges(i)
bearcad.body_cylinders(i)
bearcad.selection()
bearcad.sketch_dof()
bearcad.sketch_conflicts()
bearcad.status()
```

Never assume a call did what you meant: read it back and assert.

## Files

```
bearcad.new()
bearcad.open("part.bearcad")
bearcad.save()                     -- or save("other.bearcad")
bearcad.undo()
bearcad.import_step("part.step")
bearcad.import_stl("part.stl")
bearcad.export_step("out.step")
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
bearcad.ui.camera{ yaw?, pitch?, distance?, target? }
bearcad.ui.pane("ai"|"hierarchy"|"context"|"parameters"|…, "show"|"hide"|"toggle")
bearcad.ui.click_ground(x, y)               -- sketch-plane millimetres
bearcad.ui.click_world(x, y, z)             -- any world point: a body's side wall, say
bearcad.ui.viewport()                       -- { width, height } of the area clicks address
bearcad.ui.right_click_ground(x, y)         -- opens a context menu
bearcad.ui.context_menu()                   -- { kind, index } of the open menu, or nil
bearcad.ui.key("enter")
bearcad.ui.palette("Export STEP")
```

## Every function

A name that is not in this list is not a function:

- `bearcad.add_angle_constraint`
- `bearcad.add_constraint`
- `bearcad.add_geometric_constraint`
- `bearcad.add_unit_instance`
- `bearcad.apply_construction`
- `bearcad.apply_visibility`
- `bearcad.begin_combine`
- `bearcad.begin_joint`
- `bearcad.begin_move`
- `bearcad.begin_sketch`
- `bearcad.body_cylinders`
- `bearcad.body_edges`
- `bearcad.body_faces`
- `bearcad.body_stats`
- `bearcad.calibrate_image`
- `bearcad.calibration_point`
- `bearcad.chamfer_edge`
- `bearcad.chamfer_vertex`
- `bearcad.circle`
- `bearcad.clear`
- `bearcad.clear_selection`
- `bearcad.clone_unit_instance`
- `bearcad.combine`
- `bearcad.commit_dim`
- `bearcad.commit_plane`
- `bearcad.component`
- `bearcad.constraint_shortcut`
- `bearcad.copy`
- `bearcad.count`
- `bearcad.cuboid`
- `bearcad.cylinder`
- `bearcad.delete_selection`
- `bearcad.derive_parameter`
- `bearcad.drag_gizmo`
- `bearcad.drag_line`
- `bearcad.drag_vertex`
- `bearcad.drawing`
- `bearcad.drawing_align_view`
- `bearcad.drawing_angle`
- `bearcad.drawing_circle_dim_offset`
- `bearcad.drawing_circle_dimension`
- `bearcad.drawing_dim_offset`
- `bearcad.drawing_dimension`
- `bearcad.drawing_move_view`
- `bearcad.drawing_page`
- `bearcad.drawing_point_dimension`
- `bearcad.drawing_point_dimension_axis`
- `bearcad.drawing_text`
- `bearcad.drawing_view`
- `bearcad.drawing_view_add`
- `bearcad.drawing_view_align_lines`
- `bearcad.drawing_view_label`
- `bearcad.drawing_view_size`
- `bearcad.edit_boolean`
- `bearcad.edit_dim`
- `bearcad.edit_extrusion`
- `bearcad.edit_joint`
- `bearcad.edit_mirror`
- `bearcad.edit_move`
- `bearcad.edit_plane`
- `bearcad.edit_repeat`
- `bearcad.edit_shape`
- `bearcad.edit_shell`
- `bearcad.edit_sketch_mirror`
- `bearcad.edit_sketch_offset`
- `bearcad.edit_sketch_repeat`
- `bearcad.edit_sketch_slice`
- `bearcad.edit_slice`
- `bearcad.element`
- `bearcad.exit_sketch`
- `bearcad.exploder`
- `bearcad.export_3mf`
- `bearcad.export_drawing_pdf`
- `bearcad.export_drawing_svg`
- `bearcad.export_preview`
- `bearcad.export_step`
- `bearcad.export_stl`
- `bearcad.extrude`
- `bearcad.extrude_face`
- `bearcad.fillet_edge`
- `bearcad.fillet_vertex`
- `bearcad.find`
- `bearcad.get`
- `bearcad.gizmos`
- `bearcad.hovered`
- `bearcad.image_corners`
- `bearcad.image_opacity`
- `bearcad.import`
- `bearcad.import_image`
- `bearcad.import_lua`
- `bearcad.import_step`
- `bearcad.import_stl`
- `bearcad.import_unit`
- `bearcad.joint`
- `bearcad.line`
- `bearcad.line_endpoints`
- `bearcad.loft`
- `bearcad.material`
- `bearcad.mesh_cache`
- `bearcad.mirror_bodies`
- `bearcad.mirror_sketch`
- `bearcad.move_bodies`
- `bearcad.move_preview`
- `bearcad.move_to_component`
- `bearcad.new`
- `bearcad.offset_sketch`
- `bearcad.open`
- `bearcad.open_sketch`
- `bearcad.parameter`
- `bearcad.paste`
- `bearcad.pickers`
- `bearcad.plane`
- `bearcad.project`
- `bearcad.quit`
- `bearcad.rebuild_geometry`
- `bearcad.rect`
- `bearcad.remove_calibration_point`
- `bearcad.repeat_bodies`
- `bearcad.repeat_cut`
- `bearcad.repeat_sketch`
- `bearcad.repeat_sketches`
- `bearcad.revert_joint`
- `bearcad.revert_joints`
- `bearcad.revolve`
- `bearcad.save`
- `bearcad.select`
- `bearcad.selection`
- `bearcad.session_writes`
- `bearcad.set_body_shadow`
- `bearcad.set_construction`
- `bearcad.set_dim`
- `bearcad.set_dim_label_offset`
- `bearcad.set_gizmo`
- `bearcad.set_joint_rest`
- `bearcad.set_material`
- `bearcad.set_name`
- `bearcad.set_unit_parameter`
- `bearcad.set_units`
- `bearcad.set_visible`
- `bearcad.shell`
- `bearcad.sketch_conflicts`
- `bearcad.sketch_dof`
- `bearcad.slice`
- `bearcad.slice_sketch`
- `bearcad.sphere`
- `bearcad.sqlite_scalar`
- `bearcad.status`
- `bearcad.sweep`
- `bearcad.sync_unit`
- `bearcad.text`
- `bearcad.toggle_construction`
- `bearcad.toggle_visibility`
- `bearcad.tool_row`
- `bearcad.tool_table`
- `bearcad.ui.ai_mcp`
- `bearcad.ui.ai_pane_sections`
- `bearcad.ui.ai_sections`
- `bearcad.ui.angle_snap`
- `bearcad.ui.animate_joints`
- `bearcad.ui.animate_zoom_to_fit`
- `bearcad.ui.auto_zoom`
- `bearcad.ui.camera`
- `bearcad.ui.changelog`
- `bearcad.ui.click`
- `bearcad.ui.click_ground`
- `bearcad.ui.click_world`
- `bearcad.ui.close_tab`
- `bearcad.ui.complete_all_tutorials`
- `bearcad.ui.complete_tutorial`
- `bearcad.ui.context_menu`
- `bearcad.ui.detach_tab`
- `bearcad.ui.drag`
- `bearcad.ui.drag_ground`
- `bearcad.ui.drag_line`
- `bearcad.ui.drag_vertex`
- `bearcad.ui.elements_view`
- `bearcad.ui.focus_calibrate`
- `bearcad.ui.focus_dim`
- `bearcad.ui.focus_name`
- `bearcad.ui.focused_window`
- `bearcad.ui.fps`
- `bearcad.ui.fps_advance`
- `bearcad.ui.fps_fly`
- `bearcad.ui.fps_jump`
- `bearcad.ui.fps_look`
- `bearcad.ui.fps_move`
- `bearcad.ui.fps_scale`
- `bearcad.ui.ground`
- `bearcad.ui.help`
- `bearcad.ui.install_age`
- `bearcad.ui.key`
- `bearcad.ui.keydown`
- `bearcad.ui.keyup`
- `bearcad.ui.mcmaster`
- `bearcad.ui.menu_structure`
- `bearcad.ui.move`
- `bearcad.ui.move_ground`
- `bearcad.ui.move_world`
- `bearcad.ui.new_tab`
- `bearcad.ui.orbit`
- `bearcad.ui.os_open`
- `bearcad.ui.palette`
- `bearcad.ui.pan`
- `bearcad.ui.pane`
- `bearcad.ui.pane_rect`
- `bearcad.ui.pane_scroll`
- `bearcad.ui.picker_focus`
- `bearcad.ui.reorder_tab`
- `bearcad.ui.report_issue`
- `bearcad.ui.right_click`
- `bearcad.ui.right_click_ground`
- `bearcad.ui.right_drag`
- `bearcad.ui.right_drag_pan`
- `bearcad.ui.screenshot`
- `bearcad.ui.scroll_pane`
- `bearcad.ui.set_home_view`
- `bearcad.ui.settings`
- `bearcad.ui.shading`
- `bearcad.ui.snapping`
- `bearcad.ui.tab`
- `bearcad.ui.tab_count`
- `bearcad.ui.tabs`
- `bearcad.ui.toggle_projection`
- `bearcad.ui.tool`
- `bearcad.ui.tool_hints`
- `bearcad.ui.tool_mode`
- `bearcad.ui.toolbar_shortcuts`
- `bearcad.ui.toolbar_tools`
- `bearcad.ui.touch`
- `bearcad.ui.tutorial`
- `bearcad.ui.tutorial_assist`
- `bearcad.ui.tutorial_bubble`
- `bearcad.ui.tutorial_end`
- `bearcad.ui.tutorial_highlight`
- `bearcad.ui.tutorial_narration`
- `bearcad.ui.tutorial_next`
- `bearcad.ui.tutorial_orb`
- `bearcad.ui.tutorial_pane`
- `bearcad.ui.tutorial_prompt`
- `bearcad.ui.tutorial_step`
- `bearcad.ui.tutorials`
- `bearcad.ui.type`
- `bearcad.ui.unstart_all_tutorials`
- `bearcad.ui.update_channel`
- `bearcad.ui.view`
- `bearcad.ui.view_home`
- `bearcad.ui.viewport`
- `bearcad.ui.wait`
- `bearcad.ui.wait_ms`
- `bearcad.ui.wheel`
- `bearcad.ui.widget_id_warnings`
- `bearcad.ui.window_count`
- `bearcad.ui.windows`
- `bearcad.ui.zoom_fit`
- `bearcad.undo`
- `bearcad.unit_link`
- `bearcad.unit_override`
- `bearcad.version`
