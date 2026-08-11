Combine the extrude, sweep, loft and revolve tools into a single multi-extrude tool. Keep the other tools around for now -- I just want to test and see if it's intuitive this way. The context menu will have the following fields:

| Name         | Input type |
|--------------|------------|
| Faces        | ElementPicker for faces just like the current extrude tool |
| Distance     | See below. |
| Around       | ElementPicker for any edge or axis about which the extrusion will rotate. Choosing something here will turn this into a revolve |
| Along        | ElementPicker for any contiguous lines/axes/curves that is not coplanar to the Faces. If selected, the extrusion will follow these. |
| Skew         | See below. Disabled by default. |
| Taper        | Taper angle/distance |
| Mode         | Rename from 'Output' field. Use the same options for add body, combine body and cut body. |
| Symmetric    | See below. |

A few of these fields can be toggled just like the Gap and Distance fields in the Repeat tool. Provide sensible icons for toggle-able fields.

## Distance / Target

The `Distance` field is the default and is a ValueInput for the amount to linearly extrude faces. It can toggle to be `Target` instead, in which case it becomes an ElementPicker like the current `Up to` ElementPicker, where you choose the element the extrusion goes to rather than a distance expression.

## Mode

If I begin extruding into a body instead of away (whether via linear, revolve, sweep or loft), change the mode to `Cut`. But if I've manually changed the `Mode` at any point from the time I started using this tool, don't automatically change it to anything. This flag resets every time I choose the extrude tool (from another tool). If I drag into a body (causing it to switch to `Cut`), then come back and drag out (and I haven't manually changed the mode), automatically change the mode back to `Combine` or `New` just like it works now. Don't auto change the mode when editing an existing extrusion.

## Linear extrude to

After the first face is chosen, if a face is chosen for the Faces elementpicker that is NOT coplanar with all existing faces, toggle `Distance` to `Target` and set the `Target` to the non-coplanar face.

## Revolve

This mode is activated when an `Around` axis is chosen. Choosing this clears the `Along` selection and removes any faces from `Faces` that are not coplanar with the first chosen face.  The following fields then become available:

`Gap` / `Offset` -- this is a toggle using the same icons as in the `Repeat` tool. This value is the amount the ending face is away from the starting face after one revolution.

The `Distance` field during a revolve determines if the revolve becomes a spring-like extrusion. For a distance of 0, it produces closed revolutions. But if the distance is non-zero, it becomes the distance the ending face is from the starting face after 360degrees of rotation.

If a revolve is being done, this is the distance along the axis change the label to `Angle` and the units to degrees. Clamp this value between -360deg and 360deg.


## Sweep

This mode is activated when an `Along` axis/path/edge is chosen. Choosing this clears the `Around` selection. This mode IS compatible with `Loft` iff the chosen `Along` intersects both the `Faces` plane and the `Target` plane. By default, the Sweep goes the distance of the `Along` from where it intersects the `Faces` plane to the end of the path. But if the `Along` is an infinite axis or the user chooses to, you can use the `Distance`/`Target` field to decide where the extrusion ends. A gizmo should be available along the path that I can use to set `Distance`.

If the `Along` path crosses the face plane mid-segment, you can only extrude one way or the other.

If the `Along` path never crosses the face plane, no extrusion is possible -- reject that as an option for the `Along` elementpicker.

If the `Along` is an infinite axis, choose a good starting value for `Distance` (10mm).

## Loft

If a `Target` non-coplanar face is chosen, and there's only one face chosen in `Faces`, enable the `Loft` field which is a radio letting you pick between doing no loft, linear loft and smooth loft (make good icons for these).

## Skew

This setting only applies to sweeps. It is checkbox.

Unchecked -- (default) source face is extruded to the end and rotated so that the cross-section of the extrusion is normal to the path of extrusion at every point along the way and so that the final face is normal to the path
Linear skew -- source face is extruded such that the cross-section all along the path is without any rotation of the face.

## Morph

This setting only applies to lofts. It is a radio selection (with nice icons) of the following options:

No morph -- default. Ending face is identical to starting face
Linear morph -- source face is linearly changed to ending face.
Smooth morph -- source face is smoothly changed to ending face (with nice easing) This is not allowed with the target is a point.

## Symmetric

This is a radio between no symmetry, even symmetry and odd symmetry. Use icons for these. Even symmetry means the starting face is treated like a mirror. Odd symmetry means, as viewed from the starting face edge-on, every movement above is done opposite. For example, a move up and to the right is also done down and to the left.

For linear extrusions, the distinction between even and odd is only pertinent when extruding to a non-parallel target plane/face.
For revolve extrusions, even symmetry means extrusion happens in reverse around the axis, completing the circle. Odd symmetry starts by making an S and if extruded completely becomes an infinity/8 symbol.

For symmetric extrusions, the `Distance` or `Angle` is the distance/angle of the total extrusion. If a `Target` is being used, go all the way to the target and match the amount in reverse symmetrically. 

## Taper

The amount the end face of the extrude is different than the start face. This can be either a distance unit or an angle unit. Show an icon and let me click to toggle (like the repeat tool) between distance or angle.

If it's a distance unit, it's the amount the end face is larger than the first along each dimension. For example, if the starting face is a 10x10 square and the taper is set to 5, then the ending face is 20x20 (5 added to each side). And if taper is -5, the ending face would be 0x0. If taper is less than -5, the ending face would still be 0x0. For circles, add to the radius, not the diameter.

If this is an angle, it must be greater than -90deg and less than 90deg. It defines the angle that each edge of the extrusion takes measured against the normal. So 0deg is parallel with the normal, 45deg sends the extrusion out at 45 degrees AWAY from the center of the face and -45deg sends each edge toward the center of the face. If degrees are more negative, the extrude distance is cut. For example, if I was extruding a 10x10 square a distance of 10:

- if taper was 0deg, ending face would be 10x10
- if taper was -45deg, ending face would be a 0x0 point at a distance of 5

If an irregular polygon or one with holes is tapered, join holes together, or taper to points, but don't invert anything.

## Special cases

To make this tool most intuitive, ensure the following -- assuming I just switched to the extrude tool with nothing selected:

- If I select a face and then immediately use the keyboard to type an expression, it should do a linear extrude for the distance in the expression.
- If I select a face and then select a non-coplanar face or point, it should set the `Target` to that face/point as a linear extrude.
- If I select multiple coplanar faces, and then select a non-coplanar face, it should set the `Target` to that face as a linear extrude.
- If I select one or more coplanar faces, then select an edge/axis that is parallel to the plane of those faces, set the edge/axis as the `Around` value and begin a revolve.
- When a revolve is started, start with a 360deg default value for the `Angle`.

## Clarifications

After I select a face:

- If a curve is chosen that is not coplanar to the face, it is set as the `Along` path.
- Coplanar curves are not selectable.
- If a world axis that is not parallel, set it as the `Along` target with a default extrusion amount
- If an edge/line that is neither parallel nor coplanar is chosen, set is as the `Along` target and extrude the face in a skewed way.
- If I'm in revolve mode and I choose a target, the only valid targets are faces that I can extrude to.
- If I choose `Around` when `Target` is already set, clear `Target` unless it's a valid target for a revolve
- Loft to a point is allowed
- Loft to an edge is allowed
- Taper + symmetry = taper both ends