//! VS Code-style command palette (SPEC §11.2).
//!
//! Lists context-pertinent commands from the shared action layer. Fuzzy search
//! filters the list; Enter runs the selected command.

use crate::actions::{Action, AppState, CommandPaletteState, Pane, Tool};
use crate::camera::StandardView;
use crate::shortcuts;
use eframe::egui::{self, Key, ScrollArea, TextEdit};

/// Stable command id for scripting and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteCommandId {
    NewDocument,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Clear,
    ToolSelect,
    ToolSketch,
    ToolRectangle,
    ToolLine,
    ToolCircle,
    ToolDimension,
    ToolConstraint,
    ToolPlane,
    ToolExtrude,
    ToolChamfer,
    ToolFillet,
    ToolOffset,
    ToolProject,
    ToolLoft,
    ToolRevolve,
    /// The Create Shape tool (#909).
    ToolShape,
    ToolSweep,
    ToolCombine,
    ToolMove,
    ToolMirror,
    ToolRepeat,
    ToolSlice,
    ToolShell,
    ToolText,
    /// Open the Selection Exploder at the cursor (#576) — the palette equivalent of pressing Space.
    OpenExploder,
    ExitSketch,
    CommitRectangle,
    CommitLine,
    CommitCircle,
    CommitPlane,
    CancelOperation,
    ViewFront,
    ViewBack,
    ViewLeft,
    ViewRight,
    ViewTop,
    ViewBottom,
    ViewHome,
    SetHomeView,
    ToggleProjection,
    ShowPaneHierarchy,
    HidePaneHierarchy,
    ShowPaneParameters,
    HidePaneParameters,
    ShowPaneContext,
    HidePaneContext,
    ShowPaneViewCube,
    HidePaneViewCube,
    DeleteSelection,
    ExportLua,
    /// Import a document Lua script (#1160).
    ImportLua,
    DocumentJson,
    ToggleFpsMode,
    ZoomToFit,
    ProjectSelection,
    ShowShortcuts,
    /// Open the Changelog window (#1328).
    ShowChangelog,
    /// Open the Settings window (#720). Native only: the settings it edits (the library
    /// directory) are filesystem paths the web build has no use for.
    ShowSettings,
    /// Import another BearCAD document as a unit (#721). Native only (path-based).
    ImportUnit,
    /// Open the McMaster-Carr catalog window (#1022). Native only: it runs a second process.
    ImportMcMaster,
    ShowHelpMode,
    HideHelpMode,
}

/// What happens when a palette entry is chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteOutcome {
    Action(Action),
    /// Open the Keyboard Shortcuts window (#434).
    ShowShortcuts,
    /// Open the Settings window (#720).
    ShowSettings,
    /// Pick a BearCAD file and import it as a unit (#721).
    ImportUnit,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ExportLua,
    /// Pick a document Lua script and import it (#1160).
    ImportLua,
    DocumentJson,
    /// Open the Selection Exploder at the cursor (#576).
    OpenExploder,
}

/// One invokable palette entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteCommand {
    pub id: PaletteCommandId,
    pub label: &'static str,
    pub search_text: &'static str,
    /// What this command wants typed after it is chosen (#1022), as the prompt's hint text.
    /// `None` — the usual case — runs on Enter as commands always have; `Some` turns the
    /// palette into a prompt for that argument, **in the pane**, and runs on the next Enter.
    pub argument: Option<&'static str>,
}

impl PaletteCommand {
    const fn new(id: PaletteCommandId, label: &'static str, search_text: &'static str) -> Self {
        Self {
            id,
            label,
            search_text,
            argument: None,
        }
    }

    /// A command that asks for something before it runs, `hint` being what to type.
    const fn with_argument(
        id: PaletteCommandId,
        label: &'static str,
        search_text: &'static str,
        hint: &'static str,
    ) -> Self {
        Self {
            id,
            label,
            search_text,
            argument: Some(hint),
        }
    }

    /// What running this command does. `argument` is what was typed at the prompt — empty
    /// for every command that doesn't ask for one.
    pub fn outcome(self, argument: &str) -> PaletteOutcome {
        match self.id {
            PaletteCommandId::NewDocument => PaletteOutcome::Action(Action::NewDocument),
            PaletteCommandId::Open => PaletteOutcome::OpenFile,
            PaletteCommandId::Save => PaletteOutcome::SaveFile,
            PaletteCommandId::SaveAs => PaletteOutcome::SaveFileAs,
            PaletteCommandId::Undo => PaletteOutcome::Action(Action::UndoLast),
            PaletteCommandId::Redo => PaletteOutcome::Action(Action::RedoLast),
            PaletteCommandId::Clear => PaletteOutcome::Action(Action::Clear),
            PaletteCommandId::ToolSelect => PaletteOutcome::Action(Action::SetTool(Tool::Select)),
            PaletteCommandId::ToolSketch => PaletteOutcome::Action(Action::SetTool(Tool::Sketch)),
            PaletteCommandId::ToolRectangle => {
                PaletteOutcome::Action(Action::SetTool(Tool::Rectangle))
            }
            PaletteCommandId::ToolLine => PaletteOutcome::Action(Action::SetTool(Tool::Line)),
            PaletteCommandId::ToolCircle => PaletteOutcome::Action(Action::SetTool(Tool::Circle)),
            PaletteCommandId::ToolDimension => {
                PaletteOutcome::Action(Action::SetTool(Tool::Dimension))
            }
            PaletteCommandId::ToolConstraint => {
                PaletteOutcome::Action(Action::SetTool(Tool::Constraint))
            }
            PaletteCommandId::ToolExtrude => PaletteOutcome::Action(Action::SetTool(Tool::Extrude)),
            PaletteCommandId::ToolChamfer => PaletteOutcome::Action(Action::SetTool(Tool::Chamfer)),
            PaletteCommandId::ToolFillet => PaletteOutcome::Action(Action::SetTool(Tool::Fillet)),
            PaletteCommandId::ToolOffset => PaletteOutcome::Action(Action::SetTool(Tool::Offset)),
            PaletteCommandId::ToolProject => PaletteOutcome::Action(Action::SetTool(Tool::Project)),
            PaletteCommandId::ToolLoft => PaletteOutcome::Action(Action::SetTool(Tool::Loft)),
            PaletteCommandId::ToolRevolve => PaletteOutcome::Action(Action::SetTool(Tool::Revolve)),
            PaletteCommandId::ToolShape => PaletteOutcome::Action(Action::SetTool(Tool::Shape)),
            PaletteCommandId::ToolSweep => PaletteOutcome::Action(Action::SetTool(Tool::Sweep)),
            PaletteCommandId::ToolCombine => PaletteOutcome::Action(Action::SetTool(Tool::Combine)),
            PaletteCommandId::ToolMove => PaletteOutcome::Action(Action::SetTool(Tool::Move)),
            PaletteCommandId::ToolMirror => PaletteOutcome::Action(Action::SetTool(Tool::Mirror)),
            PaletteCommandId::ToolRepeat => PaletteOutcome::Action(Action::SetTool(Tool::Repeat)),
            PaletteCommandId::ToolSlice => PaletteOutcome::Action(Action::SetTool(Tool::Slice)),
            PaletteCommandId::ToolShell => PaletteOutcome::Action(Action::SetTool(Tool::Shell)),
            PaletteCommandId::ToolText => PaletteOutcome::Action(Action::SetTool(Tool::Text)),
            PaletteCommandId::OpenExploder => PaletteOutcome::OpenExploder,
            PaletteCommandId::ToolPlane => {
                PaletteOutcome::Action(Action::SetTool(Tool::ConstructionPlane))
            }
            PaletteCommandId::ExitSketch => PaletteOutcome::Action(Action::ExitSketch),
            PaletteCommandId::CommitRectangle => PaletteOutcome::Action(Action::CommitRectangle),
            PaletteCommandId::CommitLine => PaletteOutcome::Action(Action::CommitLine),
            PaletteCommandId::CommitCircle => PaletteOutcome::Action(Action::CommitCircle),
            PaletteCommandId::CommitPlane => {
                PaletteOutcome::Action(Action::CommitConstructionPlane)
            }
            PaletteCommandId::CancelOperation => PaletteOutcome::Action(Action::CancelOperation),
            PaletteCommandId::ViewFront => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Front))
            }
            PaletteCommandId::ViewBack => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Back))
            }
            PaletteCommandId::ViewLeft => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Left))
            }
            PaletteCommandId::ViewRight => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Right))
            }
            PaletteCommandId::ViewTop => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Top))
            }
            PaletteCommandId::ViewBottom => {
                PaletteOutcome::Action(Action::SetStandardView(StandardView::Bottom))
            }
            PaletteCommandId::ViewHome => PaletteOutcome::Action(Action::ViewHome),
            PaletteCommandId::SetHomeView => PaletteOutcome::Action(Action::SetHomeView),
            PaletteCommandId::ToggleProjection => {
                PaletteOutcome::Action(Action::ToggleProjectionMode)
            }
            PaletteCommandId::ToggleFpsMode => PaletteOutcome::Action(Action::ToggleFpsMode),
            PaletteCommandId::ZoomToFit => PaletteOutcome::Action(Action::ZoomToFit),
            PaletteCommandId::ProjectSelection => PaletteOutcome::Action(Action::ProjectSelection),
            PaletteCommandId::ShowShortcuts => PaletteOutcome::ShowShortcuts,
            PaletteCommandId::ShowChangelog => {
                PaletteOutcome::Action(Action::SetChangelogWindow { open: Some(true) })
            }
            PaletteCommandId::ShowSettings => PaletteOutcome::ShowSettings,
            PaletteCommandId::ImportUnit => PaletteOutcome::ImportUnit,
            // The catalog opens with the search already done for whatever was typed (#1022);
            // nothing typed opens their front page.
            PaletteCommandId::ImportMcMaster => {
                PaletteOutcome::Action(Action::SetMcMasterWindow {
                    open: Some(true),
                    part: Some(argument.to_string()),
                })
            }
            PaletteCommandId::ShowPaneHierarchy => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Hierarchy,
                visible: true,
            }),
            PaletteCommandId::HidePaneHierarchy => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Hierarchy,
                visible: false,
            }),
            PaletteCommandId::ShowPaneParameters => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Parameters,
                visible: true,
            }),
            PaletteCommandId::HidePaneParameters => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Parameters,
                visible: false,
            }),
            PaletteCommandId::ShowPaneContext => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Context,
                visible: true,
            }),
            PaletteCommandId::HidePaneContext => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::Context,
                visible: false,
            }),
            PaletteCommandId::ShowHelpMode => {
                PaletteOutcome::Action(Action::SetHelpMode(Some(true)))
            }
            PaletteCommandId::HideHelpMode => {
                PaletteOutcome::Action(Action::SetHelpMode(Some(false)))
            }
            PaletteCommandId::ShowPaneViewCube => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::ViewCube,
                visible: true,
            }),
            PaletteCommandId::HidePaneViewCube => PaletteOutcome::Action(Action::SetPaneVisible {
                pane: Pane::ViewCube,
                visible: false,
            }),
            PaletteCommandId::DeleteSelection => {
                PaletteOutcome::Action(Action::DeleteSelection)
            }
            PaletteCommandId::ExportLua => PaletteOutcome::ExportLua,
            PaletteCommandId::ImportLua => PaletteOutcome::ImportLua,
            PaletteCommandId::DocumentJson => PaletteOutcome::DocumentJson,
        }
    }
}

/// Fuzzy-match `query` as a subsequence of `target`. Higher scores are better.
pub fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    let q: Vec<char> = query.trim().to_ascii_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = target.to_ascii_lowercase().chars().collect();
    let mut score = 0i32;
    let mut qi = 0usize;
    let mut prev_match: Option<usize> = None;
    for (ti, &tc) in t.iter().enumerate() {
        if qi < q.len() && tc == q[qi] {
            score += 1;
            if prev_match == Some(ti.saturating_sub(1)) {
                score += 4;
            }
            if ti == 0 || !t[ti - 1].is_ascii_alphanumeric() {
                score += 8;
            }
            if q[qi].is_ascii_alphanumeric() && (ti == 0 || !t[ti - 1].is_ascii_alphanumeric()) {
                score += 6;
            }
            prev_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q.len() {
        Some(score)
    } else {
        None
    }
}

/// Commands available for the current application state.
pub fn commands_for_state(state: &AppState) -> Vec<PaletteCommand> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<PaletteCommand>, cmd: PaletteCommand| out.push(cmd);

    for &cmd in BASE_COMMANDS {
        push(&mut out, cmd);
    }

    if !state.scene_selection.is_empty() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::DeleteSelection,
                "Delete Selection",
                "delete selection remove backspace del",
            ),
        );
    }
    if state.sketch_session.is_some() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::ExitSketch,
                "Exit Sketch",
                "exit sketch leave edit mode",
            ),
        );
    }
    if state.creating_rect.is_some() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::CommitRectangle,
                "Commit Rectangle",
                "commit rectangle enter finish",
            ),
        );
    }
    if state.creating_line.is_some() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::CommitLine,
                "Commit Line",
                "commit line enter finish",
            ),
        );
    }
    if state.creating_circle.is_some() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::CommitCircle,
                "Commit Circle",
                "commit circle enter finish",
            ),
        );
    }
    if state.creating_plane.is_some() {
        push(
            &mut out,
            PaletteCommand::new(
                PaletteCommandId::CommitPlane,
                "Commit Construction Plane",
                "commit plane construction enter finish",
            ),
        );
    }

    // Help mode (#672): whichever way it isn't, so the palette offers the change.
    push(
        &mut out,
        if state.help_mode {
            PaletteCommand::new(
                PaletteCommandId::HideHelpMode,
                "Turn Off Help Mode",
                "help mode off hide explain notes tooltips context pane",
            )
        } else {
            PaletteCommand::new(
                PaletteCommandId::ShowHelpMode,
                "Turn On Help Mode",
                "help mode on show explain notes tooltips context pane what is this",
            )
        },
    );

    for &(pane, show, hide) in PANE_COMMANDS {
        if state.panes.is_visible(pane) {
            push(&mut out, hide);
        } else {
            push(&mut out, show);
        }
    }

    out
}

const PANE_COMMANDS: &[(Pane, PaletteCommand, PaletteCommand)] = &[
    (
        Pane::Hierarchy,
        PaletteCommand::new(
            PaletteCommandId::ShowPaneHierarchy,
            "Show Elements Pane",
            "show elements pane hierarchy tree dag browser",
        ),
        PaletteCommand::new(
            PaletteCommandId::HidePaneHierarchy,
            "Hide Elements Pane",
            "hide elements pane hierarchy tree dag browser",
        ),
    ),
    (
        Pane::Parameters,
        PaletteCommand::new(
            PaletteCommandId::ShowPaneParameters,
            "Show Parameters Pane",
            "show parameters pane params variables",
        ),
        PaletteCommand::new(
            PaletteCommandId::HidePaneParameters,
            "Hide Parameters Pane",
            "hide parameters pane params variables",
        ),
    ),
    (
        Pane::Context,
        PaletteCommand::new(
            PaletteCommandId::ShowPaneContext,
            "Show Context Pane",
            "show context pane properties selection",
        ),
        PaletteCommand::new(
            PaletteCommandId::HidePaneContext,
            "Hide Context Pane",
            "hide context pane properties selection",
        ),
    ),
    (
        Pane::ViewCube,
        PaletteCommand::new(
            PaletteCommandId::ShowPaneViewCube,
            "Show View Bear Pane",
            "show view bear orientation cube pane view hud",
        ),
        PaletteCommand::new(
            PaletteCommandId::HidePaneViewCube,
            "Hide View Bear Pane",
            "hide view bear orientation cube pane view hud",
        ),
    ),
];

const BASE_COMMANDS: &[PaletteCommand] = &[
    PaletteCommand::new(
        PaletteCommandId::NewDocument,
        "New Document",
        "new document file create",
    ),
    PaletteCommand::new(PaletteCommandId::Open, "Open…", "open file document load"),
    PaletteCommand::new(PaletteCommandId::Save, "Save", "save file document write"),
    PaletteCommand::new(
        PaletteCommandId::SaveAs,
        "Save As…",
        "save as file document export",
    ),
    PaletteCommand::new(PaletteCommandId::Undo, "Undo", "undo revert last"),
    PaletteCommand::new(PaletteCommandId::Redo, "Redo", "redo repeat reapply"),
    PaletteCommand::new(PaletteCommandId::Clear, "Clear Document", "clear document delete all"),
    PaletteCommand::new(
        PaletteCommandId::ExportLua,
        "Export Lua Script…",
        "export lua script document recreate deterministic",
    ),
    PaletteCommand::new(
        PaletteCommandId::ImportLua,
        "Import Lua Script…",
        "import lua script document recreate load",
    ),
    PaletteCommand::new(
        PaletteCommandId::DocumentJson,
        "Document JSON…",
        "document json copy paste export import bug report state",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolSelect,
        "Select Tool",
        "select tool navigation mode",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolSketch,
        "Sketch Tool",
        "sketch tool edit face",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolRectangle,
        "Rectangle Tool",
        "rectangle tool rect draw",
    ),
    PaletteCommand::new(PaletteCommandId::ToolLine, "Line Tool", "line tool draw segment"),
    PaletteCommand::new(
        PaletteCommandId::ToolCircle,
        "Circle Tool",
        "circle tool draw diameter",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolDimension,
        "Dimension Tool",
        "dimension tool distance constraint length",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolConstraint,
        "Constraint Tool",
        "constraint tool parallel perpendicular coincident horizontal vertical",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolPlane,
        "Construction Plane Tool",
        "construction plane tool datum",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolExtrude,
        "Extrude Tool",
        "extrude tool solid push pull height 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolChamfer,
        "Chamfer Tool",
        "chamfer tool bevel edge corner 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolFillet,
        "Fillet Tool",
        "fillet tool round edge corner radius 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolOffset,
        "Offset Tool",
        "offset tool parallel duplicate sketch",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolProject,
        "Projection Tool",
        "projection project tool edges reference sketch onto",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolLoft,
        "Loft Tool",
        "loft tool blend profiles skin 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolRevolve,
        "Revolve Tool",
        "revolve tool lathe axis rotate 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolShape,
        "Shape Tool",
        "shape tool cuboid box cube cylinder sphere ball primitive solid 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolSweep,
        "Sweep Tool",
        "sweep tool profile path 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolCombine,
        "Combine Tool",
        "combine tool boolean union subtract intersect 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolMove,
        "Move Tool",
        "move tool translate transform body 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolMirror,
        "Mirror Tool",
        "mirror tool reflect symmetry plane 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolRepeat,
        "Repeat Tool",
        "repeat tool pattern array linear 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolSlice,
        "Slice Tool",
        "slice tool cut split plane 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolShell,
        "Shell Tool",
        "shell tool hollow wall thickness 3d",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToolText,
        "Text Tool",
        "text tool label letters sketch",
    ),
    PaletteCommand::new(
        PaletteCommandId::OpenExploder,
        "Explode Selection Under Cursor",
        "explode exploder selection crowd overlapping stacked disambiguate pick space loupe",
    ),
    PaletteCommand::new(
        PaletteCommandId::CancelOperation,
        "Cancel Operation",
        "cancel escape abort operation",
    ),
    PaletteCommand::new(PaletteCommandId::ViewFront, "View Front", "view front standard camera"),
    PaletteCommand::new(PaletteCommandId::ViewBack, "View Back", "view back standard camera"),
    PaletteCommand::new(PaletteCommandId::ViewLeft, "View Left", "view left standard camera"),
    PaletteCommand::new(PaletteCommandId::ViewRight, "View Right", "view right standard camera"),
    PaletteCommand::new(PaletteCommandId::ViewTop, "View Top", "view top standard camera"),
    PaletteCommand::new(
        PaletteCommandId::ViewBottom,
        "View Bottom",
        "view bottom standard camera",
    ),
    PaletteCommand::new(PaletteCommandId::ViewHome, "View Home", "view home camera reset"),
    PaletteCommand::new(
        PaletteCommandId::SetHomeView,
        "Set Home View",
        "set home view camera bookmark",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToggleProjection,
        "Toggle Projection Mode",
        "toggle projection orthographic natural perspective camera",
    ),
    PaletteCommand::new(
        PaletteCommandId::ToggleFpsMode,
        "Toggle FPS Mode (experimental)",
        "fps first person walk fly wasd shooter mode camera experimental",
    ),
    PaletteCommand::new(
        PaletteCommandId::ZoomToFit,
        "Zoom to Fit",
        "zoom fit frame selection center view camera",
    ),
    PaletteCommand::new(
        PaletteCommandId::ProjectSelection,
        "Project Selection into Sketch",
        "project edges body sketch reference associative Y",
    ),
    PaletteCommand::new(
        PaletteCommandId::ShowShortcuts,
        "Keyboard Shortcuts",
        "keyboard shortcuts keys hotkeys bindings help",
    ),
    PaletteCommand::new(
        PaletteCommandId::ShowChangelog,
        "Changelog",
        "changelog changes release notes history version whats new",
    ),
    #[cfg(not(target_arch = "wasm32"))]
    PaletteCommand::new(
        PaletteCommandId::ShowSettings,
        "Settings",
        "settings preferences options library directory",
    ),
    #[cfg(not(target_arch = "wasm32"))]
    PaletteCommand::new(
        PaletteCommandId::ImportUnit,
        "Import BearCAD File",
        "import bearcad file unit part assembly library",
    ),
    #[cfg(not(target_arch = "wasm32"))]
    PaletteCommand::with_argument(
        PaletteCommandId::ImportMcMaster,
        "Search McMaster-Carr",
        "import mcmaster carr catalog search part screw fastener bearing hardware step",
        "What are you after? (or a part number)",
    ),
];

/// Filter and rank commands for the current query.
pub fn filter_commands<'a>(
    query: &str,
    commands: &'a [PaletteCommand],
) -> Vec<(&'a PaletteCommand, i32)> {
    let mut matches: Vec<(&PaletteCommand, i32)> = commands
        .iter()
        .filter_map(|cmd| {
            let label_score = fuzzy_score(query, cmd.label)?;
            let text_score = fuzzy_score(query, cmd.search_text).unwrap_or(0);
            Some((cmd, label_score.max(text_score)))
        })
        .collect();
    matches.sort_by(|a, b| {
        b.1
            .cmp(&a.1)
            .then_with(|| a.0.label.cmp(b.0.label))
    });
    matches
}

/// Best matching command for a query, if any.
pub fn best_match(query: &str, commands: &[PaletteCommand]) -> Option<PaletteCommand> {
    filter_commands(query, commands)
        .first()
        .map(|(cmd, _)| **cmd)
}

/// Draw the palette console and return a chosen outcome.
pub fn show_palette(
    ui: &mut egui::Ui,
    state: &mut CommandPaletteState,
    matches: &[(&PaletteCommand, i32)],
) -> Option<PaletteOutcome> {
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let escape = ui.input(|i| i.key_pressed(Key::Escape));

    // A command that asks for an argument (#1022) takes over the pane: the list of commands
    // is behind you now, and what you type is the argument rather than the filter.
    if let Some(pending) = state.pending {
        return show_argument_prompt(ui, state, pending, enter, escape);
    }

    if state.query != state.prior_query {
        state.selected = 0;
        state.prior_query = state.query.clone();
    }

    let up = ui.input(|i| i.key_pressed(Key::ArrowUp));
    let down = ui.input(|i| i.key_pressed(Key::ArrowDown));

    if escape {
        state.close_palette();
        return None;
    }

    if !matches.is_empty() {
        if down {
            state.selected = (state.selected + 1).min(matches.len() - 1);
        }
        if up {
            state.selected = state.selected.saturating_sub(1);
        }
    } else {
        state.selected = 0;
    }

    // When the keyboard moved the selection, scroll the focused row into view so
    // the visible pane follows the selection rather than leaving it offscreen.
    let scroll_to_selected = up || down;

    if state.selected >= matches.len() {
        state.selected = matches.len().saturating_sub(1);
    }

    ui.vertical(|ui| {
        if !matches.is_empty() {
            ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, (cmd, _score)) in matches.iter().enumerate() {
                        let selected = index == state.selected;
                        let response = shortcuts::action_row(
                            ui,
                            selected,
                            cmd.label,
                            shortcuts::palette_command_shortcut(cmd.id),
                        );
                        if selected && scroll_to_selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            state.selected = index;
                        }
                    }
                });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
        } else if !state.query.trim().is_empty() {
            ui.label("No matching commands");
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(">").monospace().strong());
            // A stable id keeps keyboard focus on the input across layout changes. Without it
            // egui derives the id from the widget's position in the tree, so when the results
            // area above swaps between the matches list and the "No matching commands" label
            // the input is treated as a new widget and silently loses focus (#55).
            let response = ui.add(
                TextEdit::singleline(&mut state.query)
                    .id(egui::Id::new("command_palette_query"))
                    .hint_text("Type a command…")
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::monospace(14.0)),
            );
            if state.request_focus {
                response.request_focus();
                state.request_focus = false;
            }
        });
    });

    if enter {
        let Some((cmd, _)) = matches.get(state.selected).copied() else {
            return None;
        };
        // A command that wants an argument doesn't run yet — it asks, and the palette stays
        // open to take the answer.
        if cmd.argument.is_some() {
            state.ask_for_argument(cmd.id);
            return None;
        }
        return Some(cmd.outcome(""));
    }

    None
}

/// The argument prompt (#1022): the chosen command's name above its own input, drawn in the
/// palette pane where the command list was. Enter runs it with what's typed; Escape goes back
/// to the command list rather than closing, so a wrong turn costs one keystroke.
fn show_argument_prompt(
    ui: &mut egui::Ui,
    state: &mut CommandPaletteState,
    pending: PaletteCommandId,
    enter: bool,
    escape: bool,
) -> Option<PaletteOutcome> {
    let Some(command) = BASE_COMMANDS.iter().find(|c| c.id == pending).copied() else {
        state.clear_pending();
        return None;
    };
    if escape {
        // Back to the commands, not out of the palette.
        state.clear_pending();
        state.request_focus = true;
        return None;
    }
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(command.label).strong());
            ui.label(
                egui::RichText::new("Esc to go back")
                    .weak()
                    .size(11.0),
            );
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(">").monospace().strong());
            let response = ui.add(
                TextEdit::singleline(&mut state.argument)
                    .id(egui::Id::new("command_palette_argument"))
                    .hint_text(command.argument.unwrap_or_default())
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::monospace(14.0)),
            );
            if state.request_focus {
                response.request_focus();
                state.request_focus = false;
            }
        });
    });
    if enter {
        let argument = state.argument.clone();
        state.close_palette();
        return Some(command.outcome(&argument));
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::sketch_key_for_slot as skey;
    use super::*;
    use crate::actions::SketchSession;

    /// #1022: the first palette command that takes an argument. Choosing it doesn't run it —
    /// it asks, in the palette pane, and runs on the next Enter with what was typed. Every
    /// other command still runs on the first Enter, which is the property that would break
    /// if the argument path leaked into them.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_command_that_takes_an_argument_asks_before_it_runs() {
        let asks = BASE_COMMANDS
            .iter()
            .find(|c| c.id == PaletteCommandId::ImportMcMaster)
            .expect("the catalog command");
        assert_eq!(asks.argument, Some("What are you after? (or a part number)"));
        // The argument is what reaches the action, verbatim — the URL builder, not the
        // palette, decides whether it reads as a search or a part number.
        assert_eq!(
            asks.outcome("socket head screw"),
            PaletteOutcome::Action(Action::SetMcMasterWindow {
                open: Some(true),
                part: Some("socket head screw".to_string()),
            })
        );
        // Nothing typed still opens the catalog, at their front page.
        assert_eq!(
            asks.outcome(""),
            PaletteOutcome::Action(Action::SetMcMasterWindow {
                open: Some(true),
                part: Some(String::new()),
            })
        );
        // Everything else runs straight away, and ignores an argument it never asked for.
        let plain = BASE_COMMANDS
            .iter()
            .find(|c| c.id == PaletteCommandId::ZoomToFit)
            .expect("a command that takes nothing");
        assert_eq!(plain.argument, None);
        assert_eq!(plain.outcome("ignored"), plain.outcome(""));
    }

    /// #1022: the prompt's state machine — asking sets the pending command and clears any
    /// previous answer; backing out returns to the command list with the palette still open;
    /// closing forgets everything.
    #[test]
    fn the_argument_prompt_opens_and_backs_out() {
        let mut state = CommandPaletteState::default();
        state.open_palette();
        assert!(state.pending.is_none(), "a fresh palette asks for nothing");

        state.argument = "stale".to_string();
        state.ask_for_argument(PaletteCommandId::ZoomToFit);
        assert_eq!(state.pending, Some(PaletteCommandId::ZoomToFit));
        assert!(state.argument.is_empty(), "a new prompt starts empty");
        assert!(state.request_focus, "and takes the keyboard");

        // Escape backs out to the commands rather than out of the palette.
        state.argument = "half typed".to_string();
        state.clear_pending();
        assert!(state.pending.is_none() && state.argument.is_empty());
        assert!(state.open, "backing out leaves the palette open");

        // Closing forgets the prompt, so re-opening never resumes a stale one.
        state.ask_for_argument(PaletteCommandId::ZoomToFit);
        state.close_palette();
        assert!(!state.open && state.pending.is_none() && state.argument.is_empty());
        state.open_palette();
        assert!(state.pending.is_none(), "re-opening starts at the command list");
    }

    #[test]
    fn fuzzy_score_matches_subsequence() {
        assert!(fuzzy_score("nd", "New Document").is_some());
        assert!(fuzzy_score("rect", "Rectangle Tool").is_some());
        assert!(fuzzy_score("v fr", "View Front").is_some());
    }

    #[test]
    fn fuzzy_score_rejects_non_matches() {
        assert!(fuzzy_score("xyz", "New Document").is_none());
    }

    #[test]
    fn fuzzy_score_empty_query_matches_all() {
        assert_eq!(fuzzy_score("", "Anything"), Some(0));
    }

    #[test]
    fn filter_commands_ranks_better_matches_first() {
        let cmds = commands_for_state(&AppState::default());
        let filtered = filter_commands("new", &cmds);
        assert!(!filtered.is_empty());
        assert_eq!(filtered[0].0.id, PaletteCommandId::NewDocument);
    }

    #[test]
    fn delete_selection_only_when_something_selected() {
        let mut state = AppState::default();
        assert!(
            !commands_for_state(&state)
                .iter()
                .any(|c| c.id == PaletteCommandId::DeleteSelection)
        );
        state.apply(Action::ClickSceneElement {
            element: crate::hierarchy::SceneElement::Line(lkey(0)),
            additive: false,
        });
        assert!(
            commands_for_state(&state)
                .iter()
                .any(|c| c.id == PaletteCommandId::DeleteSelection)
        );
        assert_eq!(
            PaletteCommand::new(
                PaletteCommandId::DeleteSelection,
                "Delete Selection",
                "delete selection remove backspace del",
            )
            .outcome(""),
            PaletteOutcome::Action(Action::DeleteSelection)
        );
    }

    #[test]
    fn exit_sketch_only_when_editing_sketch() {
        let mut state = AppState::default();
        assert!(
            !commands_for_state(&state)
                .iter()
                .any(|c| c.id == PaletteCommandId::ExitSketch)
        );
        state.sketch_session = Some(SketchSession { sketch: skey(0) });
        assert!(
            commands_for_state(&state)
                .iter()
                .any(|c| c.id == PaletteCommandId::ExitSketch)
        );
    }

    #[test]
    fn pane_commands_reflect_visibility() {
        let mut state = AppState::default();
        state.panes.set(Pane::Parameters, false);
        let cmds = commands_for_state(&state);
        assert!(
            cmds.iter()
                .any(|c| c.id == PaletteCommandId::ShowPaneParameters)
        );
        assert!(
            !cmds.iter()
                .any(|c| c.id == PaletteCommandId::HidePaneParameters)
        );
    }

    #[test]
    fn best_match_finds_tool_by_alias() {
        let cmds = commands_for_state(&AppState::default());
        let cmd = best_match("rect", &cmds).unwrap();
        assert_eq!(cmd.id, PaletteCommandId::ToolRectangle);
    }

    #[test]
    fn palette_shortcuts_include_tools_and_commit() {
        assert_eq!(
            shortcuts::palette_command_shortcut(PaletteCommandId::ToolRectangle),
            Some(shortcuts::ShortcutHint::plain("R"))
        );
        assert_eq!(
            shortcuts::palette_command_shortcut(PaletteCommandId::CommitRectangle),
            Some(shortcuts::ShortcutHint::plain("Enter"))
        );
    }

    #[test]
    fn palette_command_maps_to_action() {
        let cmd = PaletteCommand::new(
            PaletteCommandId::ViewTop,
            "View Top",
            "view top",
        );
        assert_eq!(
            cmd.outcome(""),
            PaletteOutcome::Action(Action::SetStandardView(StandardView::Top))
        );
    }

    #[test]
    fn palette_lists_the_3d_tools_and_the_exploder() {
        // Every modeling tool the palette should reach is present (#576), including the exploder.
        let cmds = commands_for_state(&AppState::default());
        for (query, tool) in [
            ("extrude", Tool::Extrude),
            ("chamfer", Tool::Chamfer),
            ("fillet", Tool::Fillet),
            ("revolve", Tool::Revolve),
            ("sweep", Tool::Sweep),
            ("combine", Tool::Combine),
            ("mirror", Tool::Mirror),
            ("slice", Tool::Slice),
            ("shell", Tool::Shell),
        ] {
            let cmd = best_match(query, &cmds)
                .unwrap_or_else(|| panic!("palette should list a command matching {query:?}"));
            assert_eq!(cmd.outcome(""), PaletteOutcome::Action(Action::SetTool(tool)));
        }
        let exploder = best_match("explode", &cmds).expect("palette lists the exploder");
        assert_eq!(exploder.id, PaletteCommandId::OpenExploder);
        assert_eq!(exploder.outcome(""), PaletteOutcome::OpenExploder);
    }
}