//! Default keyboard shortcut labels for in-app UI (SPEC §11.3).
//!
//! Modifier shortcuts use the platform primary key (⌘ on macOS, Ctrl elsewhere).
//! Viewport tool keys are single-letter and shown on toolbar buttons.

use crate::actions::Tool;
use crate::command_palette::PaletteCommandId;
use eframe::egui::{self, Align, Layout, RichText, Ui};

/// A displayable keyboard shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutHint {
    pub key: &'static str,
    pub modifiers: ShortcutModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutModifiers {
    None,
    Primary,
    PrimaryShift,
}

impl ShortcutHint {
    pub const fn plain(key: &'static str) -> Self {
        Self {
            key,
            modifiers: ShortcutModifiers::None,
        }
    }

    pub const fn primary(key: &'static str) -> Self {
        Self {
            key,
            modifiers: ShortcutModifiers::Primary,
        }
    }

    pub const fn primary_shift(key: &'static str) -> Self {
        Self {
            key,
            modifiers: ShortcutModifiers::PrimaryShift,
        }
    }
}

pub fn primary_modifier_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "⌘"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "Ctrl"
    }
}

pub fn format_shortcut(hint: ShortcutHint) -> String {
    match hint.modifiers {
        ShortcutModifiers::None => hint.key.to_string(),
        ShortcutModifiers::Primary => format!("{}+{}", primary_modifier_label(), hint.key),
        ShortcutModifiers::PrimaryShift => {
            #[cfg(target_os = "macos")]
            {
                format!("{}+⇧+{}", primary_modifier_label(), hint.key)
            }
            #[cfg(not(target_os = "macos"))]
            {
                format!("{}+Shift+{}", primary_modifier_label(), hint.key)
            }
        }
    }
}

pub fn tool_shortcut(tool: Tool) -> Option<ShortcutHint> {
    match tool {
        Tool::Sketch => Some(ShortcutHint::plain("S")),
        Tool::Rectangle => Some(ShortcutHint::plain("R")),
        Tool::Line => Some(ShortcutHint::plain("L")),
        Tool::Circle => Some(ShortcutHint::plain("O")),
        Tool::Dimension => Some(ShortcutHint::plain("D")),
        Tool::Constraint => Some(ShortcutHint::plain("C")),
        // M is free of the constraint mnemonics (those are digits) and of every other tool
        // letter (#665). Repeated M cycles the translate mode.
        Tool::Move => Some(ShortcutHint::plain("M")),
        Tool::Extrude => Some(ShortcutHint::plain("E")),
        // K/F: no conflict with any other tool letter or constraint mnemonic (A/T/I/M/V/H).
        Tool::Chamfer => Some(ShortcutHint::plain("K")),
        Tool::Fillet => Some(ShortcutHint::plain("F")),
        // T also means the Tangent constraint in tangent contexts (#311); the plain-T binding
        // selects the Text tool everywhere else.
        Tool::Text => Some(ShortcutHint::plain("T")),
        // J for the Joint tool (#921): free of every other tool letter and of the
        // constraint mnemonics. Repeated J cycles the joint kind.
        Tool::Joint => Some(ShortcutHint::plain("J")),
        // B for "block" (#909): S — the shape the issue asked for — is the Sketch tool's,
        // and B collides with no other tool letter or constraint mnemonic. Repeated B
        // cycles cuboid → cylinder → sphere.
        Tool::Shape => Some(ShortcutHint::plain("B")),
        // P for Projection inside a sketch (#1193/#1197): select outside geometry, Enter
        // projects; Enter on a selection of only already-projected lines un-projects them.
        Tool::Project => Some(ShortcutHint::plain("P")),
        // No plain-letter shortcut; toolbar/palette only. (Plane creation isn't
        // common enough to spend a letter on, #462.)
        Tool::ConstructionPlane
        | Tool::Offset
        | Tool::Loft
        | Tool::Revolve
        | Tool::Sweep
        | Tool::Combine
        | Tool::Mirror
        | Tool::Repeat
        | Tool::Slice
        | Tool::Shell
        | Tool::DrawingAdd
        | Tool::DrawingAlign => None,
        Tool::Select => None,
    }
}

pub const TOGGLE_CONSTRUCTION: ShortcutHint = ShortcutHint::plain("X");
/// Toggle visibility of the selected objects on the Select tool (#1152).
pub const TOGGLE_VISIBILITY: ShortcutHint = ShortcutHint::plain("V");
/// Curve-mode toggle for the line tool (#73): the next point drawn gets bezier handles.
/// A primary-modifier shortcut (#127), not a plain letter — a bare `B` collided with typing
/// into the in-progress line's length field (its expression syntax accepts letters).
pub const TOGGLE_CURVE_MODE: ShortcutHint = ShortcutHint::primary("B");
/// Tangent-constraint toggle for the line tool (#73): keep curve handles mirrored/smooth.
pub const TOGGLE_TANGENT_CONSTRAINT: ShortcutHint = ShortcutHint::plain("T");
pub const FOCUS_ELEMENT_NAME: ShortcutHint = ShortcutHint::plain("N");
/// Cycle the active tool's Output choice, or Combine's Mode (#1397 / #1534 / #1579 / #1592).
pub const CYCLE_TOOL_OUTPUT_MODE: ShortcutHint = ShortcutHint::plain("Y");
pub const CANCEL_OPERATION: ShortcutHint = ShortcutHint::plain("Esc");
pub const UNDO: ShortcutHint = ShortcutHint::primary("Z");

pub fn palette_command_shortcut(id: PaletteCommandId) -> Option<ShortcutHint> {
    match id {
        PaletteCommandId::NewDocument => Some(ShortcutHint::primary("N")),
        PaletteCommandId::Open => Some(ShortcutHint::primary("O")),
        PaletteCommandId::Save => Some(ShortcutHint::primary("S")),
        PaletteCommandId::SaveAs => Some(ShortcutHint::primary_shift("S")),
        PaletteCommandId::Undo => Some(UNDO),
        PaletteCommandId::ToolSketch => tool_shortcut(Tool::Sketch),
        PaletteCommandId::ToolRectangle => tool_shortcut(Tool::Rectangle),
        PaletteCommandId::ToolLine => tool_shortcut(Tool::Line),
        PaletteCommandId::ToolCircle => tool_shortcut(Tool::Circle),
        PaletteCommandId::ToolPlane => tool_shortcut(Tool::ConstructionPlane),
        PaletteCommandId::ToolDimension => tool_shortcut(Tool::Dimension),
        PaletteCommandId::ToolConstraint => tool_shortcut(Tool::Constraint),
        PaletteCommandId::CancelOperation => Some(CANCEL_OPERATION),
        PaletteCommandId::CommitRectangle
        | PaletteCommandId::CommitLine
        | PaletteCommandId::CommitCircle
        | PaletteCommandId::CommitPlane => Some(ShortcutHint::plain("Enter")),
        _ => None,
    }
}

/// Label with an adjacent parenthetical shortcut, e.g. `Sketch (S)`.
pub fn compact_label(label: &str, shortcut: Option<ShortcutHint>) -> String {
    match shortcut {
        Some(hint) => format!("{label} ({})", format_shortcut(hint)),
        None => label.to_string(),
    }
}

/// Script name for `bearcad.ui.tool(...)` / [`Tool::from_name`].
pub fn tool_script_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "select",
        Tool::Rectangle => "rectangle",
        Tool::Line => "line",
        Tool::Circle => "circle",
        Tool::ConstructionPlane => "construction_plane",
        Tool::Sketch => "sketch",
        Tool::Dimension => "dimension",
        Tool::Project => "project",
        Tool::Constraint => "constraint",
        Tool::Extrude => "extrude",
        Tool::Chamfer => "chamfer",
        Tool::Fillet => "fillet",
        Tool::Offset => "offset",
        Tool::Loft => "loft",
        Tool::Revolve => "revolve",
        Tool::Shape => "shape",
        Tool::Sweep => "sweep",
        Tool::Combine => "combine",
        Tool::Move => "move",
        Tool::Mirror => "mirror",
        Tool::Repeat => "repeat",
        Tool::Slice => "slice",
        Tool::Shell => "shell",
        Tool::Joint => "joint",
        Tool::Text => "text",
        Tool::DrawingAdd => "drawing_add",
        Tool::DrawingAlign => "drawing_align",
    }
}

/// Tools on the current workbench toolbar, left to right. Defined in the tool table
/// so letter keys, the bar, and `EditDrawing` cannot drift (#1506).
pub use crate::tooltable::visible_toolbar_tools;

/// Shortcut labels shown on the toolbar when help mode is on.
///
/// Empty when help mode is off. Keys match [`tool_script_name`] / `bearcad.ui.tool`.
pub fn toolbar_help_shortcuts(
    help_mode: bool,
    drawing: bool,
    in_sketch: bool,
) -> Vec<(&'static str, String)> {
    if !help_mode {
        return Vec::new();
    }
    visible_toolbar_tools(drawing, in_sketch)
        .into_iter()
        .filter_map(|tool| {
            tool_shortcut(tool).map(|hint| (tool_script_name(tool), format_shortcut(hint)))
        })
        .collect()
}

/// Paint a small shortcut badge under each toolbar tool that has one (#1319).
///
/// Badges sit just below the button, joined by a leader, in the same dark-note
/// style as the Context pane's help notes. Only tools whose rect was recorded
/// this frame (they are actually on the bar) get a badge.
pub fn draw_toolbar_help_shortcuts(
    ctx: &egui::Context,
    anchors: &std::collections::HashMap<crate::tutorial::UiAnchor, egui::Rect>,
    drawing: bool,
    in_sketch: bool,
) {
    let mut i = 0usize;
    for tool in visible_toolbar_tools(drawing, in_sketch) {
        let Some(hint) = tool_shortcut(tool) else {
            continue;
        };
        let Some(rect) = anchors.get(&crate::tutorial::UiAnchor::Tool(tool)) else {
            continue;
        };
        draw_toolbar_shortcut_badge(ctx, i, *rect, &format_shortcut(hint));
        i += 1;
    }
}

fn draw_toolbar_shortcut_badge(ctx: &egui::Context, i: usize, tool_rect: egui::Rect, label: &str) {
    let galley = ctx.fonts_mut(|fonts| {
        fonts.layout_no_wrap(
            label.to_string(),
            egui::FontId::monospace(11.0),
            egui::Color32::from_gray(225),
        )
    });
    let pad = egui::vec2(6.0, 3.0);
    let size = galley.size() + pad * 2.0;
    const GAP: f32 = 6.0;
    let badge = egui::Rect::from_center_size(
        egui::pos2(tool_rect.center().x, tool_rect.bottom() + GAP + size.y / 2.0),
        size,
    );
    egui::Area::new(egui::Id::new(("toolbar_help_shortcut", i)))
        .order(egui::Order::Foreground)
        .fixed_pos(badge.min)
        .fade_in(false)
        .interactable(false)
        .show(ctx, |ui| {
            let painter = ui.painter();
            painter.rect_filled(badge, 3.0, egui::Color32::from_black_alpha(230));
            painter.rect_stroke(
                badge,
                3.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
                egui::StrokeKind::Inside,
            );
            painter.galley(badge.min + pad, galley, egui::Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(badge.center().x, badge.top()),
                    egui::pos2(tool_rect.center().x, tool_rect.bottom()),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(110)),
            );
            ui.allocate_space(badge.size());
        });
}

fn shortcut_rich_text(hint: ShortcutHint) -> RichText {
    RichText::new(format_shortcut(hint))
        .weak()
        .monospace()
        .size(11.0)
}

/// Row with primary label on the left and shortcut right-aligned (palette-style).
pub fn action_row(ui: &mut Ui, selected: bool, label: &str, shortcut: Option<ShortcutHint>) -> egui::Response {
    ui.horizontal(|ui| {
        let response = ui.selectable_label(selected, label);
        if let Some(hint) = shortcut {
            // Explicit id: nested RTL thrashing auto-ids across multipass (#1169 / egui#8343).
            ui.scope_builder(
                egui::UiBuilder::new()
                    .layout(Layout::right_to_left(Align::Center))
                    .id(ui.id().with(("action_row_shortcut", label))),
                |ui| {
                    ui.label(shortcut_rich_text(hint));
                },
            );
        }
        response
    })
    .inner
}

/// Fixed number shortcut for a geometric constraint row.
pub fn geometric_constraint_shortcut(
    kind: crate::geometric_constraints::GeometricConstraintType,
) -> ShortcutHint {
    ShortcutHint::plain(kind.shortcut_label())
}

/// Shortcut key shown to the left of a constraint button.
pub fn show_constraint_shortcut_left(ui: &mut Ui, hint: ShortcutHint, enabled: bool) {
    let text = shortcut_rich_text(hint);
    ui.label(if enabled { text } else { text.weak() });
}

/// One section of the app-wide shortcut list (#434), scoped to where its entries apply.
pub struct ShortcutSection {
    pub title: &'static str,
    /// When the section's shortcuts only apply in a certain state, a one-line note.
    pub scope: Option<&'static str>,
    pub entries: Vec<(String, String)>,
}

/// Every keyboard shortcut in the app, grouped by scope (#434) — the single source the
/// Keyboard Shortcuts window renders. **Keep this in sync when adding/changing a
/// binding** (see SPEC §11: tools and constraint mnemonics are derived so they can't go
/// stale; everything else is listed here explicitly).
pub fn all_shortcuts() -> Vec<ShortcutSection> {
    use crate::actions::Tool;
    let cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
    // #1131: previous/next tab — Command+Option on macOS, Ctrl+Alt elsewhere.
    let (prev_tab_keys, next_tab_keys) = if cfg!(target_os = "macos") {
        (format!("{cmd}⌥←"), format!("{cmd}⌥→"))
    } else {
        (format!("{cmd}Alt+Left"), format!("{cmd}Alt+Right"))
    };
    let mut sections = Vec::new();

    sections.push(ShortcutSection {
        title: "Everywhere",
        scope: None,
        entries: vec![
            (format!("{cmd}P"), "Command palette".to_string()),
            (format!("{cmd},"), "Settings (again closes)".to_string()),
            (format!("{cmd}/"), "Help mode (again turns it off)".to_string()),
            (format!("{cmd}Z"), "Undo".to_string()),
            (format!("{cmd}C"), "Copy".to_string()),
            (format!("{cmd}V"), "Paste".to_string()),
            (
                if cfg!(target_os = "macos") {
                    format!("{cmd}⇧V")
                } else {
                    format!("{cmd}Shift+V")
                },
                "Paste Linked (bodies/components)".to_string(),
            ),
            (format!("{cmd}T"), "New tab".to_string()),
            (format!("{cmd}W"), "Close tab".to_string()),
            // #1130: switch to the Nth tab (1-based); no-op when that tab does not exist.
            (format!("{cmd}1–9"), "Switch to tab 1–9".to_string()),
            (prev_tab_keys, "Previous tab".to_string()),
            (next_tab_keys, "Next tab".to_string()),
            // Cycle every OS window, including the McMaster catalog helper (#1023 / #1477).
            (format!("{cmd}`"), "Next window (includes McMaster-Carr catalog when open)".to_string()),
            (
                if cfg!(target_os = "macos") {
                    format!("{cmd}⇧`")
                } else {
                    format!("{cmd}Shift+`")
                },
                "Previous window".to_string(),
            ),
            ("Enter".to_string(), "Commit the in-progress shape/value".to_string()),
            (
                "Esc".to_string(),
                "Cancel what's in progress; again returns to Select (in a sketch: exit)"
                    .to_string(),
            ),
            ("Delete / Backspace".to_string(), "Delete the selection".to_string()),
            ("Z".to_string(), "Zoom to fit (the selection, or everything)".to_string()),
            ("N".to_string(), "Rename the selected element".to_string()),
            (
                "V".to_string(),
                "Toggle visibility of the selection (Select tool)".to_string(),
            ),
            (
                "Tab".to_string(),
                "Next dimension field while drawing (completes a variable name first)".to_string(),
            ),
        ],
    });

    // Tool activation: derived from the same table the toolbar shows, so a new tool
    // shortcut appears here automatically.
    let tools = [
        (Tool::Sketch, "Sketch tool"),
        (Tool::Rectangle, "Rectangle tool"),
        (Tool::Line, "Line tool"),
        (Tool::Circle, "Circle tool"),
        (Tool::ConstructionPlane, "Construction Plane tool"),
        (Tool::Dimension, "Dimension tool"),
        (Tool::Constraint, "Constraint tool"),
        (Tool::Extrude, "Extrude tool"),
        (Tool::Chamfer, "Chamfer tool"),
        (Tool::Fillet, "Fillet tool"),
        (Tool::Text, "Text tool"),
        (Tool::Shape, "Create Shape tool (again cycles cuboid/cylinder/sphere)"),
        (Tool::Joint, "Joint tool (again cycles the joint kind)"),
    ];
    sections.push(ShortcutSection {
        title: "Tools",
        scope: Some("3D modeling workbench (letters do not switch to these while a drawing is open)"),
        entries: {
            let mut entries: Vec<(String, String)> = tools
                .iter()
                .filter_map(|(tool, label)| {
                    tool_shortcut(*tool).map(|hint| (format_shortcut(hint), label.to_string()))
                })
                .collect();
            // #1397: Y cycles the active tool's Output choice (new body / add to body / cut)
            // on Extrude, Revolve, Sweep, Loft, Mirror. #1534: the same key walks Combine's
            // Mode (combine / cut / intersect / difference).
            entries.push((
                format_shortcut(CYCLE_TOOL_OUTPUT_MODE),
                "Cycle the Output choice (new body / add to body / cut), or Combine's mode"
                    .to_string(),
            ));
            entries
        },
    });

    sections.push(ShortcutSection {
        title: "Sketch mode",
        scope: Some("while a sketch is open"),
        entries: vec![
            (
                "P".to_string(),
                "Projection tool — select outside edges/bodies, Enter projects; Enter on projected lines un-projects"
                    .to_string(),
            ),
            ("X".to_string(), "Toggle construction (reference) geometry".to_string()),
            (format!("{cmd}B"), "Toggle curve mode while drawing a line".to_string()),
        ],
    });

    // Constraint mnemonics: derived from the pane's own table, so they can't go stale.
    sections.push(ShortcutSection {
        title: "Constraints",
        scope: Some("Constraint tool active, geometry selected"),
        entries: crate::geometric_constraints::GeometricConstraintType::ALL
            .iter()
            .map(|kind| (kind.shortcut_label().to_string(), kind.label().to_string()))
            .collect(),
    });

    sections.push(ShortcutSection {
        title: "Expression fields",
        scope: Some("while typing in any value input"),
        entries: vec![
            (
                "Space / Tab".to_string(),
                "Accept the highlighted autocomplete name; a second Tab moves to the next input"
                    .to_string(),
            ),
            ("Enter".to_string(), "Accept the name and commit the field".to_string()),
        ],
    });

    sections.push(ShortcutSection {
        title: "First-person mode (experimental)",
        scope: Some("View → FPS Mode (experimental)"),
        entries: vec![
            ("W A S D".to_string(), "Walk".to_string()),
            ("Mouse".to_string(), "Look around".to_string()),
            ("Space".to_string(), "Jump; double-tap to toggle flying".to_string()),
            ("Space / Shift".to_string(), "Ascend / descend while flying".to_string()),
            ("[ / ]".to_string(), "Shrink / grow the player scale".to_string()),
            ("1–9".to_string(), "Pick a tool slot".to_string()),
            ("Wheel".to_string(), "Cycle through the tools".to_string()),
            ("Esc".to_string(), "Leave FPS mode".to_string()),
        ],
    });

    sections.push(ShortcutSection {
        title: "Technical drawings",
        scope: Some("Drawing workbench"),
        entries: vec![
            ("D".to_string(), "Dimension tool".to_string()),
            ("T".to_string(), "Text (page note) tool".to_string()),
            ("Z".to_string(), "Fit the page".to_string()),
            (
                "Numpad 4 5 6 8 2 0".to_string(),
                "View direction on a focused navigation bear (left/front/right/top/bottom/back)"
                    .to_string(),
            ),
            ("Delete / Backspace".to_string(), "Remove the selected page element".to_string()),
        ],
    });

    sections
}


#[cfg(test)]
mod shortcut_list_tests {
    use super::*;
    use crate::actions::Tool;

    /// #434: the shortcut list can't go stale for tools — every tool with a shortcut
    /// appears exactly once in the Tools section.
    #[test]
    fn shortcut_list_covers_every_tool_shortcut() {
        let sections = all_shortcuts();
        let tools = sections.iter().find(|s| s.title == "Tools").expect("Tools section");
        for tool in [
            Tool::Sketch,
            Tool::Rectangle,
            Tool::Line,
            Tool::Circle,
            Tool::Dimension,
            Tool::Constraint,
            Tool::Extrude,
            Tool::Chamfer,
            Tool::Fillet,
            Tool::Text,
        ] {
            let hint = tool_shortcut(tool).expect("tool has a shortcut");
            let key = format_shortcut(hint);
            assert_eq!(
                tools.entries.iter().filter(|(k, _)| *k == key).count(),
                1,
                "tool key {key} listed exactly once"
            );
        }
    }

    /// #434: every constraint mnemonic appears (derived from the pane's own table).
    #[test]
    fn shortcut_list_covers_every_constraint_mnemonic() {
        let sections = all_shortcuts();
        let constraints = sections
            .iter()
            .find(|s| s.title == "Constraints")
            .expect("Constraints section");
        assert_eq!(
            constraints.entries.len(),
            crate::geometric_constraints::GeometricConstraintType::ALL.len()
        );
    }

    /// #1130 / #1131: tab-navigation bindings appear under Everywhere.
    #[test]
    fn shortcut_list_covers_tab_navigation() {
        let sections = all_shortcuts();
        let everywhere = sections
            .iter()
            .find(|s| s.title == "Everywhere")
            .expect("Everywhere section");
        let labels: Vec<&str> = everywhere.entries.iter().map(|(_, d)| d.as_str()).collect();
        assert!(
            labels.iter().any(|d| d.contains("Switch to tab")),
            "Cmd/Ctrl+1–9 tab switch missing: {labels:?}"
        );
        assert!(
            labels.iter().any(|d| d.contains("Previous tab")),
            "previous-tab binding missing: {labels:?}"
        );
        assert!(
            labels.iter().any(|d| d.contains("Next tab")),
            "next-tab binding missing: {labels:?}"
        );
    }

    /// #1477: ⌘` is listed as cycling every window, not as a catalog-only jump.
    #[test]
    fn shortcut_list_covers_window_cycle() {
        let sections = all_shortcuts();
        let everywhere = sections
            .iter()
            .find(|s| s.title == "Everywhere")
            .expect("Everywhere section");
        let labels: Vec<&str> = everywhere.entries.iter().map(|(_, d)| d.as_str()).collect();
        assert!(
            labels.iter().any(|d| d.contains("Next window") && d.contains("McMaster")),
            "window-cycle binding missing: {labels:?}"
        );
        assert!(
            labels.iter().any(|d| d.contains("Previous window")),
            "previous-window binding missing: {labels:?}"
        );
    }

    /// #1397 / #1534: Y is listed as cycling Output and Combine mode.
    #[test]
    fn shortcut_list_covers_y_output_and_combine_mode() {
        let sections = all_shortcuts();
        let tools = sections.iter().find(|s| s.title == "Tools").expect("Tools section");
        let y = tools
            .entries
            .iter()
            .find(|(k, _)| k == "Y")
            .map(|(_, d)| d.as_str())
            .expect("Y binding missing");
        let lower = y.to_lowercase();
        assert!(
            lower.contains("output") && lower.contains("combine"),
            "Y must mention Output and Combine: {y}"
        );
    }

    /// #1152: V toggles selection visibility on the Select tool.
    #[test]
    fn shortcut_list_covers_toggle_visibility() {
        let sections = all_shortcuts();
        let everywhere = sections
            .iter()
            .find(|s| s.title == "Everywhere")
            .expect("Everywhere section");
        assert!(
            everywhere
                .entries
                .iter()
                .any(|(k, d)| k == "V" && d.to_lowercase().contains("visibility")),
            "V visibility shortcut missing: {:?}",
            everywhere.entries
        );
        assert_eq!(format_shortcut(TOGGLE_VISIBILITY), "V");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_plain_shortcut() {
        assert_eq!(format_shortcut(ShortcutHint::plain("R")), "R");
        assert_eq!(format_shortcut(ShortcutHint::plain("Esc")), "Esc");
    }

    #[test]
    fn format_primary_shortcut_uses_platform_modifier() {
        let formatted = format_shortcut(ShortcutHint::primary("Z"));
        assert!(formatted.ends_with("+Z"));
        assert!(formatted.contains(primary_modifier_label()));
    }

    /// #127: curve mode is a primary-modifier shortcut, not a plain `B` — a bare letter
    /// collided with typing into the in-progress line's length field.
    #[test]
    fn curve_mode_shortcut_uses_a_modifier_not_a_bare_letter() {
        assert_eq!(TOGGLE_CURVE_MODE.modifiers, ShortcutModifiers::Primary);
        assert!(format_shortcut(TOGGLE_CURVE_MODE).contains(primary_modifier_label()));
    }

    #[test]
    fn tool_shortcuts_match_viewport_bindings() {
        assert_eq!(
            tool_shortcut(Tool::Rectangle),
            Some(ShortcutHint::plain("R"))
        );
        // #665: the Move tool has a letter now.
        assert_eq!(tool_shortcut(Tool::Move), Some(ShortcutHint::plain("M")));
        // #909/#921: the Shape and Joint tools too, both cycling on a repeat press.
        assert_eq!(tool_shortcut(Tool::Shape), Some(ShortcutHint::plain("B")));
        assert_eq!(tool_shortcut(Tool::Joint), Some(ShortcutHint::plain("J")));
        // #1197: P activates the Projection tool in a sketch (select, then Enter commits).
        assert_eq!(tool_shortcut(Tool::Project), Some(ShortcutHint::plain("P")));
        assert_eq!(tool_shortcut(Tool::Select), None);
    }

    /// No two tools claim the same plain letter — a collision would make one of them
    /// unreachable from the keyboard.
    #[test]
    fn tool_shortcut_letters_are_unique() {
        use std::collections::HashMap;
        let mut seen: HashMap<String, Tool> = HashMap::new();
        for tool in Tool::ALL {
            if let Some(hint) = tool_shortcut(tool) {
                let key = format_shortcut(hint);
                if let Some(other) = seen.insert(key.clone(), tool) {
                    panic!("{key} is claimed by both {other:?} and {tool:?}");
                }
            }
        }
    }

    #[test]
    fn palette_maps_document_shortcuts() {
        assert_eq!(
            palette_command_shortcut(PaletteCommandId::Undo),
            Some(UNDO)
        );
        assert_eq!(
            palette_command_shortcut(PaletteCommandId::CancelOperation),
            Some(CANCEL_OPERATION)
        );
    }

    #[test]
    fn geometric_constraint_shortcut_maps_digits() {
        use crate::geometric_constraints::GeometricConstraintType;
        assert_eq!(
            format_shortcut(geometric_constraint_shortcut(
                GeometricConstraintType::Parallel
            )),
            "1"
        );
        assert_eq!(
            format_shortcut(geometric_constraint_shortcut(
                GeometricConstraintType::Midpoint
            )),
            "5"
        );
    }

    #[test]
    fn compact_label_includes_shortcut() {
        assert_eq!(
            compact_label("Sketch", tool_shortcut(Tool::Sketch)),
            "Sketch (S)"
        );
        assert_eq!(compact_label("Select", None), "Select");
        // #1579: Combine's Mode row uses the same parenthetical form as Name (N).
        assert_eq!(
            compact_label("Mode", Some(CYCLE_TOOL_OUTPUT_MODE)),
            "Mode (Y)"
        );
        // #1592: Output rows (Extrude/Revolve/Sweep/Loft/Mirror) use the same form.
        assert_eq!(
            compact_label("Output", Some(CYCLE_TOOL_OUTPUT_MODE)),
            "Output (Y)"
        );
    }

    /// #1319: help mode off → no toolbar shortcut badges.
    #[test]
    fn toolbar_help_shortcuts_empty_when_help_mode_is_off() {
        assert!(toolbar_help_shortcuts(false, false, false).is_empty());
        assert!(toolbar_help_shortcuts(false, false, true).is_empty());
        assert!(toolbar_help_shortcuts(false, true, false).is_empty());
    }

    /// #1319: Shape's B (and the other toolbar letters) appear when help mode is on.
    #[test]
    fn toolbar_help_shortcuts_include_shape_b() {
        let labels = toolbar_help_shortcuts(true, false, false);
        assert!(
            labels.iter().any(|(n, k)| *n == "shape" && k == "B"),
            "Shape should show B, got {labels:?}"
        );
        for (name, key) in [
            ("sketch", "S"),
            ("rectangle", "R"),
            ("line", "L"),
            ("circle", "O"),
            ("fillet", "F"),
            ("chamfer", "K"),
            ("text", "T"),
            ("extrude", "E"),
            ("move", "M"),
            ("joint", "J"),
            ("dimension", "D"),
            ("constraint", "C"),
        ] {
            assert!(
                labels.iter().any(|(n, k)| *n == name && k == key),
                "{name} should show {key}, got {labels:?}"
            );
        }
    }

    /// #1319: tools without a letter (Select, Offset, …) get no badge.
    #[test]
    fn toolbar_help_shortcuts_omit_tools_without_bindings() {
        let labels = toolbar_help_shortcuts(true, false, false);
        for name in [
            "select",
            "offset",
            "construction_plane",
            "sweep",
            "loft",
            "revolve",
            "combine",
            "mirror",
            "repeat",
            "slice",
            "shell",
        ] {
            assert!(
                !labels.iter().any(|(n, _)| *n == name),
                "{name} has no shortcut, should not appear: {labels:?}"
            );
        }
    }

    /// #1494: Projection sits on the toolbar outside a sketch (it clicks a face, like Offset).
    #[test]
    fn toolbar_help_shortcuts_include_project_outside_a_sketch() {
        let outside = toolbar_help_shortcuts(true, false, false);
        assert!(
            outside.iter().any(|(n, k)| *n == "project" && k == "P"),
            "Project should show P outside a sketch, got {outside:?}"
        );
        let inside = toolbar_help_shortcuts(true, false, true);
        assert!(
            inside.iter().any(|(n, k)| *n == "project" && k == "P"),
            "Project should show P in a sketch, got {inside:?}"
        );
    }

    /// #1494: every face-click tool has a toolbar button outside a sketch.
    #[test]
    fn face_click_tools_sit_on_the_toolbar_outside_a_sketch() {
        let bar = visible_toolbar_tools(false, false);
        for tool in crate::actions::Tool::ALL {
            if crate::tooltable::opens_sketch_on_face_click(tool) {
                assert!(
                    bar.contains(&tool),
                    "{tool:?} clicks a face to start a sketch but is hidden outside one"
                );
            }
        }
    }

    /// #1319: every tool letter has a toolbar button to hang its badge on.
    #[test]
    fn every_tool_shortcut_has_a_toolbar_home() {
        for tool in Tool::ALL {
            if tool_shortcut(tool).is_none() {
                continue;
            }
            let on_bar = visible_toolbar_tools(false, true).contains(&tool)
                || visible_toolbar_tools(true, false).contains(&tool);
            assert!(
                on_bar,
                "{tool:?} has a shortcut but is on no workbench toolbar"
            );
        }
    }

    /// #1319: script names match `bearcad.ui.tool` / `Tool::from_name`.
    #[test]
    fn toolbar_tool_script_names_round_trip() {
        for drawing in [false, true] {
            for in_sketch in [false, true] {
                for tool in visible_toolbar_tools(drawing, in_sketch) {
                    let name = tool_script_name(tool);
                    assert_eq!(
                        Tool::from_name(name),
                        Some(tool),
                        "{name} should parse back to {tool:?}"
                    );
                }
            }
        }
    }

    /// #1319: the drawing workbench only badges Dimension (D) and Text (T).
    #[test]
    fn toolbar_help_shortcuts_on_the_drawing_workbench() {
        let labels = toolbar_help_shortcuts(true, true, false);
        assert!(
            labels.iter().any(|(n, k)| *n == "dimension" && k == "D"),
            "Dimension should show D, got {labels:?}"
        );
        assert!(
            labels.iter().any(|(n, k)| *n == "text" && k == "T"),
            "Text should show T, got {labels:?}"
        );
        assert_eq!(labels.len(), 2, "only Dimension and Text have letters: {labels:?}");
        assert!(
            !labels.iter().any(|(n, _)| *n == "shape" || *n == "sketch"),
            "modeling tools stay off the drawing bar: {labels:?}"
        );
    }
}
