//! Application menu commands, shared by the native OS menu bar ([`crate::native_menu`],
//! muda) and the web build's in-window menu bar ([`crate::web_menu`], egui). One enum, two
//! frontends — both dispatch through `App::handle_menu_command`.

use crate::actions::Action;
use crate::actions::Pane;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewDocument,
    /// Open a blank document in a new tab (File → New Tab / Cmd+T).
    NewTab,
    Open,
    Save,
    SaveAs,
    ExportStl,
    ExportStep,
    /// Export the current document as a deterministic Lua script (#1159).
    ExportLua,
    ImportStl,
    ImportImage,
    ImportStep,
    /// Import another BearCAD document as a unit (#721).
    ImportUnit,
    /// Open the McMaster-Carr catalog window and import a part from it (#1022).
    ImportMcMaster,
    /// Open the Document JSON dialog: the whole document as pasteable JSON text, for
    /// copying into (and loading back out of) bug reports.
    DocumentJson,
    /// DEV only (#1159): export document as Lua, replay into a temp doc, report diffs.
    VerifyLuaExport,
    /// Pick a `.lua` script and run it against the live document (File menu).
    LoadScript,
    Quit,
    UndoLast,
    Clear,
    /// Create a new technical drawing (CAD menu, #210).
    NewDrawing,
    About,
    /// Open the third-party open-source licenses document (Help menu). See #86.
    Licenses,
    /// Install the `bearcad` CLI symlink onto PATH (Help menu). See #49.
    InstallCli,
    ToggleCommandPalette,
    /// Toggle first-person (FPS) mode (#91, #118).
    ToggleFpsMode,
    ZoomToFit,
    /// Open the Keyboard Shortcuts window (View/Help menus, #434).
    ShowShortcuts,
    /// Open the Settings window (#720): app-level preferences (Cmd/Ctrl+comma).
    ShowSettings,
    /// Toggle help mode (#672): every pane control grows a note saying what it wants.
    ToggleHelpMode,
    SetPaneVisible { pane: Pane, visible: bool },
    /// Open the DEV → Report issue window (#627): dev-build-only filing of an issue (with
    /// optional screenshot/document-JSON attachments) into the local todoer db.
    ReportIssue,
}

impl MenuCommand {
    /// Convert to an [`Action`] where the mapping is direct (no file dialogs).
    pub fn to_action(self) -> Option<Action> {
        match self {
            MenuCommand::NewDocument => Some(Action::NewDocument),
            // Handled in the app frame (workspace-level).
            MenuCommand::NewTab => None,
            MenuCommand::Open | MenuCommand::Save | MenuCommand::SaveAs => None,
            // Needs a file-save dialog, handled in the app frame loop.
            MenuCommand::ExportStl
            | MenuCommand::ExportStep
            | MenuCommand::ExportLua
            | MenuCommand::ImportStl
            | MenuCommand::ImportImage
            | MenuCommand::ImportStep
            | MenuCommand::ImportUnit
            | MenuCommand::ImportMcMaster
            | MenuCommand::DocumentJson
            | MenuCommand::LoadScript
            | MenuCommand::VerifyLuaExport => None,
            MenuCommand::Quit => None,
            MenuCommand::UndoLast => Some(Action::UndoLast),
            MenuCommand::Clear => Some(Action::Clear),
            MenuCommand::NewDrawing => Some(Action::CreateDrawing { name: None }),
            MenuCommand::About => None,
            // Opens a URL in the browser, handled in the app frame loop.
            MenuCommand::Licenses => None,
            // Performs filesystem side effects + status reporting in the app frame loop.
            MenuCommand::InstallCli => None,
            MenuCommand::ToggleCommandPalette => Some(Action::ToggleCommandPalette),
            MenuCommand::ToggleFpsMode => Some(Action::ToggleFpsMode),
            MenuCommand::ZoomToFit => Some(Action::ZoomToFit),
            // Toggles UI-only window state; handled in the app frame loop.
            MenuCommand::ShowShortcuts => None,
            MenuCommand::ShowSettings => None,
            MenuCommand::ToggleHelpMode => Some(Action::SetHelpMode(None)),
            MenuCommand::SetPaneVisible { pane, visible } => {
                Some(Action::SetPaneVisible { pane, visible })
            }
            // Opens the report window; handled in the app frame loop (#627).
            MenuCommand::ReportIssue => None,
        }
    }
}
