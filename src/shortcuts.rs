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
        Tool::Extrude => Some(ShortcutHint::plain("E")),
        // K/F: no conflict with any other tool letter or constraint mnemonic (A/T/I/M/V/H).
        Tool::Chamfer => Some(ShortcutHint::plain("K")),
        Tool::Fillet => Some(ShortcutHint::plain("F")),
        // T also means the Tangent constraint in tangent contexts (#311); the plain-T binding
        // selects the Text tool everywhere else.
        Tool::Text => Some(ShortcutHint::plain("T")),
        // No plain-letter shortcut; toolbar/palette only. (Plane creation isn't
        // common enough to spend a letter on, #462.)
        Tool::ConstructionPlane
        | Tool::Offset
        | Tool::Loft
        | Tool::Project
        | Tool::Revolve
        | Tool::Sweep
        | Tool::Combine
        | Tool::Move
        | Tool::Mirror
        | Tool::Repeat
        | Tool::Slice
        | Tool::DrawingAdd
        | Tool::DrawingAlign => None,
        Tool::Select => None,
    }
}

pub const TOGGLE_CONSTRUCTION: ShortcutHint = ShortcutHint::plain("X");
/// Curve-mode toggle for the line tool (#73): the next point drawn gets bezier handles.
/// A primary-modifier shortcut (#127), not a plain letter — a bare `B` collided with typing
/// into the in-progress line's length field (its expression syntax accepts letters).
pub const TOGGLE_CURVE_MODE: ShortcutHint = ShortcutHint::primary("B");
/// Tangent-constraint toggle for the line tool (#73): keep curve handles mirrored/smooth.
pub const TOGGLE_TANGENT_CONSTRAINT: ShortcutHint = ShortcutHint::plain("T");
pub const FOCUS_ELEMENT_NAME: ShortcutHint = ShortcutHint::plain("N");
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(shortcut_rich_text(hint));
            });
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
    let mut sections = Vec::new();

    sections.push(ShortcutSection {
        title: "Everywhere",
        scope: None,
        entries: vec![
            (format!("{cmd}P"), "Command palette".to_string()),
            (format!("{cmd}Z"), "Undo".to_string()),
            ("Enter".to_string(), "Commit the in-progress shape/value".to_string()),
            (
                "Esc".to_string(),
                "Cancel what's in progress; again returns to Select (in a sketch: exit)"
                    .to_string(),
            ),
            ("Delete / Backspace".to_string(), "Delete the selection".to_string()),
            ("Z".to_string(), "Zoom to fit (the selection, or everything)".to_string()),
            ("N".to_string(), "Rename the selected element".to_string()),
            ("Tab".to_string(), "Next dimension field while drawing".to_string()),
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
    ];
    sections.push(ShortcutSection {
        title: "Tools",
        scope: Some("3D modeling workbench"),
        entries: tools
            .iter()
            .filter_map(|(tool, label)| {
                tool_shortcut(*tool).map(|hint| (format_shortcut(hint), label.to_string()))
            })
            .collect(),
    });

    sections.push(ShortcutSection {
        title: "Sketch mode",
        scope: Some("while a sketch is open"),
        entries: vec![
            ("Y".to_string(), "Project the selected outside edges/body into the sketch".to_string()),
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
            ("Space / Tab".to_string(), "Accept the highlighted autocomplete name".to_string()),
            ("Enter".to_string(), "Accept the name and commit the field".to_string()),
        ],
    });

    sections.push(ShortcutSection {
        title: "First-person mode",
        scope: Some("View → FPS Mode"),
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
        assert_eq!(tool_shortcut(Tool::Select), None);
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
    }
}
