//! Scene element selection from the elements pane and viewport.

use crate::element_picker::ElementPicker;
use crate::hierarchy::SceneElement;
use eframe::egui;

/// **Shift**+click adds to the current selection instead of replacing it — on every platform
/// (#984). ⌘/Ctrl used to do it too, which cost more than it bought: egui's `command` is Ctrl
/// everywhere but macOS, so Ctrl was spoken for on Linux and Windows and couldn't mean
/// "just the edge under the cursor, not its tangent run" there. One modifier, one meaning,
/// the same keys on every platform.
pub fn additive_click_modifiers(modifiers: &egui::Modifiers) -> bool {
    modifiers.shift
}

/// The scene selection — **the Select tool's element picker** (#966).
///
/// The picker is canonical, not a view of a separate set. A tool can have several pickers, each
/// with its own set, so no document-wide selection can own them; what the Select tool holds is
/// simply the one picker whose set every other consumer reads. This type is the name those ~200
/// read sites use for it (`is_selected`, `iter`, `single`, …) — a projection in the sense that
/// it exposes a flat set, not in the sense that it stores one.
///
/// Two things fall out of the picker being the store. Its `Vec` has a **stable order**, which is
/// what the popup's rows needed — they used to be sorted by each element's debug string purely
/// so that index→element agreed across the frame that draws a row and the frame that handles
/// its ✕. And its filter and limit are real: whatever rules the Select picker is given are
/// enforced on the way in rather than after the fact.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneSelection {
    picker: ElementPicker,
}

impl Default for SceneSelection {
    fn default() -> Self {
        SceneSelection {
            picker: ElementPicker::select_everything(),
        }
    }
}

impl SceneSelection {
    /// The picker itself, for the context pane to render and for the pick paths to consult.
    pub fn picker(&self) -> &ElementPicker {
        &self.picker
    }

    pub fn is_empty(&self) -> bool {
        self.picker.is_empty()
    }

    pub fn is_selected(&self, element: SceneElement) -> bool {
        self.picker.contains(&element)
    }

    pub fn iter(&self) -> impl Iterator<Item = SceneElement> + '_ {
        self.picker.picked().iter().cloned()
    }

    pub fn clear(&mut self) {
        self.picker.clear();
    }

    /// Drop selected elements that no longer satisfy `keep` (e.g. removed by a delete).
    pub fn retain(&mut self, keep: impl FnMut(&SceneElement) -> bool) {
        self.picker.retain(keep);
    }

    /// Add an element to the selection (idempotent). Used to fold a tool's in-progress picked
    /// set (e.g. Loft cross sections, #202) into the selection the viewport renders, so picked
    /// elements show their selection highlight without touching the persistent selection.
    pub fn insert(&mut self, element: SceneElement) {
        if !self.picker.contains(&element) {
            self.picker.push(element);
        }
    }

    /// The selection in pick order (#202/#966). The Select tool's picker needs a stable row
    /// order, and — because the remove button is handled a frame after the rows are built —
    /// index→element must agree across both. A `Vec` gives that for free.
    pub fn ordered(&self) -> Vec<SceneElement> {
        self.picker.picked().to_vec()
    }

    /// The sole selected element, if exactly one is selected.
    pub fn single(&self) -> Option<SceneElement> {
        match self.picker.picked() {
            [only] => Some(only.clone()),
            _ => None,
        }
    }
}

/// How many elements a context-menu Delete opened on `element` would take (#1853).
///
/// A menu opened on one of several selected rows acts on the selection, not on that row
/// alone; opened anywhere else it acts on just what was clicked.
pub fn delete_menu_count(selection: &SceneSelection, element: &SceneElement) -> usize {
    let picked = selection.ordered().len();
    if picked > 1 && selection.is_selected(element.clone()) {
        picked
    } else {
        1
    }
}

/// Click an elements row: deselect when already selected; replace selection unless additive.
///
/// This is the Select picker's pick behaviour (#966): additive toggles, plain replaces — so a
/// plain click on something already selected still removes it, which is what
/// [`PickOutcome::Removed`] means.
pub fn click_scene_selection(
    selection: &mut SceneSelection,
    element: SceneElement,
    additive: bool,
) {
    if selection.is_selected(element.clone()) {
        selection.picker.retain(|e| *e != element);
        return;
    }
    if !additive {
        selection.picker.clear();
    }
    selection.picker.push(element);
}

/// [`click_scene_selection`] for a click that takes several elements as **one unit** (#984) —
/// a tangent-continuous run of edges. All already selected deselects the whole unit; anything
/// short of that selects all of it (replacing unless additive), so a mixed state converges on
/// selected rather than toggling piecewise.
pub fn click_scene_selection_many(
    selection: &mut SceneSelection,
    elements: Vec<SceneElement>,
    additive: bool,
) {
    if elements.is_empty() {
        return;
    }
    if elements.iter().all(|e| selection.is_selected(e.clone())) {
        selection.picker.retain(|e| !elements.contains(e));
        return;
    }
    if !additive {
        selection.picker.clear();
    }
    for element in elements {
        selection.insert(element);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::circle_key_for_slot as rkey;
    use super::*;

    fn selection_count(selection: &SceneSelection) -> usize {
        selection.iter().count()
    }

    fn selection_single(selection: &SceneSelection) -> Option<SceneElement> {
        let mut iter = selection.iter();
        let first = iter.next()?;
        if iter.next().is_none() {
            Some(first)
        } else {
            None
        }
    }

    #[test]
    fn single_returns_one_selected_element() {
        let mut sel = SceneSelection::default();
        assert_eq!(sel.single(), None);
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        assert_eq!(sel.single(), Some(SceneElement::Circle(rkey(0))));
        click_scene_selection(&mut sel, SceneElement::Line(lkey(1)), true);
        assert_eq!(sel.single(), None);
    }

    #[test]
    fn click_replaces_selection_without_modifier() {
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        assert_eq!(selection_single(&sel), Some(SceneElement::Circle(rkey(0))));
        click_scene_selection(&mut sel, SceneElement::Line(lkey(1)), false);
        assert_eq!(selection_single(&sel), Some(SceneElement::Line(lkey(1))));
    }

    #[test]
    fn click_selected_deselects() {
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        assert!(sel.is_empty());
    }

    /// #984: a tangent run selects and deselects as one unit, and a partially-selected run
    /// converges on fully selected rather than toggling its members piecewise.
    #[test]
    fn a_run_selects_and_deselects_as_one_unit() {
        let run = || vec![SceneElement::Line(lkey(0)), SceneElement::Line(lkey(1)), SceneElement::Line(lkey(2))];
        let mut sel = SceneSelection::default();
        click_scene_selection_many(&mut sel, run(), false);
        assert_eq!(selection_count(&sel), 3);
        // All selected: the whole run comes back out.
        click_scene_selection_many(&mut sel, run(), false);
        assert!(sel.is_empty());
        // Partly selected: the click completes the run rather than removing what's there.
        click_scene_selection(&mut sel, SceneElement::Line(lkey(1)), false);
        click_scene_selection_many(&mut sel, run(), true);
        assert_eq!(selection_count(&sel), 3);
    }

    /// A plain run click replaces the selection; an additive one adds to it.
    #[test]
    fn a_plain_run_click_replaces_and_an_additive_one_adds() {
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(9)), false);
        click_scene_selection_many(
            &mut sel,
            vec![SceneElement::Line(lkey(0)), SceneElement::Line(lkey(1))],
            false,
        );
        assert_eq!(selection_count(&sel), 2);
        assert!(!sel.is_selected(SceneElement::Circle(rkey(9))));
        click_scene_selection_many(&mut sel, vec![SceneElement::Circle(rkey(9))], true);
        assert_eq!(selection_count(&sel), 3);
    }

    #[test]
    fn additive_click_builds_multi_selection() {
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        click_scene_selection(&mut sel, SceneElement::Line(lkey(1)), true);
        assert_eq!(selection_count(&sel), 2);
        assert!(sel.is_selected(SceneElement::Circle(rkey(0))));
        assert!(sel.is_selected(SceneElement::Line(lkey(1))));
    }

    #[test]
    fn additive_click_selected_deselects_one() {
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), false);
        click_scene_selection(&mut sel, SceneElement::Line(lkey(1)), true);
        click_scene_selection(&mut sel, SceneElement::Circle(rkey(0)), true);
        assert_eq!(selection_single(&sel), Some(SceneElement::Line(lkey(1))));
    }

    /// #984: Shift is the additive modifier on every platform, and ⌘/Ctrl is not — which is
    /// what leaves Ctrl free everywhere to mean "just the edge under the cursor". egui's
    /// `command` is Ctrl on Linux/Windows and ⌘ on macOS, so both spellings must stand down.
    #[test]
    fn additive_click_modifiers_ignores_command_and_ctrl() {
        for modifiers in [
            egui::Modifiers { command: true, ..Default::default() },
            egui::Modifiers { ctrl: true, ..Default::default() },
            egui::Modifiers { mac_cmd: true, command: true, ..Default::default() },
        ] {
            assert!(
                !additive_click_modifiers(&modifiers),
                "{modifiers:?} must not add to the selection"
            );
        }
    }

    /// Shift still wins when it is held alongside Control — a Shift+Ctrl+click adds the one
    /// edge under the cursor rather than its whole run.
    #[test]
    fn additive_click_modifiers_shift_with_ctrl() {
        let modifiers = egui::Modifiers {
            shift: true,
            ctrl: true,
            ..Default::default()
        };
        assert!(additive_click_modifiers(&modifiers));
    }

    #[test]
    fn additive_click_modifiers_shift() {
        let modifiers = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        assert!(additive_click_modifiers(&modifiers));
    }

    #[test]
    fn additive_click_modifiers_plain_click() {
        assert!(!additive_click_modifiers(&egui::Modifiers::default()));
    }
}