# Ready to do

# Needs description

[ ] Snap to offset places (dashed lines)
[ ] Let me right click a Sketch to export to DXF. In the context pane, show options like "include construction lines".
[ ] Extrude
[ ] STL export
[ ] Step export
[ ] Technical drawings
[ ] Click lines multiple times to draw polygon (after snapping). Let me choose relative angles from last line (or from horizontal/vertical)

# Done

[X] When focus is on a variable in the variable pane (either input), highlight all the elements in the element pane that make use of that variable.
[X] Make the take screenshot function take a screenshot of just the 3D viewing space by default (without the bear HUD, if possible). A parameter to the function can be passed to take a screenshot of the whole window, too. If not filename is given, save the screenshot as `screenshot-le3.png`
[X] Add basic snapping. When drawing things or moving them, snap to nearby things (i.e. vertex to line, vertex to vertex, etc...). If the user decides to leave something at the snap point, add an appropriate constraint (e.g. coincident). One of the snaps to support is snapping to the midpoint of a line. Add a toggle to the context menu to enable/disable snapping and only show it when on a tool that uses it.
