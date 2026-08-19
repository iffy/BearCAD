//! Echo user actions as script instructions when `--show-commands` is enabled.

use crate::actions::Action;
use crate::model::Document;
use crate::script::{instruction_from_action, instructions_for_snap_constraint, Instruction};
use crate::camera::Camera;
use egui::Vec2;

const EPS: f32 = 1e-4;

/// Records interactive user actions as script instructions for `--show-commands` (stdout echo).
/// History is retained for diagnostics; document recreate export is File → Export → Lua (#1159).
#[derive(Clone, Debug, Default)]
pub struct CommandLog {
    pending_orbit: Vec2,
    pending_pan: Vec2,
    pending_zoom: f32,
    pending_discrete: Option<Instruction>,
    defer_baseline: bool,
    print_stdout: bool,
    /// The session as **rendered Lua**, not as instructions (#1070). An instruction naming an
    /// element has to be spelled while a document is at hand to count live elements against —
    /// `Instruction::as_lua` alone can only say the arena slot, which stops matching the
    /// ordinal a replay resolves the moment anything of that kind is deleted.
    history: Vec<String>,
    extrusion_count_before: usize,
    loft_count_before: usize,
    revolution_count_before: usize,
    sweep_count_before: usize,
    edge_treatment_op_count_before: usize,
    /// Constraints alive before the action ran (#1055): anything not in this set afterwards
    /// was added by it.
    constraints_before: Vec<crate::model::ConstraintKey>,
}

impl CommandLog {
    /// A recording log; `print_stdout` echoes each instruction to stdout (`--show-commands`).
    pub fn new_recording(print_stdout: bool) -> Self {
        Self {
            print_stdout,
            ..Self::default()
        }
    }

    /// Whether any instruction has been recorded this session.
    #[allow(dead_code)] // retained for --show-commands diagnostics / tests (#1159 removed export UI)
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    /// The recorded session as a replayable, timestamped Lua script.
    #[allow(dead_code)] // session export UI removed (#1159); still used by unit tests
    pub fn session_lua_script(&self, timestamp: &str) -> String {
        let mut out = String::new();
        out.push_str("-- BearCAD session commands\n");
        out.push_str(&format!("-- Exported {timestamp} UTC\n"));
        out.push_str("-- Replay headless with: cargo run -- --script <file> --exit\n\n");
        for line in &self.history {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    pub fn is_camera_action(action: &Action) -> bool {
        matches!(
            action,
            Action::OrbitCamera { .. }
                | Action::PanCamera { .. }
                | Action::ZoomCamera { .. }
                | Action::SetStandardView(_)
                | Action::SetViewEdge(_)
                | Action::SetViewCorner(_)
                | Action::ViewHome
                | Action::SetProjectionMode(_)
                | Action::ToggleProjectionMode
                | Action::SetShadingMode(_)
        )
    }

    fn is_automatic_camera_side_effect(action: &Action) -> bool {
        matches!(
            action,
            Action::BeginSketch { .. } | Action::OpenSketch { .. } | Action::ExitSketch
        )
    }

    fn should_log(action: &Action) -> bool {
        !matches!(
            action,
            Action::CancelOperation
                | Action::BeginConstructionPlane { .. }
                | Action::BeginDimensionEdit { .. }
        )
    }

    /// Actions that can silently add a constraint as a snapping side effect
    /// (`AppState::add_snap_constraint`), bypassing the normal `Action::AddGeometricConstraint`
    /// path the rest of the log relies on to hear about new constraints.
    fn can_add_snap_constraint(action: &Action) -> bool {
        matches!(
            action,
            Action::CommitRectangle
                | Action::CommitLine
                | Action::CommitCircle
                | Action::ApplySnapConstraint { .. }
        )
    }

    pub fn before_apply(&mut self, action: &Action, doc: &Document, cam: &Camera) {
        // CommitExtrusion is reused for both creating a new extrusion and editing an
        // existing one (#59); only the former is replayable via the declarative
        // `bearcad.extrude{}` call, so remember the pre-commit count to tell them apart
        // in `after_apply`.
        if matches!(action, Action::CommitExtrusion) {
            self.extrusion_count_before = doc.extrusions.len();
        }
        // Same new-vs-noop split for CommitLoft: only a commit that actually appended a
        // loft is replayable as `bearcad.loft{}`.
        if matches!(action, Action::CommitLoft) {
            self.loft_count_before = doc.lofts.len();
        }
        if matches!(action, Action::CommitRevolve) {
            self.revolution_count_before = doc.revolutions.len();
        }
        if matches!(action, Action::CommitSweep) {
            self.sweep_count_before = doc.sweeps.len();
        }
        if matches!(action, Action::CommitEdgeTreatments { .. }) {
            self.edge_treatment_op_count_before = doc.edge_treatment_ops.len();
        }
        if Self::can_add_snap_constraint(action) {
            self.constraints_before = doc.constraints.keys().collect();
        }
        if Self::should_log(action) && !Self::is_camera_action(action) {
            self.flush_camera(cam);
        }
    }

    pub fn after_apply(&mut self, action: Action, doc: &Document) {
        if Self::is_camera_action(&action) {
            self.note_camera_action(action);
            return;
        }
        if !Self::should_log(&action) {
            if Self::is_automatic_camera_side_effect(&action) {
                self.defer_baseline = true;
            }
            return;
        }
        let instruction = if matches!(action, Action::CommitExtrusion) {
            (doc.extrusions.len() > self.extrusion_count_before)
                .then(|| crate::script::instruction_for_new_extrusion(doc))
                .flatten()
        } else if matches!(action, Action::CommitLoft) {
            (doc.lofts.len() > self.loft_count_before)
                .then(|| crate::script::instruction_for_new_loft(doc))
                .flatten()
        } else if matches!(action, Action::CommitRevolve) {
            (doc.revolutions.len() > self.revolution_count_before)
                .then(|| crate::script::instruction_for_new_revolution(doc))
                .flatten()
        } else if matches!(action, Action::CommitSweep) {
            (doc.sweeps.len() > self.sweep_count_before)
                .then(|| crate::script::instruction_for_new_sweep(doc))
                .flatten()
        } else if matches!(action, Action::CommitEdgeTreatments { .. }) {
            // A chamfer/fillet commit records the new operation as one script call carrying
            // every treated edge (#531/#672); emit it here and yield nothing below.
            if doc.edge_treatment_ops.len() > self.edge_treatment_op_count_before {
                for instr in crate::script::instructions_for_new_edge_treatment_op(doc) {
                    self.emit_in(instr, Some(doc));
                }
            }
            None
        } else {
            instruction_from_action(&action, doc)
        };
        if let Some(instruction) = instruction {
            self.emit_in(instruction, Some(doc));
        }
        if Self::can_add_snap_constraint(&action) {
            for (key, constraint) in doc.constraints.iter() {
                if self.constraints_before.contains(&key) {
                    continue;
                }
                if let Some(extra) = instructions_for_snap_constraint(&constraint.kind) {
                    for instruction in extra {
                        self.emit_in(instruction, Some(doc));
                    }
                }
            }
        }
        if Self::is_automatic_camera_side_effect(&action) {
            self.defer_baseline = true;
        }
    }

    pub fn on_transition_complete(&mut self, cam: &Camera) {
        if self.defer_baseline {
            self.clear_pending();
            self.defer_baseline = false;
            let _ = cam;
        }
    }

    pub fn note_orbit(&mut self, delta: Vec2) {
        if delta.length_sq() < EPS {
            return;
        }
        self.pending_orbit += delta;
        self.pending_discrete = None;
    }

    pub fn note_pan(&mut self, delta: Vec2) {
        if delta.length_sq() < EPS {
            return;
        }
        self.pending_pan += delta;
        self.pending_discrete = None;
    }

    pub fn note_zoom(&mut self, scroll: f32) {
        if scroll.abs() < EPS {
            return;
        }
        self.pending_zoom += scroll;
        self.pending_discrete = None;
    }

    pub fn note_view_instruction(&mut self, instruction: Instruction) {
        self.clear_pending();
        self.pending_discrete = Some(instruction);
    }

    fn note_camera_action(&mut self, action: Action) {
        match action {
            Action::OrbitCamera { delta } => self.note_orbit(Vec2::new(delta.0, delta.1)),
            Action::PanCamera { delta, .. } => self.note_pan(Vec2::new(delta.0, delta.1)),
            Action::ZoomCamera { scroll, .. } => self.note_zoom(scroll),
            Action::SetStandardView(view) => {
                self.note_view_instruction(Instruction::View(view));
            }
            Action::SetViewEdge(edge) => {
                self.note_view_instruction(Instruction::ViewEdge(edge));
            }
            Action::SetViewCorner(corner) => {
                self.note_view_instruction(Instruction::ViewCorner(corner));
            }
            Action::ViewHome => self.note_view_instruction(Instruction::ViewHome),
            Action::SetProjectionMode(mode) => {
                self.note_view_instruction(Instruction::ProjectionMode(mode));
            }
            Action::ToggleProjectionMode => {
                self.note_view_instruction(Instruction::ToggleProjectionMode);
            }
            Action::SetShadingMode(mode) => {
                self.note_view_instruction(Instruction::ShadingMode(mode));
            }
            _ => {}
        }
    }

    fn flush_camera(&mut self, cam: &Camera) {
        let has_delta = self.pending_orbit.length_sq() > EPS
            || self.pending_pan.length_sq() > EPS
            || self.pending_zoom.abs() > EPS;

        if !has_delta {
            if let Some(instruction) = self.pending_discrete.take() {
                self.emit(instruction);
            }
        } else {
            self.pending_discrete = None;
            if self.pending_orbit.length_sq() > EPS {
                self.emit(Instruction::Orbit {
                    dx: self.pending_orbit.x,
                    dy: self.pending_orbit.y,
                });
            }
            if self.pending_pan.length_sq() > EPS {
                self.emit(Instruction::Pan {
                    dx: self.pending_pan.x,
                    dy: self.pending_pan.y,
                });
            }
            if self.pending_zoom.abs() > EPS {
                self.emit(Instruction::Zoom {
                    scroll: self.pending_zoom,
                });
            }
        }

        self.clear_pending();
        let _ = cam;
    }

    fn clear_pending(&mut self) {
        self.pending_orbit = Vec2::ZERO;
        self.pending_pan = Vec2::ZERO;
        self.pending_zoom = 0.0;
        self.pending_discrete = None;
    }

    /// Render and record one instruction. `doc` is the document it was recorded against, so
    /// elements are named by ordinal (#1070); the camera flush passes `None`, which is safe
    /// because a camera instruction names no elements.
    fn emit_in(&mut self, instruction: Instruction, doc: Option<&Document>) {
        let line = instruction.as_lua_in(doc);
        if self.print_stdout {
            println!("{line}");
        }
        self.history.push(line);
    }

    fn emit(&mut self, instruction: Instruction) {
        self.emit_in(instruction, None);
    }
}

/// Current UTC time as `YYYYMMDD-HHMMSS` (Howard Hinnant's civil-from-days algorithm), used
/// for session-export filenames and headers without pulling in a date/time dependency.
pub fn utc_timestamp() -> String {
    // Through `crate::time` (#1048): a raw `std::time::SystemTime::now()` here took the whole
    // web app down the moment Export Session Commands asked for a filename timestamp.
    use crate::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (hour, min, sec) = (tod / 3600, (tod % 3600) / 60, tod % 60);

    // Days since 1970-01-01 -> civil (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}{month:02}{day:02}-{hour:02}{min:02}{sec:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    #[test]
    fn accumulates_orbit_until_non_camera_action() {
        let mut log = CommandLog::new_recording(false);
        let cam = Camera::default();
        log.note_orbit(Vec2::new(10.0, 0.0));
        log.note_orbit(Vec2::new(-4.0, 5.0));
        log.before_apply(&Action::SetTool(crate::actions::Tool::Select), &Document::default(), &cam);
        assert_eq!(log.pending_orbit, Vec2::ZERO);
    }

    #[test]
    fn discrete_view_survives_until_flush_without_drag() {
        let mut log = CommandLog::new_recording(false);
        let cam = Camera::default();
        log.note_view_instruction(Instruction::View(crate::camera::StandardView::Front));
        log.before_apply(&Action::SetTool(crate::actions::Tool::Rectangle), &Document::default(), &cam);
        assert!(log.pending_discrete.is_none());
    }

    /// #1070: an exported session names an element by its **ordinal** among the live ones,
    /// not by its arena slot. The two agree until something is deleted — delete image 0 and
    /// the surviving image is ordinal 0 while its slot is still 1, so an export naming the
    /// slot replays to nothing.
    #[test]
    fn an_exported_session_names_an_element_by_its_ordinal_not_its_slot() {
        use crate::hierarchy::SceneElement;
        let cam = Camera::default();
        let mut doc = Document::default();
        let image = || crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: doc.construction_planes.keys().next().expect("the ground plane"),
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 10.0,
            height_mm: 10.0,
            opacity: crate::model::DEFAULT_TRACING_IMAGE_OPACITY,
            name: None,
            calibration: None,
        };
        let gone = doc.tracing_images.insert(image());
        let kept = doc.tracing_images.insert(image());
        doc.tracing_images.remove(gone);
        assert_eq!(kept.index(), 1, "the survivor kept slot 1");
        assert_eq!(
            doc.tracing_images.keys().position(|k| k == kept),
            Some(0),
            "and is the only live image, so ordinal 0"
        );

        let mut log = CommandLog::new_recording(false);
        let action = Action::SetElementVisible {
            element: SceneElement::Image(kept),
            visible: false,
        };
        log.before_apply(&action, &doc, &cam);
        log.after_apply(action, &doc);
        let script = log.session_lua_script("20260101-000000");
        assert!(
            script.contains("index = 0"),
            "the export should name the ordinal a replay resolves, got:\n{script}"
        );
        assert!(
            !script.contains("index = 1"),
            "naming the slot replays to nothing, got:\n{script}"
        );
    }

    #[test]
    fn session_script_contains_recorded_instructions_with_header() {
        let mut log = CommandLog::new_recording(false);
        log.emit(Instruction::New);
        log.emit(Instruction::CreateRect {
            width_expr: None, height_expr: None,
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 50.0,
        });
        let script = log.session_lua_script("20260630-000000");
        assert!(script.starts_with("-- BearCAD session commands"));
        assert!(script.contains("-- Exported 20260630-000000 UTC"));
        assert!(script.contains("bearcad.new()"));
        assert!(script.contains("bearcad.rect"));
        assert!(!log.is_empty());
    }

    #[test]
    fn utc_timestamp_has_expected_shape() {
        let ts = utc_timestamp();
        assert_eq!(ts.len(), 15, "timestamp = {ts}");
        assert_eq!(&ts[8..9], "-");
        assert!(ts[..8].chars().all(|c| c.is_ascii_digit()));
        assert!(ts[9..].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn drag_after_view_clears_discrete_view() {
        let mut log = CommandLog::new_recording(false);
        log.note_view_instruction(Instruction::View(crate::camera::StandardView::Front));
        log.note_orbit(Vec2::new(1.0, 2.0));
        assert!(log.pending_discrete.is_none());
        assert!(log.pending_orbit.length_sq() > 0.0);
    }
}