#![allow(dead_code)] // wired up progressively from main; public API is intentional

//! Document tabs and multi-window tab hosts.
//!
//! A **window** holds an ordered list of **tabs**. Each tab shows one document with its
//! own view state (camera, tool, drawing workbench, selection). Several tabs — across one
//! or more windows — may share the same [`DocumentId`]; their document cores stay in sync
//! after every mutation of any of them.
//!
//! Closing the last view of a dirty document asks the host to warn; closing the last tab
//! of the last window opens a blank document instead of quitting.

use crate::actions::AppState;
use crate::model::Document;

/// Stable id for a tab (per session).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Identity of a document across tabs/windows. Tabs that share this id share one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

/// Host window id. [`WindowId::MAIN`] is the root OS window; others are detached viewports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

impl WindowId {
    pub const MAIN: Self = Self(0);
}

/// One tab: a view onto a document.
pub struct Tab {
    pub id: TabId,
    pub document_id: DocumentId,
    pub state: AppState,
}

/// One OS window's tab strip and active tab.
pub struct TabWindow {
    pub id: WindowId,
    /// Hash key for `ViewportId::from_hash_of` on detached windows (unused for main).
    pub viewport_key: u64,
    pub tabs: Vec<Tab>,
    pub active: usize,
}

/// What should happen after a close request on a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    /// Close immediately.
    Close,
    /// Document is dirty and this is its last view — host should prompt Save / Don't Save / Cancel.
    PromptSave { document_id: DocumentId },
}

/// Outcome of closing a tab (after the host has approved any save prompt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Tab closed; another tab in the same window is now active.
    Closed,
    /// Tab closed and its window is now empty (and was not the last window) — host should
    /// destroy the window.
    WindowEmpty { window: WindowId },
    /// Last tab of the last window was closed; a fresh blank tab was opened in its place.
    ReplacedWithBlank,
}

/// All windows and id counters for the session.
pub struct Workspace {
    pub windows: Vec<TabWindow>,
    /// The one AI state every tab shares (#1598). The MCP server belongs to
    /// the app, so a tab created here adopts this handle rather than starting its own.
    pub ai: crate::ai::SharedAi,
    next_tab_id: u64,
    next_doc_id: u64,
    next_window_id: u64,
    next_viewport_key: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        let mut ws = Self {
            windows: Vec::new(),
            ai: crate::ai::SharedAi::default(),
            next_tab_id: 1,
            next_doc_id: 1,
            next_window_id: 1,
            next_viewport_key: 1,
        };
        let tab = ws.make_blank_tab();
        ws.windows.push(TabWindow {
            id: WindowId::MAIN,
            viewport_key: 0,
            tabs: vec![tab],
            active: 0,
        });
        ws
    }

    fn alloc_tab_id(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    fn alloc_doc_id(&mut self) -> DocumentId {
        let id = DocumentId(self.next_doc_id);
        self.next_doc_id += 1;
        id
    }

    fn alloc_window_id(&mut self) -> WindowId {
        let id = WindowId(self.next_window_id);
        self.next_window_id += 1;
        id
    }

    fn alloc_viewport_key(&mut self) -> u64 {
        let k = self.next_viewport_key;
        self.next_viewport_key += 1;
        k
    }

    pub fn make_blank_tab(&mut self) -> Tab {
        let mut state = AppState::default();
        state.ai = std::rc::Rc::clone(&self.ai);
        Tab {
            id: self.alloc_tab_id(),
            document_id: self.alloc_doc_id(),
            state,
        }
    }

    pub fn main(&self) -> &TabWindow {
        &self.windows[0]
    }

    pub fn main_mut(&mut self) -> &mut TabWindow {
        &mut self.windows[0]
    }

    pub fn window_index(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    pub fn find_tab(&self, tab_id: TabId) -> Option<(usize, usize)> {
        for (wi, win) in self.windows.iter().enumerate() {
            if let Some(ti) = win.tabs.iter().position(|t| t.id == tab_id) {
                return Some((wi, ti));
            }
        }
        None
    }

    /// Title shown on a tab (and the window title when that tab is active).
    ///
    /// Basename without the `.bearcad` / `.bearcad.json` extension (or "Untitled"), a leading
    /// `*` when dirty, and — when the tab is in a sketch or drawing workbench — the open
    /// sketch/drawing's name: `{basename} {view_name}`. Main modeling view shows only the
    /// basename (the view portion is blank) (#1137).
    pub fn tab_title(state: &AppState) -> String {
        let basename = document_basename(state.path.as_deref());
        let name = match view_name_suffix(state) {
            Some(view) => format!("{basename} {view}"),
            None => basename,
        };
        if state.dirty {
            format!("*{name}")
        } else {
            name
        }
    }
}

/// File basename for a tab title: drop `.bearcad` / `.bearcad.json` so tabs read "bracket",
/// not "bracket.bearcad" (#1137).
fn document_basename(path: Option<&str>) -> String {
    let Some(path) = path else {
        return "Untitled".to_string();
    };
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());
    if let Some(stem) = file_name.strip_suffix(".bearcad.json") {
        stem.to_string()
    } else if let Some(stem) = file_name.strip_suffix(".bearcad") {
        stem.to_string()
    } else {
        std::path::Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(file_name)
    }
}

/// Name of the open sketch or drawing workbench for the tab title, if any (#1137).
fn view_name_suffix(state: &AppState) -> Option<String> {
    // Drawing workbench and sketch session are mutually exclusive in normal use; prefer the
    // drawing when set so leaving a sketch before opening a sheet doesn't leave a stale name.
    if let Some(di) = state.editing_drawing {
        return Some(crate::names::node_label(
            &state.doc,
            crate::hierarchy::HierarchyNode::Drawing(di),
        ));
    }
    if let Some(session) = state.sketch_session {
        return Some(crate::names::node_label(
            &state.doc,
            crate::hierarchy::HierarchyNode::Sketch(session.sketch),
        ));
    }
    None
}

impl Workspace {
    /// How many tabs (across all windows) currently show `document_id`.
    pub fn document_view_count(&self, document_id: DocumentId) -> usize {
        self.windows
            .iter()
            .flat_map(|w| w.tabs.iter())
            .filter(|t| t.document_id == document_id)
            .count()
    }

    /// Open a new blank document tab in `window`, activate it, return its index.
    pub fn open_blank_tab(&mut self, window: WindowId) -> Option<usize> {
        let wi = self.window_index(window)?;
        let tab = self.make_blank_tab();
        let win = &mut self.windows[wi];
        win.tabs.push(tab);
        win.active = win.tabs.len() - 1;
        Some(win.active)
    }

    /// Open a new tab in `window` showing the same document as `source_tab`, with a fresh
    /// view (camera/tool/selection). Document core is copied from the source.
    pub fn open_same_document_tab(&mut self, window: WindowId, source_tab: TabId) -> Option<usize> {
        let (swi, sti) = self.find_tab(source_tab)?;
        let doc_id = self.windows[swi].tabs[sti].document_id;
        let mut state = AppState::default();
        state.ai = std::rc::Rc::clone(&self.ai);
        copy_document_core(&self.windows[swi].tabs[sti].state, &mut state);
        // Carry the drawing workbench flag only if we want independent views — leave
        // editing_drawing unset so the new tab opens on the 3D model; the user can open
        // a drawing there separately.
        let tab = Tab {
            id: self.alloc_tab_id(),
            document_id: doc_id,
            state,
        };
        let wi = self.window_index(window)?;
        let win = &mut self.windows[wi];
        win.tabs.push(tab);
        win.active = win.tabs.len() - 1;
        Some(win.active)
    }

    /// Decide whether closing tab `tab_id` needs a save prompt.
    pub fn close_decision(&self, tab_id: TabId) -> Option<CloseDecision> {
        let (wi, ti) = self.find_tab(tab_id)?;
        let tab = &self.windows[wi].tabs[ti];
        if tab.state.dirty && self.document_view_count(tab.document_id) == 1 {
            Some(CloseDecision::PromptSave {
                document_id: tab.document_id,
            })
        } else {
            Some(CloseDecision::Close)
        }
    }

    /// Close `tab_id` (caller already handled any save prompt). Enforces the last-tab rule.
    pub fn close_tab(&mut self, tab_id: TabId) -> Option<CloseOutcome> {
        let (wi, ti) = self.find_tab(tab_id)?;
        let window_id = self.windows[wi].id;
        let sole_tab = self.windows[wi].tabs.len() == 1;
        if sole_tab {
            if self.windows.len() == 1 {
                // Last tab of last window → blank document.
                let fresh = self.make_blank_tab();
                self.windows[0].tabs[0] = fresh;
                self.windows[0].active = 0;
                return Some(CloseOutcome::ReplacedWithBlank);
            }
            // Empty a non-main window: remove it.
            self.windows.remove(wi);
            return Some(CloseOutcome::WindowEmpty { window: window_id });
        }
        let win = &mut self.windows[wi];
        win.tabs.remove(ti);
        if win.active >= win.tabs.len() {
            win.active = win.tabs.len() - 1;
        } else if ti < win.active {
            win.active -= 1;
        }
        Some(CloseOutcome::Closed)
    }

    /// Activate tab index `index` in `window`.
    pub fn select_tab(&mut self, window: WindowId, index: usize) -> bool {
        let Some(wi) = self.window_index(window) else {
            return false;
        };
        let win = &mut self.windows[wi];
        if index >= win.tabs.len() {
            return false;
        }
        win.active = index;
        true
    }

    /// Move tab at `from` to `to` within the same window (reorder).
    pub fn reorder_tab(&mut self, window: WindowId, from: usize, to: usize) -> bool {
        let Some(wi) = self.window_index(window) else {
            return false;
        };
        let win = &mut self.windows[wi];
        let n = win.tabs.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let tab = win.tabs.remove(from);
        win.tabs.insert(to, tab);
        // Keep the same tab active.
        if win.active == from {
            win.active = to;
        } else if from < win.active && to >= win.active {
            win.active -= 1;
        } else if from > win.active && to <= win.active {
            win.active += 1;
        }
        true
    }

    /// Detach tab `tab_id` into a new window (alone). Returns the new window id.
    pub fn detach_tab(&mut self, tab_id: TabId) -> Option<WindowId> {
        let (wi, ti) = self.find_tab(tab_id)?;
        let win_id = self.windows[wi].id;
        let sole = self.windows[wi].tabs.len() == 1;
        // Already the only tab in a detached window.
        if sole && win_id != WindowId::MAIN {
            return Some(win_id);
        }
        // Sole tab of main: leave a blank in main, move content out.
        if sole && self.windows.len() == 1 {
            let blank = self.make_blank_tab();
            let tab = std::mem::replace(&mut self.windows[0].tabs[0], blank);
            self.windows[0].active = 0;
            let new_id = self.alloc_window_id();
            let viewport_key = self.alloc_viewport_key();
            self.windows.push(TabWindow {
                id: new_id,
                viewport_key,
                tabs: vec![tab],
                active: 0,
            });
            return Some(new_id);
        }
        let tab = self.windows[wi].tabs.remove(ti);
        let win = &mut self.windows[wi];
        if win.active >= win.tabs.len() {
            win.active = win.tabs.len().saturating_sub(1);
        } else if ti < win.active {
            win.active -= 1;
        }
        if win.tabs.is_empty() {
            let blank = self.make_blank_tab();
            // wi still valid: we didn't remove the window.
            self.windows[wi].tabs.push(blank);
            self.windows[wi].active = 0;
        }
        let new_id = self.alloc_window_id();
        let viewport_key = self.alloc_viewport_key();
        self.windows.push(TabWindow {
            id: new_id,
            viewport_key,
            tabs: vec![tab],
            active: 0,
        });
        Some(new_id)
    }

    /// After a tab's document core changed, copy it to every other tab with the same
    /// [`DocumentId`]. Reads the source from the tab's parked state.
    pub fn sync_document(&mut self, document_id: DocumentId, source_tab: TabId) {
        let Some((swi, sti)) = self.find_tab(source_tab) else {
            return;
        };
        let core = DocumentCore::from_state(&self.windows[swi].tabs[sti].state);
        self.apply_document_core(document_id, source_tab, &core);
    }

    /// Like [`Self::sync_document`], but the source state is supplied externally (the live
    /// `App.state` for the active main tab, which is not stored in the workspace slot).
    pub fn sync_document_from(
        &mut self,
        document_id: DocumentId,
        source_tab: TabId,
        source: &AppState,
    ) {
        let core = DocumentCore::from_state(source);
        self.apply_document_core(document_id, source_tab, &core);
    }

    fn apply_document_core(
        &mut self,
        document_id: DocumentId,
        source_tab: TabId,
        core: &DocumentCore,
    ) {
        for win in &mut self.windows {
            for tab in &mut win.tabs {
                if tab.document_id == document_id && tab.id != source_tab {
                    core.apply_to(&mut tab.state);
                }
            }
        }
    }

    /// Assign a fresh document id (e.g. after File → New on a tab).
    pub fn rebind_document(&mut self, tab_id: TabId) {
        if let Some((wi, ti)) = self.find_tab(tab_id) {
            self.windows[wi].tabs[ti].document_id = self.alloc_doc_id();
        }
    }

    /// Total tab count across all windows.
    pub fn tab_count(&self) -> usize {
        self.windows.iter().map(|w| w.tabs.len()).sum()
    }
}

/// Map a 1-based keyboard ordinal (`1`–`9`) to a 0-based tab index when that tab exists.
/// Used by Cmd/Ctrl+1…9 (#1130).
pub fn tab_index_for_number(number: usize, tab_count: usize) -> Option<usize> {
    if (1..=9).contains(&number) && number <= tab_count {
        Some(number - 1)
    } else {
        None
    }
}

/// Next tab index after moving `delta` steps from `active`, wrapping in `0..tab_count`.
/// Returns `None` when there is nothing to switch to (`tab_count < 2`).
/// Used by Cmd/Ctrl+Alt+←/→ (#1131).
pub fn adjacent_tab_index(active: usize, tab_count: usize, delta: isize) -> Option<usize> {
    if tab_count < 2 {
        return None;
    }
    let n = tab_count as isize;
    let next = (active as isize + delta).rem_euclid(n) as usize;
    if next == active {
        None
    } else {
        Some(next)
    }
}

/// Document-owned fields shared by every tab of the same [`DocumentId`].
struct DocumentCore {
    doc: Document,
    path: Option<String>,
    saved_snapshot: Document,
    dirty: bool,
    #[cfg(not(target_arch = "wasm32"))]
    document_session: Option<std::rc::Rc<std::cell::RefCell<crate::storage::DocumentSession>>>,
    undo_stack: Vec<Document>,
    redo_stack: Vec<Document>,
    construction_plane_edit_undo: Vec<crate::arena::Arena<crate::model::ConstructionPlane>>,
    document_health: crate::document_health::DocumentHealth,
    kernel_fallback_warning: Option<String>,
}

impl DocumentCore {
    fn from_state(state: &AppState) -> Self {
        Self {
            doc: state.doc.clone(),
            path: state.path.clone(),
            saved_snapshot: state.saved_snapshot.clone(),
            dirty: state.dirty,
            #[cfg(not(target_arch = "wasm32"))]
            document_session: state.document_session.clone(),
            undo_stack: state.undo_stack.clone(),
            redo_stack: state.redo_stack.clone(),
            construction_plane_edit_undo: state.construction_plane_edit_undo.clone(),
            document_health: state.document_health.clone(),
            kernel_fallback_warning: state.kernel_fallback_warning.clone(),
        }
    }

    fn apply_to(&self, state: &mut AppState) {
        state.doc = self.doc.clone();
        state.path = self.path.clone();
        state.saved_snapshot = self.saved_snapshot.clone();
        state.dirty = self.dirty;
        #[cfg(not(target_arch = "wasm32"))]
        {
            state.document_session = self.document_session.clone();
        }
        state.undo_stack = self.undo_stack.clone();
        state.redo_stack = self.redo_stack.clone();
        state.construction_plane_edit_undo = self.construction_plane_edit_undo.clone();
        state.document_health = self.document_health.clone();
        state.kernel_fallback_warning = self.kernel_fallback_warning.clone();
    }
}

/// Copy document core from one tab state into another (same document, new view).
pub fn copy_document_core(from: &AppState, to: &mut AppState) {
    DocumentCore::from_state(from).apply_to(to);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;
    use crate::model::FaceId;

    fn dirty_state() -> AppState {
        let mut state = AppState::default();
        let plane = state
            .doc
            .ground_plane()
            .expect("default doc has ground plane");
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(plane),
            viewport: None,
        });
        if !state.dirty {
            state.dirty = true;
        }
        state
    }

    #[test]
    fn workspace_starts_with_one_blank_tab() {
        let ws = Workspace::new();
        assert_eq!(ws.windows.len(), 1);
        assert_eq!(ws.main().tabs.len(), 1);
        assert_eq!(ws.main().active, 0);
        assert!(!ws.main().tabs[0].state.dirty);
        assert_eq!(Workspace::tab_title(&ws.main().tabs[0].state), "Untitled");
    }

    #[test]
    fn open_blank_tab_activates_new_document() {
        let mut ws = Workspace::new();
        let first_doc = ws.main().tabs[0].document_id;
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        assert_eq!(ws.main().tabs.len(), 2);
        assert_eq!(ws.main().active, 1);
        assert_ne!(ws.main().tabs[1].document_id, first_doc);
    }

    #[test]
    fn close_last_tab_of_last_window_opens_blank() {
        let mut ws = Workspace::new();
        let id = ws.main().tabs[0].id;
        let old_doc = ws.main().tabs[0].document_id;
        let outcome = ws.close_tab(id).unwrap();
        assert_eq!(outcome, CloseOutcome::ReplacedWithBlank);
        assert_eq!(ws.main().tabs.len(), 1);
        assert_ne!(ws.main().tabs[0].document_id, old_doc);
        assert_ne!(ws.main().tabs[0].id, id);
    }

    #[test]
    fn close_decision_prompts_when_last_dirty_view() {
        let mut ws = Workspace::new();
        ws.main_mut().tabs[0].state = dirty_state();
        let id = ws.main().tabs[0].id;
        let doc = ws.main().tabs[0].document_id;
        assert_eq!(
            ws.close_decision(id),
            Some(CloseDecision::PromptSave { document_id: doc })
        );
    }

    #[test]
    fn close_decision_is_silent_when_another_view_exists() {
        let mut ws = Workspace::new();
        ws.main_mut().tabs[0].state = dirty_state();
        let source = ws.main().tabs[0].id;
        ws.open_same_document_tab(WindowId::MAIN, source).unwrap();
        // Both tabs share the dirty document; closing one does not need a prompt.
        let other = ws.main().tabs[1].id;
        assert_eq!(ws.close_decision(other), Some(CloseDecision::Close));
        assert_eq!(ws.document_view_count(ws.main().tabs[0].document_id), 2);
    }

    #[test]
    fn same_document_tabs_sync_core_after_edit() {
        let mut ws = Workspace::new();
        let source = ws.main().tabs[0].id;
        ws.open_same_document_tab(WindowId::MAIN, source).unwrap();
        let doc_id = ws.main().tabs[0].document_id;
        assert_eq!(ws.main().tabs[1].document_id, doc_id);

        // Edit tab 0's document (add a named parameter — pure document mutation).
        ws.main_mut().tabs[0].state.apply(Action::AddParameter {
            name: "w".into(),
            expression: "10".into(),
        });
        assert!(ws.main().tabs[0].state.dirty);
        assert_eq!(ws.main().tabs[0].state.doc.parameters.len(), 1);

        ws.sync_document(doc_id, source);
        assert_eq!(ws.main().tabs[1].state.doc.parameters.len(), 1);
        assert_eq!(ws.main().tabs[1].state.dirty, ws.main().tabs[0].state.dirty);
        // View state stays independent.
        assert!(ws.main().tabs[1].state.sketch_session.is_none());
    }

    #[test]
    fn reorder_tabs_keeps_active_identity() {
        let mut ws = Workspace::new();
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        // tabs: 0, 1, 2; active = 2
        let active_id = ws.main().tabs[2].id;
        assert!(ws.reorder_tab(WindowId::MAIN, 2, 0));
        assert_eq!(ws.main().tabs[0].id, active_id);
        assert_eq!(ws.main().active, 0);
    }

    #[test]
    fn detach_tab_creates_new_window() {
        let mut ws = Workspace::new();
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        let tab_id = ws.main().tabs[1].id;
        let new_win = ws.detach_tab(tab_id).unwrap();
        assert_ne!(new_win, WindowId::MAIN);
        assert_eq!(ws.windows.len(), 2);
        assert_eq!(ws.main().tabs.len(), 1);
        let detached = ws.windows.iter().find(|w| w.id == new_win).unwrap();
        assert_eq!(detached.tabs.len(), 1);
        assert_eq!(detached.tabs[0].id, tab_id);
    }

    #[test]
    fn detach_sole_tab_leaves_blank_in_main() {
        let mut ws = Workspace::new();
        let tab_id = ws.main().tabs[0].id;
        let new_win = ws.detach_tab(tab_id).unwrap();
        assert_eq!(ws.main().tabs.len(), 1);
        assert_ne!(ws.main().tabs[0].id, tab_id);
        let detached = ws.windows.iter().find(|w| w.id == new_win).unwrap();
        assert_eq!(detached.tabs[0].id, tab_id);
    }

    #[test]
    fn tab_title_shows_dirty_star() {
        let mut state = AppState::default();
        assert_eq!(Workspace::tab_title(&state), "Untitled");
        state.dirty = true;
        assert_eq!(Workspace::tab_title(&state), "*Untitled");
        state.path = Some("/tmp/bracket.bearcad".into());
        // Modeling view: basename only, no `.bearcad` (#1137).
        assert_eq!(Workspace::tab_title(&state), "*bracket");
        state.dirty = false;
        assert_eq!(Workspace::tab_title(&state), "bracket");
    }

    /// #1137: tab titles drop `.bearcad` / `.bearcad.json` and show the open view name.
    #[test]
    fn tab_title_strips_extension_and_shows_view() {
        let mut state = AppState::default();
        state.path = Some("/tmp/bracket.bearcad".into());
        assert_eq!(Workspace::tab_title(&state), "bracket");

        // JSON save path strips both suffixes.
        state.path = Some("/tmp/bracket.bearcad.json".into());
        assert_eq!(Workspace::tab_title(&state), "bracket");

        // Open a sketch: "{basename} {sketch_name}".
        let plane = state
            .doc
            .ground_plane()
            .expect("default doc has ground plane");
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(plane),
            viewport: None,
        });
        state.path = Some("/tmp/bracket.bearcad".into());
        state.dirty = false;
        assert_eq!(Workspace::tab_title(&state), "bracket Sketch 0");

        // Custom sketch name.
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.sketches.get_mut(sketch).unwrap().name = Some("Front".into());
        assert_eq!(Workspace::tab_title(&state), "bracket Front");

        // Dirty star prefixes the whole title.
        state.dirty = true;
        assert_eq!(Workspace::tab_title(&state), "*bracket Front");

        // Leave sketch, open a drawing: "{basename} {drawing_name}".
        state.sketch_session = None;
        state.apply(Action::CreateDrawing { name: None });
        state.dirty = false; // title assertions ignore dirty for this branch
        assert_eq!(Workspace::tab_title(&state), "bracket Drawing 0");
        let drawing = state.editing_drawing.unwrap();
        state.apply(Action::RenameDrawing {
            drawing,
            name: "Sheet A".into(),
        });
        state.dirty = false;
        assert_eq!(Workspace::tab_title(&state), "bracket Sheet A");

        // Back to modeling: basename only (view portion blank).
        state.apply(Action::EditDrawing { drawing: None });
        state.dirty = false;
        assert_eq!(Workspace::tab_title(&state), "bracket");
    }

    #[test]
    fn close_non_last_tab_activates_neighbor() {
        let mut ws = Workspace::new();
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        ws.open_blank_tab(WindowId::MAIN).unwrap();
        // active = 2; close middle (1)
        let mid = ws.main().tabs[1].id;
        ws.select_tab(WindowId::MAIN, 1);
        assert_eq!(ws.close_tab(mid), Some(CloseOutcome::Closed));
        assert_eq!(ws.main().tabs.len(), 2);
        assert!(ws.main().active < 2);
    }

    /// #1130: Cmd/Ctrl+N maps to the Nth tab (1-based); out of range / 0 is a no-op.
    #[test]
    fn tab_index_for_number_maps_one_based_ordinals() {
        assert_eq!(tab_index_for_number(1, 3), Some(0));
        assert_eq!(tab_index_for_number(2, 3), Some(1));
        assert_eq!(tab_index_for_number(3, 3), Some(2));
        assert_eq!(tab_index_for_number(4, 3), None);
        assert_eq!(tab_index_for_number(0, 3), None);
        assert_eq!(tab_index_for_number(9, 9), Some(8));
        assert_eq!(tab_index_for_number(10, 20), None); // only 1–9 are bound
        assert_eq!(tab_index_for_number(1, 0), None);
    }

    /// #1131: adjacent navigation wraps; a single tab does nothing.
    #[test]
    fn adjacent_tab_index_wraps() {
        assert_eq!(adjacent_tab_index(0, 1, 1), None);
        assert_eq!(adjacent_tab_index(0, 3, 1), Some(1));
        assert_eq!(adjacent_tab_index(2, 3, 1), Some(0));
        assert_eq!(adjacent_tab_index(0, 3, -1), Some(2));
        assert_eq!(adjacent_tab_index(1, 3, -1), Some(0));
        assert_eq!(adjacent_tab_index(1, 3, 0), None);
    }
}
