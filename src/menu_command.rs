//! Application menu commands, shared by the native OS menu bar ([`crate::native_menu`],
//! muda) and the web build's in-window menu bar ([`crate::web_menu`], egui). One enum, two
//! frontends — both dispatch through `App::handle_menu_command`.

use crate::actions::Action;
use crate::actions::Pane;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewDocument,
    Open,
    Save,
    SaveAs,
    ExportStl,
    ExportStep,
    ImportStl,
    ImportImage,
    ImportStep,
    ExportSessionCommands,
    /// Open the Document JSON dialog: the whole document as pasteable JSON text, for
    /// copying into (and loading back out of) bug reports.
    DocumentJson,
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
    SetPaneVisible { pane: Pane, visible: bool },
}

impl MenuCommand {
    /// Convert to an [`Action`] where the mapping is direct (no file dialogs).
    pub fn to_action(self) -> Option<Action> {
        match self {
            MenuCommand::NewDocument => Some(Action::NewDocument),
            MenuCommand::Open | MenuCommand::Save | MenuCommand::SaveAs => None,
            // Needs a file-save dialog, handled in the app frame loop.
            MenuCommand::ExportStl
            | MenuCommand::ExportStep
            | MenuCommand::ImportStl
            | MenuCommand::ImportImage
            | MenuCommand::ImportStep
            | MenuCommand::ExportSessionCommands
            | MenuCommand::DocumentJson
            | MenuCommand::LoadScript => None,
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
            MenuCommand::SetPaneVisible { pane, visible } => {
                Some(Action::SetPaneVisible { pane, visible })
            }
        }
    }
}
