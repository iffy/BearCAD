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
    pub new_tab: MenuId,
    pub open: MenuId,
    pub save: MenuId,
    pub save_as: MenuId,
    pub rebuild_geometry: MenuId,
    pub export_stl: MenuId,
    pub export_3mf: MenuId,
    pub export_step: MenuId,
    pub export_lua: MenuId,
    pub import_stl: MenuId,
    /// File → Import → BearCAD File… (#721).
    pub import_unit: MenuId,
    /// File → Import → McMaster-Carr… (#1022).
    pub import_mcmaster: MenuId,
    pub load_script: MenuId,
    pub import_image: MenuId,
    pub import_step: MenuId,
    /// File → Import → Lua Script… (#1160).
    pub import_lua: MenuId,
    pub document_json: MenuId,
    pub quit: MenuId,
    pub undo: MenuId,
    pub clear: MenuId,
    pub copy: MenuId,
    pub paste: MenuId,
    pub paste_linked: MenuId,
    pub new_drawing: MenuId,
    pub about: MenuId,
    pub licenses: MenuId,
    pub install_cli: MenuId,
    pub command_palette: MenuId,
    pub fps_mode: MenuId,
    pub zoom_to_fit: MenuId,
    pub shortcuts_view: MenuId,
    pub shortcuts_help: MenuId,
    /// Help ▸ Changelog (#1328).
    pub changelog: MenuId,
    /// Help ▸ Install AI Agent Skill… (#1604).
    pub install_ai_skill: MenuId,
    /// Integration ▸ AI MCP Server… (#1622): open the AI pane at its MCP Server section.
    pub mcp_server: MenuId,
    /// Help ▸ Report Problem… (#1372): open the browser at a new-issue form on the repo.
    pub report_problem: MenuId,
    /// Settings… (#720): the app menu on macOS, the File menu elsewhere.
    pub settings: MenuId,
    /// Help ▸ Help Mode (#672), a checked toggle with the Cmd/Ctrl+/ accelerator.
    pub help_mode: MenuId,
    /// View ▸ Tool Hints (#1509), a checked toggle for the viewport hint overlay.
    pub tool_hints: MenuId,
    /// DEV → Report issue (#627); the DEV menu only appears in debug builds.
    pub report_issue: MenuId,
    /// DEV → Verify Lua export (#1159); debug builds only.
    pub verify_lua_export: MenuId,
    pub pane_checks: Vec<(Pane, MenuId)>,
}

/// Native menu bar and handles for syncing pane checkboxes.
pub struct NativeMenu {
    #[allow(dead_code)]
    menu: Menu,
    ids: MenuIds,
    fps_mode: CheckMenuItem,
    /// Help ▸ Help Mode's checkbox (#672), synced from app state each frame.
    help_mode: CheckMenuItem,
    /// View ▸ Tool Hints checkbox (#1509), synced from app state each frame.
    tool_hints: CheckMenuItem,
    pane_checks: Vec<(Pane, CheckMenuItem)>,
}

static PENDING_MENU_EVENTS: Mutex<Vec<MenuEvent>> = Mutex::new(Vec::new());
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

/// The built menu bar's shape, recorded at install time (#1622): top-level menu titles,
/// each with the labels of its direct items. `bearcad.ui.menu_structure()` surfaces this
/// to scripts so the OS menu bar — which pointer input can't drive — stays testable.
static MENU_STRUCTURE: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());

/// Snapshot of the built menu bar: top-level title → direct item labels.
pub fn menu_structure() -> Vec<(String, Vec<String>)> {
    MENU_STRUCTURE.lock().expect("menu structure").clone()
}

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
    if ids.new_tab == id {
        return Some(MenuCommand::NewTab);
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
    if ids.rebuild_geometry == id {
        return Some(MenuCommand::RebuildGeometry);
    }
    if ids.export_stl == id {
        return Some(MenuCommand::ExportStl);
    }
    if ids.export_3mf == id {
        return Some(MenuCommand::Export3mf);
    }
    if ids.export_step == id {
        return Some(MenuCommand::ExportStep);
    }
    if ids.export_lua == id {
        return Some(MenuCommand::ExportLua);
    }
    if ids.import_image == id {
        return Some(MenuCommand::ImportImage);
    }
    if ids.import_stl == id {
        return Some(MenuCommand::ImportStl);
    }
    if ids.import_unit == id {
        return Some(MenuCommand::ImportUnit);
    }
    if ids.import_mcmaster == id {
        return Some(MenuCommand::ImportMcMaster);
    }
    if ids.load_script == id {
        return Some(MenuCommand::LoadScript);
    }
    if ids.import_step == id {
        return Some(MenuCommand::ImportStep);
    }
    if ids.import_lua == id {
        return Some(MenuCommand::ImportLua);
    }
    if ids.document_json == id {
        return Some(MenuCommand::DocumentJson);
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
    if ids.copy == id {
        return Some(MenuCommand::Copy);
    }
    if ids.paste == id {
        return Some(MenuCommand::Paste);
    }
    if ids.paste_linked == id {
        return Some(MenuCommand::PasteLinked);
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
    if ids.changelog == id {
        return Some(MenuCommand::ShowChangelog);
    }
    if ids.install_ai_skill == id {
        return Some(MenuCommand::InstallAiSkill);
    }
    if ids.mcp_server == id {
        return Some(MenuCommand::McpServer);
    }
    if ids.report_problem == id {
        return Some(MenuCommand::ReportProblem);
    }
    if ids.settings == id {
        return Some(MenuCommand::ShowSettings);
    }
    if ids.help_mode == id {
        return Some(MenuCommand::ToggleHelpMode);
    }
    if ids.tool_hints == id {
        return Some(MenuCommand::ToggleToolHints);
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
    if ids.report_issue == id {
        return Some(MenuCommand::ReportIssue);
    }
    if ids.verify_lua_export == id {
        return Some(MenuCommand::VerifyLuaExport);
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

        // Settings… (#720): the app menu on macOS (the conventional spot), the File menu
        // elsewhere. No muda accelerator — Cmd/Ctrl+comma is handled in the egui key
        // layer, one code path for every platform (and it toggles, which a menu can't).
        let settings_item = MenuItem::with_id("settings", "Settings…", true, None);

        #[cfg(target_os = "macos")]
        {
            let app_menu = Submenu::new("BearCAD", true);
            app_menu.append_items(&[
                &PredefinedMenuItem::about(
                    Some("About BearCAD"),
                    Some(AboutMetadata {
                        name: Some("BearCAD".to_string()),
                        version: Some(crate::full_version()),
                        copyright: Some("On-device parametric CAD (prototype)".to_string()),
                        // The BearCAD icon, so the About panel shows it instead of the
                        // generic (folder-like) placeholder macOS uses otherwise (#529).
                        icon: crate::app_icon::about_icon_rgba(256)
                            .and_then(|(rgba, w, h)| muda::Icon::from_rgba(rgba, w, h).ok()),
                        ..Default::default()
                    }),
                ),
                &PredefinedMenuItem::separator(),
                &settings_item,
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
        // #1622: the developer-facing integration items (CLI symlink, AI agent skill, AI
        // MCP server) group into their own top-level menu instead of sprawling through Help.
        let integration_menu = Submenu::new("Integration", true);

        let new_document = MenuItem::with_id(
            "new_document",
            "New",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyN)),
        );
        // Accelerator is handled in the egui key layer so one path works with/without muda.
        let new_tab = MenuItem::with_id("new_tab", "New Tab", true, None);
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
        let rebuild_geometry = MenuItem::with_id("rebuild_geometry", "Rebuild Geometry", true, None);
        // Import/Export items live under grouped submenus (#352); their IDs are unchanged so the
        // command dispatch still matches, only the visible labels drop the redundant verb.
        let export_stl = MenuItem::with_id("export_stl", "STL…", true, None);
        let export_3mf = MenuItem::with_id("export_3mf", "3MF…", true, None);
        let export_step = MenuItem::with_id("export_step", "STEP…", true, None);
        let export_lua = MenuItem::with_id("export_lua", "Lua Script…", true, None);
        let load_script = MenuItem::with_id("load_script", "Load Script…", true, None);
        let import_unit = MenuItem::with_id("import_unit", "BearCAD File…", true, None);
        let import_mcmaster =
            MenuItem::with_id("import_mcmaster", "McMaster-Carr…", true, None);
        let import_stl = MenuItem::with_id("import_stl", "STL…", true, None);
        let import_image = MenuItem::with_id("import_image", "Image…", true, None);
        let import_step = MenuItem::with_id("import_step", "STEP…", true, None);
        let import_lua = MenuItem::with_id("import_lua", "Lua Script…", true, None);
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
        // Accelerators also handled in the egui key layer (one path with/without muda).
        let copy = MenuItem::with_id("copy", "Copy", true, None);
        let paste = MenuItem::with_id("paste", "Paste", true, None);
        let paste_linked = MenuItem::with_id("paste_linked", "Paste Linked", true, None);
        let clear = MenuItem::with_id("clear", "Clear", true, None);
        let new_drawing = MenuItem::with_id("new_drawing", "New Drawing", true, None);
        let command_palette = MenuItem::with_id(
            "command_palette",
            "Command Palette…",
            true,
            Some(Accelerator::new(Some(primary), Code::KeyP)),
        );
        let fps_mode =
            CheckMenuItem::with_id("fps_mode", "FPS Mode (experimental)", true, false, None);
        let tool_hints =
            CheckMenuItem::with_id("tool_hints", "Tool Hints", true, true, None);
        let zoom_to_fit = MenuItem::with_id("zoom_to_fit", "Zoom to Fit", true, None);
        let about = MenuItem::with_id("about", "About BearCAD", true, None);
        let shortcuts_view =
            MenuItem::with_id("shortcuts_view", "Keyboard Shortcuts", true, None);
        let shortcuts_help =
            MenuItem::with_id("shortcuts_help", "Keyboard Shortcuts", true, None);
        let changelog = MenuItem::with_id("changelog", "Changelog", true, None);
        // Report Problem… (#1372): open the browser at a new-issue form on the repo.
        let report_problem = MenuItem::with_id("report_problem", "Report Problem…", true, None);
        // Help mode (#672): the pane-note overlay, toggled from the Help menu or
        // Cmd/Ctrl+/ (a slash — the question mark without reaching for Shift).
        let help_mode = CheckMenuItem::with_id(
            "help_mode",
            "Help Mode",
            true,
            false,
            Some(Accelerator::new(Some(primary), Code::Slash)),
        );
        let licenses = MenuItem::with_id("licenses", "Licenses", true, None);
        let install_cli = MenuItem::with_id(
            "install_cli",
            "Install \"bearcad\" Command in PATH",
            true,
            None,
        );
        // #1604: the same idea for AI tools — opens the AI pane at Agents & Skill.
        let install_ai_skill = MenuItem::with_id(
            "install_ai_skill",
            "Install AI Agent Skill…",
            true,
            None,
        );
        // #1622: open the AI pane at its MCP Server section.
        let mcp_server = MenuItem::with_id("mcp_server", "AI MCP Server…", true, None);
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
        file_menu.append(&new_tab)?;
        file_menu.append(&open)?;
        file_menu.append(&file_sep)?;
        file_menu.append(&save)?;
        file_menu.append(&save_as)?;
        file_menu.append(&rebuild_geometry)?;
        file_menu.append(&PredefinedMenuItem::separator())?;
        let import_menu = Submenu::new("Import", true);
        import_menu.append(&import_unit)?;
        import_menu.append(&import_mcmaster)?;
        import_menu.append(&import_stl)?;
        import_menu.append(&import_step)?;
        import_menu.append(&import_image)?;
        import_menu.append(&import_lua)?;
        let export_menu = Submenu::new("Export", true);
        export_menu.append(&export_stl)?;
        export_menu.append(&export_3mf)?;
        export_menu.append(&export_step)?;
        export_menu.append(&export_lua)?;
        file_menu.append(&import_menu)?;
        file_menu.append(&export_menu)?;
        file_menu.append(&load_script)?;
        file_menu.append(&document_json)?;
        #[cfg(not(target_os = "macos"))]
        {
            file_menu.append(&PredefinedMenuItem::separator())?;
            file_menu.append(&settings_item)?;
            let quit_sep = PredefinedMenuItem::separator();
            file_menu.append(&quit_sep)?;
            file_menu.append(&quit)?;
        }

        edit_menu.append(&undo)?;
        edit_menu.append(&PredefinedMenuItem::separator())?;
        edit_menu.append(&copy)?;
        edit_menu.append(&paste)?;
        edit_menu.append(&paste_linked)?;
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
        view_menu.append(&tool_hints)?;
        view_menu.append(&shortcuts_view)?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append(&panes_menu)?;
        help_menu.append(&help_mode)?;
        help_menu.append(&shortcuts_help)?;
        help_menu.append(&changelog)?;
        help_menu.append(&report_problem)?;
        integration_menu.append(&install_cli)?;
        integration_menu.append(&install_ai_skill)?;
        integration_menu.append(&mcp_server)?;
        help_menu.append(&PredefinedMenuItem::separator())?;
        help_menu.append(&licenses)?;
        help_menu.append(&about)?;

        menu.append_items(&[
            &file_menu,
            &edit_menu,
            &cad_menu,
            &view_menu,
            &integration_menu,
            &help_menu,
        ])?;

        // DEV menu (#627/#1159), debug builds (`cargo run`) only: developer utilities.
        let report_issue = MenuItem::with_id("report_issue", "Report issue…", true, None);
        let verify_lua_export =
            MenuItem::with_id("verify_lua_export", "Verify Lua export…", true, None);
        if cfg!(debug_assertions) {
            let dev_menu = Submenu::new("DEV", true);
            dev_menu.append(&report_issue)?;
            dev_menu.append(&verify_lua_export)?;
            menu.append(&dev_menu)?;
        }

        attach_to_platform(&menu, cc)?;

        // Publish the bar's shape for `bearcad.ui.menu_structure()` (#1622).
        *MENU_STRUCTURE.lock().expect("menu structure") = summarize_menu(&menu);

        #[cfg(target_os = "macos")]
        help_menu.set_as_help_menu_for_nsapp();

        let ids = MenuIds {
            new_document: new_document.id().clone(),
            new_tab: new_tab.id().clone(),
            open: open.id().clone(),
            save: save.id().clone(),
            save_as: save_as.id().clone(),
            rebuild_geometry: rebuild_geometry.id().clone(),
            export_stl: export_stl.id().clone(),
            export_3mf: export_3mf.id().clone(),
            export_step: export_step.id().clone(),
            export_lua: export_lua.id().clone(),
            load_script: load_script.id().clone(),
            import_stl: import_stl.id().clone(),
            import_unit: import_unit.id().clone(),
            import_mcmaster: import_mcmaster.id().clone(),
            import_image: import_image.id().clone(),
            import_step: import_step.id().clone(),
            import_lua: import_lua.id().clone(),
            document_json: document_json.id().clone(),
            quit: quit.id().clone(),
            undo: undo.id().clone(),
            clear: clear.id().clone(),
            copy: copy.id().clone(),
            paste: paste.id().clone(),
            paste_linked: paste_linked.id().clone(),
            new_drawing: new_drawing.id().clone(),
            about: about.id().clone(),
            licenses: licenses.id().clone(),
            install_cli: install_cli.id().clone(),
            install_ai_skill: install_ai_skill.id().clone(),
            mcp_server: mcp_server.id().clone(),
            command_palette: command_palette.id().clone(),
            fps_mode: fps_mode.id().clone(),
            zoom_to_fit: zoom_to_fit.id().clone(),
            shortcuts_view: shortcuts_view.id().clone(),
            shortcuts_help: shortcuts_help.id().clone(),
            changelog: changelog.id().clone(),
            report_problem: report_problem.id().clone(),
            settings: settings_item.id().clone(),
            help_mode: help_mode.id().clone(),
            tool_hints: tool_hints.id().clone(),
            report_issue: report_issue.id().clone(),
            verify_lua_export: verify_lua_export.id().clone(),
            pane_checks: pane_ids,
        };

        Ok(Self {
            menu,
            ids,
            fps_mode,
            help_mode,
            tool_hints,
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

    /// Keep the View ▸ FPS Mode (experimental) checkmark aligned with FPS mode (#118).
    pub fn sync_fps_mode(&self, active: bool) {
        self.fps_mode.set_checked(active);
    }

    /// Keep the Help ▸ Help Mode checkmark aligned with the app state (#672).
    pub fn sync_help_mode(&self, active: bool) {
        self.help_mode.set_checked(active);
    }

    /// Keep the View ▸ Tool Hints checkmark aligned with the app state (#1509).
    pub fn sync_tool_hints(&self, active: bool) {
        self.tool_hints.set_checked(active);
    }
}

/// The labels a menu's items show, in order; submenu entries contribute their own titles.
fn summarize_items(items: Vec<muda::MenuItemKind>) -> Vec<String> {
    let mut labels = Vec::new();
    for item in items {
        match item {
            muda::MenuItemKind::Submenu(sub) => labels.push(sub.text()),
            muda::MenuItemKind::MenuItem(item) => labels.push(item.text()),
            muda::MenuItemKind::Check(item) => labels.push(item.text()),
            muda::MenuItemKind::Predefined(item) => labels.push(item.text()),
            // Icon items carry no label of interest to the tests.
            muda::MenuItemKind::Icon(_) => {}
        }
    }
    labels
}

/// The top-level menu titles and the labels of their direct items, as built (#1622).
fn summarize_menu(menu: &Menu) -> Vec<(String, Vec<String>)> {
    let mut sections = Vec::new();
    for item in menu.items() {
        if let Some(sub) = item.as_submenu() {
            sections.push((sub.text(), summarize_items(sub.items())));
        }
    }
    sections
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
            new_tab: MenuId::new("new_tab"),
            open: MenuId::new("open"),
            save: MenuId::new("save"),
            save_as: MenuId::new("save_as"),
            rebuild_geometry: MenuId::new("rebuild_geometry"),
            export_stl: MenuId::new("export_stl"),
            export_3mf: MenuId::new("export_3mf"),
            export_step: MenuId::new("export_step"),
            export_lua: MenuId::new("export_lua"),
            load_script: MenuId::new("load_script"),
            import_stl: MenuId::new("import_stl"),
            import_unit: MenuId::new("import_unit"),
            import_mcmaster: MenuId::new("import_mcmaster"),
            import_image: MenuId::new("import_image"),
            import_step: MenuId::new("import_step"),
            import_lua: MenuId::new("import_lua"),
            document_json: MenuId::new("document_json"),
            quit: MenuId::new("quit"),
            undo: MenuId::new("undo"),
            clear: MenuId::new("clear"),
            copy: MenuId::new("copy"),
            paste: MenuId::new("paste"),
            paste_linked: MenuId::new("paste_linked"),
            new_drawing: MenuId::new("new_drawing"),
            about: MenuId::new("about"),
            licenses: MenuId::new("licenses"),
            install_cli: MenuId::new("install_cli"),
            install_ai_skill: MenuId::new("install_ai_skill"),
            mcp_server: MenuId::new("mcp_server"),
            command_palette: MenuId::new("command_palette"),
            fps_mode: MenuId::new("fps_mode"),
            zoom_to_fit: MenuId::new("zoom_to_fit"),
            shortcuts_view: MenuId::new("shortcuts_view"),
            shortcuts_help: MenuId::new("shortcuts_help"),
            changelog: MenuId::new("changelog"),
            report_problem: MenuId::new("report_problem"),
            settings: MenuId::new("settings"),
            help_mode: MenuId::new("help_mode"),
            tool_hints: MenuId::new("tool_hints"),
            report_issue: MenuId::new("report_issue"),
            verify_lua_export: MenuId::new("verify_lua_export"),
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
            command_for_id(&ids.export_lua, &ids, |_| true),
            Some(MenuCommand::ExportLua)
        );
        assert_eq!(
            command_for_id(&ids.import_lua, &ids, |_| true),
            Some(MenuCommand::ImportLua)
        );
        assert_eq!(
            command_for_id(&ids.verify_lua_export, &ids, |_| true),
            Some(MenuCommand::VerifyLuaExport)
        );
        assert_eq!(
            command_for_id(&ids.install_cli, &ids, |_| true),
            Some(MenuCommand::InstallCli)
        );
    }

    #[test]
    fn maps_mcp_server_menu_item() {
        // #1622: Integration ▸ AI MCP Server… opens the AI pane at its MCP Server section.
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.mcp_server, &ids, |_| true),
            Some(MenuCommand::McpServer)
        );
        assert_eq!(
            MenuCommand::McpServer.to_action(),
            Some(Action::ShowMcpServerSection)
        );
        assert_eq!(
            command_for_id(&ids.install_ai_skill, &ids, |_| true),
            Some(MenuCommand::InstallAiSkill)
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
    fn maps_settings_menu_item() {
        // #720: the Settings… item opens the Settings window (no direct Action — the
        // window is frame-loop state, like Keyboard Shortcuts).
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.settings, &ids, |_| true),
            Some(MenuCommand::ShowSettings)
        );
        assert_eq!(MenuCommand::ShowSettings.to_action(), None);
    }

    #[test]
    fn maps_changelog_menu_item() {
        // #1328: Help ▸ Changelog opens the changelog window for this build.
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.changelog, &ids, |_| true),
            Some(MenuCommand::ShowChangelog)
        );
        assert_eq!(
            MenuCommand::ShowChangelog.to_action(),
            Some(Action::SetChangelogWindow { open: Some(true) })
        );
    }

    #[test]
    fn maps_report_problem_menu_item() {
        // #1372: Help ▸ Report Problem… is a browser link, so it has no direct Action.
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.report_problem, &ids, |_| true),
            Some(MenuCommand::ReportProblem)
        );
        assert_eq!(MenuCommand::ReportProblem.to_action(), None);
    }

    #[test]
    fn maps_help_mode_menu_item() {
        // #672: Help ▸ Help Mode (Cmd/Ctrl+/) toggles the pane-note overlay.
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.help_mode, &ids, |_| true),
            Some(MenuCommand::ToggleHelpMode)
        );
        assert_eq!(
            MenuCommand::ToggleHelpMode.to_action(),
            Some(Action::SetHelpMode(None))
        );
    }

    #[test]
    fn maps_tool_hints_menu_item() {
        // #1509: View ▸ Tool Hints toggles the viewport usage overlay.
        let ids = ids_with_pane("view_cube").0;
        assert_eq!(
            command_for_id(&ids.tool_hints, &ids, |_| true),
            Some(MenuCommand::ToggleToolHints)
        );
        assert_eq!(
            MenuCommand::ToggleToolHints.to_action(),
            Some(Action::SetToolHints(None))
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