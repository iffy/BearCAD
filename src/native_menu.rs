//! Native OS menu bar (File / Edit / View / Help) via [`muda`].
//!
//! Menu items dispatch the same [`Action`] values as the toolbar and scripts.

use crate::actions::Pane;
use crate::menu_command::MenuCommand;
use eframe::CreationContext;
use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
#[cfg(target_os = "macos")]
use muda::AboutMetadata;
#[cfg(target_os = "windows")]
use raw_window_handle::HasWindowHandle;
use std::sync::{Mutex, OnceLock};

/// Stable menu-item ids for mapping [`MenuEvent`]s to [`MenuCommand`]s.
#[derive(Clone, Debug)]
pub struct MenuIds {
    pub new_document: MenuId,
    pub open: MenuId,
    pub save: MenuId,
    pub save_as: MenuId,
    pub export_stl: MenuId,
    pub export_step: MenuId,
    pub import_stl: MenuId,
    pub load_script: MenuId,
    pub import_image: MenuId,
    pub import_step: MenuId,
    pub document_json: MenuId,
    pub export_session_commands: MenuId,
    pub quit: MenuId,
    pub undo: MenuId,
    pub clear: MenuId,
    pub new_drawing: MenuId,
    pub about: MenuId,
    pub licenses: MenuId,
    pub install_cli: MenuId,
    pub command_palette: MenuId,
    pub fps_mode: MenuId,
    pub zoom_to_fit: MenuId,
    pub shortcuts_view: MenuId,
    pub shortcuts_help: MenuId,
    pub pane_checks: Vec<(Pane, MenuId)>,
}

/// Native menu bar and handles for syncing pane checkboxes.
pub struct NativeMenu {
    #[allow(dead_code)]
    menu: Menu,
    ids: MenuIds,
    fps_mode: CheckMenuItem,
    pane_checks: Vec<(Pane, CheckMenuItem)>,
}

static PENDING_MENU_EVENTS: Mutex<Vec<MenuEvent>> = Mutex::new(Vec::new());
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

fn primary_modifier() -> Modifiers {
    #[cfg(target_os = "macos")]
    {
        Modifiers::SUPER
    }
    #[cfg(not(target_os = "macos"))]
    {
        Modifiers::CONTROL
    }
}

/// Map a menu item id to a [`MenuCommand`], if it belongs to this app menu.
pub fn command_for_id(
    id: &MenuId,
    ids: &MenuIds,
    pane_visible: impl Fn(Pane) -> bool,
) -> Option<MenuCommand> {
    if ids.new_document == id {
        return Some(MenuCommand::NewDocument);
    }
    if ids.open == id {
        return Some(MenuCommand::Open);
    }
    if ids.save == id {
        return Some(MenuCommand::Save);
    }
    if ids.save_as == id {
        return Some(MenuCommand::SaveAs);
    }
    if ids.export_stl == id {
        return Some(MenuCommand::ExportStl);
    }
    if ids.export_step == id {
        return Some(MenuCommand::ExportStep);
    }
    if ids.import_image == id {
        return Some(MenuCommand::ImportImage);
    }
    if ids.import_stl == id {
        return Some(MenuCommand::ImportStl);
    }
    if ids.load_script == id {
        return Some(MenuCommand::LoadScript);
    }
    if ids.import_step == id {
        return Some(MenuCommand::ImportStep);
    }
    if ids.document_json == id {
        return Some(MenuCommand::DocumentJson);
    }
    if ids.export_session_commands == id {
        return Some(MenuCommand::ExportSessionCommands);
    }
    if ids.quit == id {
        return Some(MenuCommand::Quit);
    }
    if ids.undo == id {
        return Some(MenuCommand::UndoLast);
    }
    if ids.clear == id {
        return Some(MenuCommand::Clear);
    }
    if id == &ids.new_drawing {
        return Some(MenuCommand::NewDrawing);
    }
    if ids.about == id {
        return Some(MenuCommand::About);
    }
    if ids.shortcuts_view == id || ids.shortcuts_help == id {
        return Some(MenuCommand::ShowShortcuts);
    }
    if ids.licenses == id {
        return Some(MenuCommand::Licenses);
    }
    if ids.install_cli == id {
        return Some(MenuCommand::InstallCli);
    }
    if ids.command_palette == id {
        return Some(MenuCommand::ToggleCommandPalette);
    }
    if ids.fps_mode == id {
        return Some(MenuCommand::ToggleFpsMode);
    }
    if ids.zoom_to_fit == id {
        return Some(MenuCommand::ZoomToFit);
    }
    for &(pane, ref check_id) in &ids.pane_checks {
        if check_id == id {
            return Some(MenuCommand::SetPaneVisible {
                pane,
                visible: pane_visible(pane),
            });
        }
    }
    None
}

/// Map a menu event to a [`MenuCommand`], if it belongs to this app menu.
pub fn command_for_event(event: &MenuEvent, menu: &NativeMenu) -> Option<MenuCommand> {
    command_for_id(
        event.id(),
        &menu.ids,
        |pane| {
            menu.pane_checks
                .iter()
                .find(|(p, _)| *p == pane)
                .map(|(_, item)| item.is_checked())
                .unwrap_or(true)
        },
    )
}

impl NativeMenu {
    /// Build and attach the native menu bar to the running application.
    pub fn install(cc: &CreationContext<'_>) -> Result<Self, muda::Error> {
        let _ = EGUI_CTX.set(cc.egui_ctx.clone());
        install_event_handler();

        let menu = Menu::new();
        let primary = primary_modifier();

        #[cfg(target_os = "macos")]
        {
            let app_menu = Submenu::new("BearCAD", true);
            app_menu.append_items(&[
                &PredefinedMenuItem::about(
                    Some("About BearCAD"),
                    Some(AboutMetadata {
                        name: Some("BearCAD".to_string()),
                        version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        copyright: Some("On-device parametric CAD (prototype)".to_string()),
                        ..Default::default()
                    }),
                ),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ])?;
            menu.append(&app_menu)?;
        }

        let file_menu = Submenu::new("File", true);
        let edit_menu = Submenu::new("Edit", true);
        let cad_menu = Submenu::new("CAD", true);
        let view_menu = Submenu::new("View", true);
        let panes_menu = Submenu::new("Panes", true);
        let help_menu = Submenu::new("Help", true);

        let new_document = MenuItem::with_id(
            "new_document",
            "New",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyN)),
        );
        let open = MenuItem::with_id(
            "open",
            "Open…",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyO)),
        );
        let save = MenuItem::with_id(
            "save",
            "Save",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyS)),
        );
        let save_as = MenuItem::with_id(
            "save_as",
            "Save As…",
            true,
            Some(Accelerator::new(
                Some(primary | Modifiers::SHIFT),
                Code::KeyS,
            )),
        );
        // Import/Export items live under grouped submenus (#352); their IDs are unchanged so the
        // command dispatch still matches, only the visible labels drop the redundant verb.
        let export_stl = MenuItem::with_id("export_stl", "STL…", true, None);
        let export_step = MenuItem::with_id("export_step", "STEP…", true, None);
        let load_script = MenuItem::with_id("load_script", "Load Script…", true, None);
        let import_stl = MenuItem::with_id("import_stl", "STL…", true, None);
        let import_image = MenuItem::with_id("import_image", "Image…", true, None);
        let import_step = MenuItem::with_id("import_step", "STEP…", true, None);
        let document_json = MenuItem::with_id("document_json", "Document JSON…", true, None);
        let quit = MenuItem::with_id(
            "quit",
            "Quit",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyQ)),
        );
        let undo = MenuItem::with_id(
            "undo",
            "Undo",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyZ)),
        );
        let clear = MenuItem::with_id("clear", "Clear", true, None);
        let new_drawing = MenuItem::with_id("new_drawing", "New Drawing", true, None);
        let command_palette = MenuItem::with_id(
            "command_palette",
            "Command Palette…",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyP)),
        );
        let fps_mode = CheckMenuItem::with_id("fps_mode", "FPS Mode", true, false, None);
        let zoom_to_fit = MenuItem::with_id("zoom_to_fit", "Zoom to Fit", true, None);
        let about = MenuItem::with_id("about", "About BearCAD", true, None);
        let shortcuts_view =
            MenuItem::with_id("shortcuts_view", "Keyboard Shortcuts", true, None);
        let shortcuts_help =
            MenuItem::with_id("shortcuts_help", "Keyboard Shortcuts", true, None);
        let licenses = MenuItem::with_id("licenses", "Licenses", true, None);
        let install_cli = MenuItem::with_id(
            "install_cli",
            "Install \"bearcad\" Command in PATH",
            true,
            None,
        );
        let export_session_commands =
            MenuItem::with_id("export_session_commands", "Export Session Commands…", true, None);

        let mut pane_checks = Vec::new();
        let mut pane_ids = Vec::new();
        for &pane in Pane::ALL {
            let check = CheckMenuItem::with_id(
                pane.script_name(),
                pane.label(),
                true,
                true,
                None,
            );
            pane_ids.push((pane, check.id().clone()));
            pane_checks.push((pane, check));
        }

        let file_sep = PredefinedMenuItem::separator();
        file_menu.append(&new_document)?;
        file_menu.append(&open)?;
        file_menu.append(&file_sep)?;
        file_menu.append(&save)?;
        file_menu.append(&save_as)?;
        file_menu.append(&PredefinedMenuItem::separator())?;
        let import_menu = Submenu::new("Import", true);
        import_menu.append(&import_stl)?;
        import_menu.append(&import_step)?;
        import_menu.append(&import_image)?;
        let export_menu = Submenu::new("Export", true);
        export_menu.append(&export_stl)?;
        export_menu.append(&export_step)?;
        file_menu.append(&import_menu)?;
        file_menu.append(&export_menu)?;
        file_menu.append(&load_script)?;
        file_menu.append(&document_json)?;
        #[cfg(not(target_os = "macos"))]
        {
            let quit_sep = PredefinedMenuItem::separator();
            file_menu.append(&quit_sep)?;
            file_menu.append(&quit)?;
        }

        edit_menu.append(&undo)?;
        edit_menu.append(&PredefinedMenuItem::separator())?;
        edit_menu.append(&clear)?;

        cad_menu.append(&new_drawing)?;

        let pane_item_refs: Vec<&dyn muda::IsMenuItem> = pane_checks
            .iter()
            .map(|(_, item)| item as &dyn muda::IsMenuItem)
            .collect();
        panes_menu.append_items(&pane_item_refs)?;
        view_menu.append(&command_palette)?;
        view_menu.append(&zoom_to_fit)?;
        view_menu.append(&fps_mode)?;
        view_menu.append(&shortcuts_view)?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append(&panes_menu)?;
        help_menu.append(&shortcuts_help)?;
        help_menu.append(&export_session_commands)?;
        help_menu.append(&install_cli)?;
        help_menu.append(&PredefinedMenuItem::separator())?;
        help_menu.append(&licenses)?;
        help_menu.append(&about)?;

        menu.append_items(&[&file_menu, &edit_menu, &cad_menu, &view_menu, &help_menu])?;

        attach_to_platform(&menu, cc)?;

        #[cfg(target_os = "macos")]
        help_menu.set_as_help_menu_for_nsapp();

        let ids = MenuIds {
            new_document: new_document.id().clone(),
            open: open.id().clone(),
            save: save.id().clone(),
            save_as: save_as.id().clone(),
            export_stl: export_stl.id().clone(),
            export_step: export_step.id().clone(),
            load_script: load_script.id().clone(),
            import_stl: import_stl.id().clone(),
            import_image: import_image.id().clone(),
            import_step: import_step.id().clone(),
            document_json: document_json.id().clone(),
            export_session_commands: export_session_commands.id().clone(),
            quit: quit.id().clone(),
            undo: undo.id().clone(),
            clear: clear.id().clone(),
            new_drawing: new_drawing.id().clone(),
            about: about.id().clone(),
            licenses: licenses.id().clone(),
            install_cli: install_cli.id().clone(),
            command_palette: command_palette.id().clone(),
            fps_mode: fps_mode.id().clone(),
            zoom_to_fit: zoom_to_fit.id().clone(),
            shortcuts_view: shortcuts_view.id().clone(),
            shortcuts_help: shortcuts_help.id().clone(),
            pane_checks: pane_ids,
        };

        Ok(Self {
            menu,
            ids,
            fps_mode,
            pane_checks,
        })
    }

    /// Drain pending native menu events received since the last call.
    pub fn drain_events(&self) -> Vec<MenuEvent> {
        let mut pending = PENDING_MENU_EVENTS.lock().expect("menu event queue");
        std::mem::take(&mut *pending)
    }

    /// Keep pane checkmarks aligned with application state.
    pub fn sync_pane_checks(&self, is_visible: impl Fn(Pane) -> bool) {
        for &(pane, ref check) in &self.pane_checks {
            check.set_checked(is_visible(pane));
        }
    }

    /// Keep the View ▸ FPS Mode checkmark aligned with whether FPS mode is active (#118).
    pub fn sync_fps_mode(&self, active: bool) {
        self.fps_mode.set_checked(active);
    }
}

fn install_event_handler() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        MenuEvent::set_event_handler(Some(|event| {
            if let Ok(mut pending) = PENDING_MENU_EVENTS.lock() {
                pending.push(event);
            }
            if let Some(ctx) = EGUI_CTX.get() {
                ctx.request_repaint();
            }
        }));
    });
}

fn attach_to_platform(menu: &Menu, cc: &CreationContext<'_>) -> Result<(), muda::Error> {
    #[cfg(target_os = "macos")]
    {
        let _ = cc;
        menu.init_for_nsapp();
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::RawWindowHandle;
        let handle = cc
            .window_handle()
            .map_err(|_| muda::Error::NotInitialized)?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => unsafe {
                menu.init_for_hwnd(handle.hwnd.get())
            },
            _ => Err(muda::Error::NotInitialized),
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (menu, cc);
        // Native menu bar is macOS/Windows only; egui toolbar/palette cover Linux.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;

    fn ids_with_pane(pane_id: &str) -> (MenuIds, MenuId) {
        let pane_menu_id = MenuId::new(pane_id);
        let ids = MenuIds {
            new_document: MenuId::new("new_document"),
            open: MenuId::new("open"),
            save: MenuId::new("save"),
            save_as: MenuId::new("save_as"),
            export_stl: MenuId::new("export_stl"),
            export_step: MenuId::new("export_step"),
            load_script: MenuId::new("load_script"),
            import_stl: MenuId::new("import_stl"),
            import_image: MenuId::new("import_image"),
            import_step: MenuId::new("import_step"),
            document_json: MenuId::new("document_json"),
            export_session_commands: MenuId::new("export_session_commands"),
            quit: MenuId::new("quit"),
            undo: MenuId::new("undo"),
            clear: MenuId::new("clear"),
            new_drawing: MenuId::new("new_drawing"),
            about: MenuId::new("about"),
            licenses: MenuId::new("licenses"),
            install_cli: MenuId::new("install_cli"),
            command_palette: MenuId::new("command_palette"),
            fps_mode: MenuId::new("fps_mode"),
            zoom_to_fit: MenuId::new("zoom_to_fit"),
            shortcuts_view: MenuId::new("shortcuts_view"),
            shortcuts_help: MenuId::new("shortcuts_help"),
            pane_checks: vec![(Pane::ViewCube, pane_menu_id.clone())],
        };
        (ids, pane_menu_id)
    }

    #[test]
    fn maps_file_and_edit_commands() {
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.new_document, &ids, |_| true),
            Some(MenuCommand::NewDocument)
        );
        assert_eq!(
            command_for_id(&ids.open, &ids, |_| true),
            Some(MenuCommand::Open)
        );
        assert_eq!(
            command_for_id(&ids.save, &ids, |_| true),
            Some(MenuCommand::Save)
        );
        assert_eq!(
            command_for_id(&ids.save_as, &ids, |_| true),
            Some(MenuCommand::SaveAs)
        );
        assert_eq!(
            command_for_id(&ids.undo, &ids, |_| true),
            Some(MenuCommand::UndoLast)
        );
        assert_eq!(
            command_for_id(&ids.clear, &ids, |_| true),
            Some(MenuCommand::Clear)
        );
        assert_eq!(
            command_for_id(&ids.export_session_commands, &ids, |_| true),
            Some(MenuCommand::ExportSessionCommands)
        );
        assert_eq!(
            command_for_id(&ids.install_cli, &ids, |_| true),
            Some(MenuCommand::InstallCli)
        );
    }

    #[test]
    fn maps_command_palette_menu_item() {
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.command_palette, &ids, |_| true),
            Some(MenuCommand::ToggleCommandPalette)
        );
        assert_eq!(
            MenuCommand::ToggleCommandPalette.to_action(),
            Some(Action::ToggleCommandPalette)
        );
    }

    #[test]
    fn maps_fps_mode_menu_item() {
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.fps_mode, &ids, |_| true),
            Some(MenuCommand::ToggleFpsMode)
        );
        assert_eq!(
            MenuCommand::ToggleFpsMode.to_action(),
            Some(Action::ToggleFpsMode)
        );
    }

    #[test]
    fn maps_pane_checkbox_state() {
        let (ids, pane_id) = ids_with_pane("view_cube");
        assert_eq!(
            command_for_id(&pane_id, &ids, |_| false),
            Some(MenuCommand::SetPaneVisible {
                pane: Pane::ViewCube,
                visible: false,
            })
        );
    }

    #[test]
    fn ignores_unknown_menu_ids() {
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&MenuId::new("unknown"), &ids, |_| true),
            None
        );
    }

    #[test]
    fn direct_actions_skip_dialog_commands() {
        assert_eq!(
            MenuCommand::Open.to_action(),
            None
        );
        assert_eq!(
            MenuCommand::Save.to_action(),
            None
        );
        assert_eq!(
            MenuCommand::About.to_action(),
            None
        );
        assert_eq!(
            MenuCommand::NewDocument.to_action(),
            Some(Action::NewDocument)
        );
        assert_eq!(
            MenuCommand::SetPaneVisible {
                pane: Pane::ViewCube,
                visible: true
            }
            .to_action(),
            Some(Action::SetPaneVisible {
                pane: Pane::ViewCube,
                visible: true
            })
        );
    }
}