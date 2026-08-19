//! Exhaustive `[ ]` test checklist (`bearcad testplan`).
//!
//! Print with `bearcad testplan`. Tutorials are listed from
//! [`crate::tutorial::TUTORIALS`] so a new walkthrough shows up automatically.
//! Extra one-off items go in [`CUSTOM_ITEMS`].

use crate::opsigs::{OpSpace, ALL_OPERATIONS};
use crate::tutorial::TUTORIALS;

/// One-off items included at the end of `bearcad testplan`.
///
/// Add a line here and it shows up as `[ ] …` under **Custom**. This is the place
/// for regressions, shop-specific checks, and anything the generated list does not
/// already cover.
pub const CUSTOM_ITEMS: &[&str] = &[
    // Example: "Open last week's bracket file and rebuild geometry",
];

struct Section {
    title: &'static str,
    items: &'static [&'static str],
}

/// Hand-written checks for tool options, file I/O, UI, and other actions that
/// operation signatures do not enumerate on their own.
const FEATURE_SECTIONS: &[Section] = &[
    Section {
        title: "Sketch tools",
        items: &[
            "Sketch: start a sketch on the ground plane",
            "Sketch: start a sketch on a construction plane",
            "Sketch: start a sketch on a body face (it moves with the body)",
            "Sketch: reopen a sketch by clicking its face with the Sketch tool",
            "Sketch: leave a sketch with Esc (orange viewport border gone)",
            "Rectangle: draw by two corners (corner-anchored)",
            "Rectangle: draw from the centre (centre-anchored); toggle with R",
            "Rectangle: type width and height, Tab between fields, Enter commits",
            "Rectangle: X makes construction geometry",
            "Line: chain segments and close by snapping to the start",
            "Line: curve mode (Cmd/Ctrl+B) draws a bezier",
            "Line: convert a corner to a bezier; straighten a curve",
            "Line: toggle a tangent constraint on a curve joint",
            "Circle: draw by centre and radius",
            "Circle: draw by two opposite rim points (diameter mode); toggle with O",
            "Circle: type a diameter",
            "Circle: X makes construction geometry",
            "Offset: parallel copy of sketch edges outward",
            "Offset: shrink a closed loop or circle (negative distance)",
            "Offset: click a face to take all of its edges at once",
            "Offset: emit construction copies (X)",
            "Fillet: round a sketch vertex",
            "Chamfer: cut a sketch vertex flat",
            "Fillet/Chamfer: re-click a vertex to drop it from the set",
            "Mirror: reflect sketch shapes across a line",
            "Mirror: reflect sketch shapes across a sketch axis (LX/LY)",
            "Projection: project a body edge into a sketch",
            "Projection: un-project projected lines with Enter",
            "Text: click to place grow-to-fit lettering in a sketch",
            "Text: drag a wrap-width box",
            "Text: edit string, font, bold/italic/underline, size, rotation, flip",
            "Text: drag the rotation handle while creating or editing",
            "Text: embed a parameter with {name}",
            "Text: extrude or cut glyph outlines",
            "Snapping: toggle in the context pane; snap to ends, midpoints, lines",
            "Construction geometry: X while drawing any sketch shape",
        ],
    },
    Section {
        title: "Dimensions and constraints",
        items: &[
            "Dimension: a line length",
            "Dimension: a circle diameter",
            "Dimension: the angle between two lines (Shift+click)",
            "Dimension: a point's distance from the sketch origin",
            "Dimension: against a host face's edges (sketch on a body face)",
            "Dimension: drag a label; double-click to edit",
            "Dimension: type an expression; create a parameter inline with name=value",
            "Dimension (3D): measure an edge length and derive a parameter",
            "Dimension (3D): measure distance between two vertices",
            "Dimension (3D): measure the angle between two edges",
            "Constraint: Parallel",
            "Constraint: Perpendicular",
            "Constraint: Equal",
            "Constraint: Coincident (point-point, point-line, point-circle, point-origin, collinear lines)",
            "Constraint: Midpoint",
            "Constraint: Parallel to X axis",
            "Constraint: Parallel to Y axis",
            "Constraint: Tangent (curve joint)",
            "Constraint: Angle (via the Dimension tool)",
            "Pin a point to the sketch origin; hold a point on a sketch axis",
            "Fully constrain a sketch (fully-constrained colour)",
        ],
    },
    Section {
        title: "Construction planes",
        items: &[
            "Construction plane: offset from a face (ground, plane, or body face)",
            "Construction plane: pivot around an edge or axis with an angle",
            "Construction plane: through a vertex, normal to a curve",
            "Construction plane: edit offset/angle later",
            "Hide a construction plane without hiding sketches on it",
        ],
    },
    Section {
        title: "3D shapes and solids",
        items: &[
            "Shape: place a cuboid (width, depth, height)",
            "Shape: place a cylinder (radius, height)",
            "Shape: place a sphere (radius)",
            "Shape: cycle cuboid → cylinder → sphere with B",
            "Shape: snap placement to a body corner or edge midpoint",
            "Extrude: pull a profile as a new body",
            "Extrude: join profiles into one body",
            "Extrude: merge into a host body (Add)",
            "Extrude: cut a hole through a body",
            "Extrude: a concentric ring (two circles)",
            "Extrude: a region divided by crossing sketch lines",
            "Extrude: drag the distance handle; type a distance expression",
            "Extrude: Flip to the other side of the sketch plane",
            "Extrude: distance taper (mm added per side)",
            "Extrude: angle taper (draft); clamp/warn past 89°",
            "Extrude: up to a face",
            "Extrude: up to a plane",
            "Extrude: up to a vertex",
            "Extrude: a body face without a new sketch",
            "Extrude: edit a committed extrusion (double-click)",
            "Loft: blend two or more profiles as a new body",
            "Loft: add to touching bodies",
            "Loft: cut bodies",
            "Loft: blend a circle into a rectangle",
            "Revolve: 360° as a new body",
            "Revolve: a partial angle; switch to revolutions (revs)",
            "Revolve: offset/gap coil (spring)",
            "Revolve: Symmetric",
            "Revolve: add to touching bodies",
            "Revolve: cut bodies",
            "Sweep: a profile along a straight path as a new body",
            "Sweep: along a curved path",
            "Sweep: add to touching bodies",
            "Sweep: cut bodies",
            "Combine: union two bodies",
            "Combine: cut (A − B)",
            "Combine: intersect",
            "Combine: difference (symmetric)",
            "Combine: Keep leftovers",
            "Move: Point Snap, A pair only (translate)",
            "Move: Point Snap A+B (translate and rotate)",
            "Move: Point Snap A+B+C (fully decided pose)",
            "Move: Point Snap roll angle instead of pair C",
            "Move: Face Snap with Flip and Turn",
            "Move: Free mode — typed XYZ, rotations, and gizmos",
            "Mirror: bodies as a new body",
            "Mirror: join (fuse the reflection)",
            "Mirror: cut",
            "Repeat: linear along an axis (count / gap / distance)",
            "Repeat: Flip the path direction",
            "Repeat: Distance-to a plane, face, or vertex",
            "Repeat: rotationally around an axis",
            "Repeat: rotational last-copy-at-angle vs ending-at-angle",
            "Repeat: a standalone extrude as separate bodies with their own materials",
            "Slice: with a construction plane",
            "Slice: with a flat face",
            "Slice: sketch-line laser cut",
            "Slice: Infinite cut on and off",
            "Slice: several cutters (crossing planes)",
            "Shell: closed hollow (wall thickness)",
            "Shell: with open faces",
            "Fillet: a body edge",
            "Fillet: a cylinder or hole rim",
            "Fillet: several edges at one radius",
            "Chamfer: a body edge",
            "Chamfer: several edges at one distance",
            "Edit fillet/chamfer/slice/repeat from the Elements pane",
        ],
    },
    Section {
        title: "Joints",
        items: &[
            "Joint: rigid (two parts)",
            "Joint: rigid group (three or more parts)",
            "Joint: slider",
            "Joint: revolute",
            "Joint: cylindrical",
            "Joint: planar",
            "Joint: ball",
            "Joint: pin-slot",
            "Joint: screw with a lead",
            "Joint: cycle types with J",
            "Joint: mate with Face Snap (Flip, Gap, Turn)",
            "Joint: mate with Point Snap",
            "Joint: mate with Free",
            "Joint: mate in place",
            "Joint: ground a part to a datum plane, world axis, or origin",
            "Joint: position as an expression; travel limits (min/max, stop face)",
            "Joint: drag a jointed part with Select",
            "Joint: animate motion; set and revert rest position",
            "Joint: a component; an imported unit instance",
        ],
    },
    Section {
        title: "Drawings",
        items: &[
            "Drawing: CAD → New Drawing",
            "Drawing: create from a body (right-click)",
            "Drawing projection: drop a body view",
            "Drawing projection: drop a sketch view",
            "Drawing projection: drop a component view",
            "Drawing projection: add another body (Shift-click)",
            "Drawing projection: set orientation (front / top / iso / …)",
            "Drawing align: aligned view down, up, left, and right of a parent",
            "Drawing align: projection lines between parent and child",
            "Drawing: resize a projection card; aligned partners share an axis",
            "Drawing dimension: edge length",
            "Drawing dimension: circle diameter",
            "Drawing dimension: angle between two edges",
            "Drawing: show/hide all dimensions on a view",
            "Drawing text: a page note with {parameter} expressions",
            "Drawing: edit a view caption label",
            "Drawing: page size and margins",
            "Drawing: export as PDF",
            "Drawing: export as SVG",
            "Drawing: delete a view",
        ],
    },
    Section {
        title: "Files, import and export",
        items: &[
            "New document; Open; Save; Save As a .bearcad file",
            "New tab; switch tabs (Cmd/Ctrl+1–9); close a dirty tab (save prompt)",
            "Duplicate tab (same document); move a tab to a new window",
            "Import an STL",
            "Import a STEP",
            "Import an image (PNG/JPEG) for tracing",
            "Import an image onto a construction plane",
            "Calibrate a tracing image's scale",
            "Import a Lua document script",
            "Import a BearCAD file as a unit",
            "Import the same unit a second time (second instance, shared copy)",
            "Clone a unit instance; override a unit parameter; restore the part's value",
            "Import a McMaster-Carr part",
            "Export STL (whole document, one body, a component)",
            "Export 3MF with material colours",
            "Export STEP (whole document, one body, a component)",
            "Export as Lua; File → Import Lua Script round-trip",
            "Rebuild Geometry (File menu and --rebuild)",
            "Document JSON (copy for a bug report)",
            "Load and run a Lua script from the File menu",
            "Dirty-document * in the window/tab title",
            "Quit with unsaved changes (Save / Don't Save / Cancel)",
            "Undo a feature as one step",
            "Copy the selection; paste an independent copy; paste a linked copy",
            "Shadow bodies are left out of whole-document export",
        ],
    },
    Section {
        title: "Parameters, units and materials",
        items: &[
            "Create a document parameter and reference it in a value field",
            "Expression arithmetic (+ − * / parentheses)",
            "Expression functions (max, min, abs, floor, ceil, round)",
            "Mixed units in an expression (3mm + 2in)",
            "Autocomplete parameter names",
            "Inline name=value creates a parameter",
            "Parameter min / max / step; min/max slider",
            "Private vs public parameter",
            "Hovering a parameter glows its users in the view",
            "Document default length and angle units",
            "Component unit override; sketch unit override",
            "Bare numbers use the document's default units",
            "Assign a built-in material to a body (click swatch or name)",
            "Create a new material; edit name and colour",
            "Mixed materials on a multi-body selection",
            "3MF export writes material colours",
        ],
    },
    Section {
        title: "Components",
        items: &[
            "New component; nest a component inside another",
            "Drag an element into a component; drop on Document to un-file",
            "Active component: new features land inside it",
            "Hide a component (hides contents)",
            "Delete a component (re-homes contents)",
            "Export a component to STL or STEP from its right-click menu",
            "Elements graph view: components as areas; type filter",
        ],
    },
    Section {
        title: "Navigation and view",
        items: &[
            "Orbit (right-drag); pan (middle-drag or Shift+right-drag); zoom (wheel)",
            "Zoom to fit; Auto-zoom",
            "View bear: click a face, edge, or corner for a standard view",
            "View bear Home; save the current view as Home",
            "Projection: orthographic and perspective",
            "Shading: wireframe, transparent, solid, solid + visible edges, realistic",
            "Ground: grid vs solid plane",
            "First-person (FPS) mode: walk, look, jump, fly",
            "Tool Hints overlay on and off",
            "Help Mode",
            "Touch: two-finger pan, pinch zoom, three-finger orbit",
            "Touch loupe while dragging; on-screen keypad for value fields",
            "Cycle app windows with Command+` (including McMaster-Carr)",
        ],
    },
    Section {
        title: "Selection and UI",
        items: &[
            "Select a body; additive select (Shift)",
            "Selection Exploder (Space): pick a face, hover, Shift-click, scroll loupes, dismiss",
            "Command palette: search and run a command",
            "Command palette: Search McMaster-Carr prompt",
            "Toggle Elements, Context, Parameters, Tutorials panes and the View Bear",
            "Elements pane: hide/show, rename, delete an element",
            "Elements pane: graph view; type filter; Unit contents filter",
            "Keyboard Shortcuts window",
            "Settings: library directory",
            "Settings: update channel (release vs pre-release)",
            "Settings: animate zoom to fit",
            "Changelog window; About; Licenses",
            "Install CLI from the Help menu",
            "Report Problem",
            "Follow a tutorial; skip a step; mark all complete / unstarted",
        ],
    },
    Section {
        title: "Scripting and CLI",
        items: &[
            "bearcad --script file.lua --exit",
            "bearcad --repl",
            "bearcad --show-commands",
            "bearcad --tutorial <name>",
            "bearcad --timeout <seconds>",
            "bearcad --rebuild",
            "bearcad opsigs (and opsigs --html)",
            "bearcad testplan",
            "bearcad install-cli / uninstall-cli",
            "Declarative modeling API (rect, extrude, joint, drawing, …)",
            "bearcad.ui.tool / click / screenshot",
            "Export Lua round-trip of a document",
        ],
    },
];

/// Render the full checklist (stdout of `bearcad testplan`).
pub fn render() -> String {
    render_with_custom(CUSTOM_ITEMS)
}

/// Same as [`render`], with a custom-item slice (so tests can inject a sentinel).
pub fn render_with_custom(custom: &[&str]) -> String {
    let mut out = String::from(
        "# BearCAD test plan\n\n\
         Extra one-off items: add a line to `CUSTOM_ITEMS` in `src/testplan.rs`.\n",
    );

    out.push_str("\n## Tools and operation variants\n");
    for sig in ALL_OPERATIONS {
        let space = match sig.space {
            OpSpace::TwoD => "2D",
            OpSpace::ThreeD => "3D",
        };
        if sig.variant.is_empty() {
            push_item(&mut out, &format!("{} ({space})", sig.tool_name()));
        } else {
            push_item(
                &mut out,
                &format!("{} — {} ({space})", sig.tool_name(), sig.variant),
            );
        }
    }

    for section in FEATURE_SECTIONS {
        out.push_str("\n## ");
        out.push_str(section.title);
        out.push('\n');
        for item in section.items {
            push_item(&mut out, item);
        }
    }

    out.push_str("\n## Tutorials\n");
    for tut in TUTORIALS {
        push_item(
            &mut out,
            &format!("Walk through tutorial: {} ({})", tut.title, tut.name),
        );
    }

    out.push_str("\n## Custom\n");
    if custom.is_empty() {
        out.push_str("# (none — add lines to CUSTOM_ITEMS in src/testplan.rs)\n");
    } else {
        for item in custom {
            push_item(&mut out, item);
        }
    }
    out
}

fn push_item(out: &mut String, text: &str) {
    out.push_str("[ ] ");
    out.push_str(text);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Tool;
    use crate::geometric_constraints::GeometricConstraintType;
    use crate::model::JointKind;
    use crate::opsigs::{tool_label, ALL_OPERATIONS};

    fn plan() -> String {
        render()
    }

    fn lines_with(plan: &str, needle: &str) -> Vec<String> {
        let n = needle.to_ascii_lowercase();
        plan.lines()
            .filter(|l| l.to_ascii_lowercase().contains(&n))
            .map(|l| l.to_string())
            .collect()
    }

    fn has_checkbox_mentioning(plan: &str, needle: &str) -> bool {
        lines_with(plan, needle)
            .iter()
            .any(|l| l.trim_start().starts_with("[ ]"))
    }

    fn has_tool_variant(plan: &str, tool: Tool, variant: &str) -> bool {
        let tool_name = tool_label(tool).to_ascii_lowercase();
        let variant = variant.to_ascii_lowercase();
        plan.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("[ ]") && l.contains(&tool_name) && l.contains(&variant)
        })
    }

    #[test]
    fn plan_is_a_checkbox_list() {
        let p = plan();
        assert!(
            p.lines().any(|l| l.trim_start().starts_with("[ ]")),
            "testplan should print [ ] checklist items"
        );
        assert!(
            !p.contains("[x]") && !p.contains("[X]"),
            "testplan items start unchecked"
        );
    }

    #[test]
    fn plan_is_comprehensive() {
        let n = plan()
            .lines()
            .filter(|l| l.trim_start().starts_with("[ ]"))
            .count();
        assert!(
            n >= 150,
            "testplan should be fairly comprehensive, got {n} items"
        );
    }

    #[test]
    fn every_tool_has_an_item() {
        let p = plan();
        for tool in Tool::ALL {
            let name = tool_label(tool);
            assert!(
                has_checkbox_mentioning(&p, name),
                "missing a [ ] item for tool {name}"
            );
        }
    }

    #[test]
    fn every_operation_variant_has_an_item() {
        let p = plan();
        for sig in ALL_OPERATIONS {
            if sig.variant.is_empty() {
                continue;
            }
            assert!(
                has_tool_variant(&p, sig.tool, sig.variant),
                "missing a [ ] item for {} ({})",
                tool_label(sig.tool),
                sig.variant
            );
        }
    }

    #[test]
    fn issue_examples_are_listed() {
        let p = plan();
        assert!(
            has_checkbox_mentioning(&p, "taper"),
            "should list testing the taper of an extrusion"
        );
        assert!(
            has_checkbox_mentioning(&p, "aligned"),
            "should list drawing an aligned view"
        );
        assert!(
            has_checkbox_mentioning(&p, "stl") && p.to_ascii_lowercase().contains("import"),
            "should list importing an STL"
        );
        assert!(
            has_checkbox_mentioning(&p, "step") && p.to_ascii_lowercase().contains("import"),
            "should list importing a STEP"
        );
    }

    #[test]
    fn every_joint_kind_has_an_item() {
        let p = plan();
        let kinds = [
            JointKind::Rigid,
            JointKind::Slider,
            JointKind::Revolute,
            JointKind::Cylindrical,
            JointKind::Planar,
            JointKind::Ball,
            JointKind::PinSlot,
            JointKind::Screw {
                lead: String::new(),
            },
        ];
        for kind in kinds {
            let name = match &kind {
                JointKind::Rigid => "rigid",
                JointKind::Slider => "slider",
                JointKind::Revolute => "revolute",
                JointKind::Cylindrical => "cylindrical",
                JointKind::Planar => "planar",
                JointKind::Ball => "ball",
                JointKind::PinSlot => "pin-slot",
                JointKind::Screw { .. } => "screw",
            };
            assert!(
                has_checkbox_mentioning(&p, name),
                "missing a [ ] item for joint kind {name}"
            );
        }
    }

    #[test]
    fn every_constraint_type_has_an_item() {
        let p = plan();
        for kind in GeometricConstraintType::ALL {
            assert!(
                has_checkbox_mentioning(&p, kind.label()),
                "missing a [ ] item for constraint {}",
                kind.label()
            );
        }
    }

    #[test]
    fn custom_items_are_included() {
        let p = render_with_custom(&["my one-off shop check"]);
        assert!(
            p.contains("[ ] my one-off shop check"),
            "CUSTOM_ITEMS must appear as [ ] lines:\n{p}"
        );
        assert!(
            p.to_ascii_lowercase().contains("custom"),
            "custom items should sit under a Custom heading"
        );
    }

    #[test]
    fn custom_items_const_is_the_hook() {
        // The public const is what humans edit. Empty is fine; render must still
        // consult it so adding a line here is all it takes.
        let p = render();
        for item in CUSTOM_ITEMS {
            assert!(
                p.contains(&format!("[ ] {item}")),
                "CUSTOM_ITEMS entry {item:?} missing from render()"
            );
        }
    }

    #[test]
    fn usage_mentions_testplan() {
        // Smoke: the module's public render is what the CLI prints.
        assert!(!render().contains("[x]"));
    }

    fn tutorials_section(plan: &str) -> Option<String> {
        let mut lines = plan.lines();
        lines.find(|l| l.trim() == "## Tutorials")?;
        let mut out = String::new();
        for line in lines {
            if line.starts_with("## ") {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        Some(out)
    }

    /// #1590: every registered walkthrough is a checklist item, so a new entry
    /// in `TUTORIALS` shows up in `bearcad testplan` without a second edit.
    #[test]
    fn every_registered_tutorial_has_an_item() {
        let p = plan();
        let section = tutorials_section(&p)
            .expect("testplan should have a ## Tutorials section generated from TUTORIALS");
        assert!(
            !TUTORIALS.is_empty(),
            "TUTORIALS is the source of the Tutorials section"
        );
        for tut in TUTORIALS {
            assert!(
                section.lines().any(|l| {
                    let l = l.trim();
                    l.starts_with("[ ]") && l.contains(tut.title) && l.contains(tut.name)
                }),
                "missing a [ ] item for tutorial {} ({}):\n{section}",
                tut.name,
                tut.title
            );
        }
    }
}
