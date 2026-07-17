//! BearCAD — early prototype GUI.
//!
//! Rectangle tool: click to fix first corner, move mouse for second, with live
//! dimension inputs on the sides. Type to constrain a side, Tab to cycle,
//! Enter to commit. Right-drag orbit, wheel zoom. Save/Open .bearcad. (prototype)
//!
//! Fully scriptable via Lua files (SPEC §8):
//!   bearcad --script demo.lua
//!   bearcad --exit
//!   bearcad drawing.bearcad --exit
//!   bearcad demo.lua --exit

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The web build compiles the whole native codebase with scripting/CLI/SQLite gated out,
// which strands many natively-used helpers; keep the strict lints for native builds only.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports, unused_variables))]

mod actions;
mod projection;
mod style_swatches;
mod app_icon;
mod camera;
mod cli_install;
mod command_log;
mod command_palette;
mod constraints;
mod constraint_viewport;
mod geometric_constraints;
mod context;
mod construction;
mod dimensions;
mod drawing;
mod document_health;
mod element_picker;
mod document_lifecycle;
mod expression_input;
mod extrude;
mod face;
mod fps;
mod gpu_view_cube;
mod gpu_viewport;
mod hierarchy;
mod icons;
mod kernel;
mod names;
mod parameters;
mod polygon;
mod polygon_boolean;

mod model;
mod menu_command;
#[cfg(not(target_arch = "wasm32"))]
mod native_menu;
#[cfg(target_arch = "wasm32")]
mod web_menu;
#[cfg(target_arch = "wasm32")]
mod web_lua;
#[cfg(not(target_arch = "wasm32"))]
mod lua_script;
#[cfg(any(test, not(target_arch = "wasm32")))]
mod release_artifacts;
#[cfg(not(target_arch = "wasm32"))]
mod updater;
mod script;
// The JSON command dispatcher (todoer #179) is the web build's scripting hook: on wasm it
// backs `web_lua`'s bearcad_call dispatch; on native it's exercised by its own tests.
#[cfg(any(test, target_arch = "wasm32"))]
mod script_json;
mod selection;
mod shortcuts;
mod sketch_solver;
mod snapping;
mod step;
mod stl;
mod storage;
// #282a: consumed by the SketchText element (#282b) and the extrude/context work that follows.
#[allow(dead_code)]
mod text;
mod theme;
mod value;
mod vertex_drag;
mod view_cube;

use actions::{
    angle_gizmo_constraint_for_edit, chained_curve_handles, constraint_is_angle,
    constraint_is_circle_diameter, Action, AppState, CreatingCircle, CreatingConstructionPlane,
    CreatingEdgeTreatment, CreatingExtrusion, CreatingLine, CreatingRect, CreatingVertexTreatment,
    DimEditTarget, DimLabelTarget, Pane, RectAxis, SketchSession, Tool,
    DEFAULT_VERTEX_TREATMENT_AMOUNT,
};
use model::VertexTreatmentKind;
use constraint_viewport::{
    build_constraint_icon_hits, draw_constraint_connectors, draw_constraint_icons,
    pointer_over_constraint_icon, viewport_constraints_for_selection,
};
use constraints::{
    constraint_evaluated_length, constraint_segment_endpoints, distance_target_from_pick,
    distance_target_segment_endpoints,
};
use std::collections::HashSet;
use command_palette::{commands_for_state, filter_commands, show_palette, PaletteOutcome};
use hierarchy::SceneElement;
use selection::additive_click_modifiers;
use construction::{
    angle_from_axis_plane_hit, axis_angle_handle, axis_gizmo_hit, axis_normal,
    axis_offset_handle, draw_axis_plane_gizmo, draw_circle_face_highlight, draw_offset_gizmo,
    draw_polygon_face_highlight, draw_quad_face_highlight,
    nearest_sketch_line_in_sketch, nearest_sketch_point_in_sketch, offset_from_normal_drag,
    offset_gizmo_hit, offset_handle,
    parent_from_pick_target, plane_corners, point_world_position, preview_plane_edit_dependents,
    resolve_pick_target, scene_element_from_pick, AxisGizmoDrag,
    AxisGizmoHit, PlaneDim, PlaneReference, AXIS_GIZMO_HANDLE_HIT_RADIUS_PX, PLANE_DISPLAY_HALF,
};
use document_health::{health_tint_color, DocumentHealth, HealthStatus};
use document_lifecycle::{circle_alive, constraint_alive, line_alive};
use constraints::{
    angle_constraint_display, angle_dimension_hover_sign, angle_rad_from_sketch_hit,
    constraint_evaluated_angle, default_angle_expression, AngleConstraintDisplay,
};
use dimensions::{
    angle_gizmo_handle_hit, angle_gizmo_handle_world, arc_dimension_world_geom,
    circle_diameter_dimension_world_geom, circle_diameter_label_outward_px,
    draw_angle_constraint_annotation, draw_linear_dimension, effective_circle_diameter_label_offset,
    effective_arc_dim_offset, effective_dim_offset, planar_dimension_label_layout, PlanarLabelView,
    linear_dimension_world_geom,
    outward_perpendicular_uv, pixels_to_world_distance, preferred_outward_uv,
    project_arc_dimension_geom, project_linear_dimension_geom, uv_dir_to_world,
    EXTENSION_OVERSHOOT, LABEL_FONT_SIZE, LABEL_OUTSET,
};
use face::{
    circle_world_diameter_endpoints, circle_world_perimeter,
    line_world_polyline, local_to_world, pick_sketch_face, sketch_frame,
    sketch_geometry_frame, sketch_label, world_to_local,
};
use model::SketchId;
use model::{
    Circle, ConstraintKind, ConstraintPoint, DistanceTarget, FaceId, Line,
};
use eframe::egui;
use menu_command::MenuCommand;
#[cfg(not(target_arch = "wasm32"))]
use native_menu::NativeMenu;
use glam::Vec3;
use model::ConstructionPlane;
use script::{ScriptRunner, SyntheticInput};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use expression_input::{
    expression_autocomplete_handle_keys, expression_autocomplete_show_dropdown,
    length_expression_field_errors, show_expression_error_tooltips_above, INVALID_BG,
    INVALID_BORDER, INVALID_TEXT,
};
use value::{computed_length_in_doc, shows_computed_length_in_doc};

/// macOS maximize must run after eframe shows the window (post-first-paint).
fn uses_deferred_launch_maximize() -> bool {
    cfg!(target_os = "macos")
}

/// Frames to wait after startup before sending maximize on macOS.
const MACOS_LAUNCH_MAXIMIZE_DELAY_FRAMES: u8 = 2;

fn initial_launch_maximize_frames() -> u8 {
    if uses_deferred_launch_maximize() {
        MACOS_LAUNCH_MAXIMIZE_DELAY_FRAMES
    } else {
        0
    }
}

fn tick_launch_maximize(frames_remaining: &mut u8, ctx: &egui::Context) {
    if *frames_remaining == 0 {
        return;
    }
    *frames_remaining -= 1;
    if *frames_remaining == 0 {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_options() -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 640.0])
        .with_title("BearCAD")
        .with_icon(app_icon::load_for_viewport());
    if !uses_deferred_launch_maximize() {
        viewport = viewport.with_maximized(true);
    }

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        let mut options = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        };
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_default_menu(false);
        }));
        options
    }
    #[cfg(not(target_os = "macos"))]
    {
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    match script::parse_cli(std::env::args()) {
        script::CliOutcome::Help => {
            script::print_usage();
            return Ok(());
        }
        script::CliOutcome::InstallCli => {
            run_cli_action(cli_install::run_install());
            return Ok(());
        }
        script::CliOutcome::UninstallCli => {
            run_cli_action(cli_install::run_uninstall());
            return Ok(());
        }
        script::CliOutcome::Run(script_opts) => run_app(script_opts),
    }
}

/// Web entry point: eframe renders into the `bearcad_canvas` element of the hosting page
/// (web/index.html). No CLI, no scripts — just the interactive app.
#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("browser document");
        let canvas = document
            .get_element_by_id("bearcad_canvas")
            .expect("page must have a canvas with id bearcad_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("bearcad_canvas must be a canvas element");
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    theme::apply(&cc.egui_ctx);
                    Ok(Box::new(App::new(
                        cc,
                        None,
                        None,
                        false,
                        false,
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    )))
                }),
            )
            .await
            .expect("failed to start BearCAD web app");
    });
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Print the result of a CLI install/uninstall action and exit non-zero on failure.
#[cfg(not(target_arch = "wasm32"))]
fn run_cli_action(result: Result<String, String>) {
    match result {
        Ok(msg) => println!("{msg}"),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_app(script_opts: script::ScriptOptions) -> eframe::Result<()> {
    if let Some(secs) = script_opts.timeout_secs {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            eprintln!("error: bearcad did not exit within {secs}s, forcing exit");
            std::process::exit(1);
        });
    }
    let options = native_options();

    let script = if script_opts.repl {
        // Interactive Lua REPL on stdin against the live app; mutually exclusive with a
        // script file (the REPL *is* the script source).
        if script_opts.script_path.is_some() {
            return Err(eframe::Error::AppCreation(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--repl and --script are mutually exclusive",
            ))));
        }
        Some(ScriptRunner::repl()).transpose()
    } else {
        script_opts
            .script_path
            .as_ref()
            .map(|p| ScriptRunner::from_file(Path::new(p)))
            .transpose()
    }
    .map_err(|e| eframe::Error::AppCreation(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        e.to_string(),
    ))))?;

    let script_failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let script_failed_for_app = script_failed.clone();
    let result = eframe::run_native(
        "BearCAD",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let native_menu = NativeMenu::install(cc).map_err(|e| {
                eframe::Error::AppCreation(Box::new(std::io::Error::other(
                    e.to_string(),
                )))
            })?;
            Ok(Box::new(App::new(
                cc,
                script,
                script_opts.document_path,
                script_opts.exit_on_complete,
                script_opts.show_commands,
                native_menu,
                script_failed_for_app,
            )) as Box<dyn eframe::App>)
        }),
    );
    // A script that errored under `--exit` closed the window cleanly (#125) — that must
    // still fail the process, e.g. so CI catches a broken script instead of a green run.
    if script_failed.load(std::sync::atomic::Ordering::SeqCst) {
        std::process::exit(1);
    }
    result
}

#[cfg(test)]
mod cli_tests {
    use super::script;

    #[test]
    fn help_outcome_is_distinct_from_default_run() {
        assert_ne!(
            script::parse_cli(["bearcad", "--help"]),
            script::CliOutcome::Run(script::ScriptOptions::default())
        );
    }

    #[test]
    fn install_cli_subcommands_parse() {
        assert_eq!(
            script::parse_cli(["bearcad", "install-cli"]),
            script::CliOutcome::InstallCli
        );
        assert_eq!(
            script::parse_cli(["bearcad", "uninstall-cli"]),
            script::CliOutcome::UninstallCli
        );
    }

    #[test]
    fn install_cli_does_not_shadow_a_document_named_like_it() {
        // A real path/script argument still runs the app; only the bare subcommand installs.
        assert!(matches!(
            script::parse_cli(["bearcad", "drawing.bearcad", "--exit"]),
            script::CliOutcome::Run(_)
        ));
    }
}

const DIM_LABEL_DRAG_THRESHOLD_PX: f32 = 4.0;

/// Minimum horizontal drag (sketch mm) for the Text tool to treat a press-release as a
/// box-drag that sets a wrap width instead of a plain click (#282/#407).
const TEXT_DRAG_MIN_WIDTH_MM: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct DimLabelDrag {
    target: DimLabelTarget,
    outward: egui::Vec2,
    start_offset: f32,
    anchor_screen: egui::Pos2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AngleGizmoDrag {
    constraint_id: DimLabelTarget,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ExtrudeGizmoDrag {
    start_screen: egui::Pos2,
    start_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VertexTreatmentGizmoDrag {
    start_screen: egui::Pos2,
    start_amount: f32,
}

/// The 3D analogue of [`VertexTreatmentGizmoDrag`] (#77): same click-to-grab, follow-the-cursor
/// push/pull gizmo, just anchored on an extrusion's analytic edge instead of a sketch vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EdgeTreatmentGizmoDrag {
    start_screen: egui::Pos2,
    start_amount: f32,
}

/// An in-flight drag of a drawing dimension label (#294): the label rides the pointer's
/// perpendicular offset from its edge, written back as a `dimension_offsets` override.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawingDimLabelDrag {
    drawing: usize,
    view: usize,
    key: ([i32; 3], [i32; 3]),
    /// A circle Ø-label drag (#397): `key.0` is the circle's quantized centre and the offset
    /// writes through `SetDrawingCircleDimOffset` instead of the edge-keyed action.
    circle: bool,
    start_offset: f32,
    start_pointer: egui::Pos2,
    /// Outward unit direction in screen space (pixels), for projecting the drag delta.
    outward_screen: egui::Vec2,
    /// Projected-mm per screen pixel (1 / scale), to convert the pixel delta to a mm offset.
    mm_per_px: f32,
}

/// A drag on one of the Move tool's translation arrows (#215): which axis (0=X, 1=Y, 2=Z), and
/// the translation + cursor position when the grab started. Follows the cursor along that world
/// axis and writes the result through the `move_{x,y,z}` gizmo setter.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MoveGizmoDrag {
    axis: usize,
    start_translation: f32,
    start_screen: egui::Pos2,
}

/// A drag on the Move tool's rotation ring (#216): the cursor's angle around the ring centre
/// and the move angle when the grab started; the ring turns with the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MoveRotationDrag {
    start_cursor_angle: f32,
    start_angle_deg: f32,
}

/// A drag rotating a selected sketch text with the Move tool's rotation ring (#286). The text's
/// `rotation` follows the cursor's angle around the text origin; the context pane's Rotation°
/// field reads the model each frame, so it stays in sync automatically.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TextRotationDrag {
    index: usize,
    start_cursor_angle: f32,
    /// Model rotation (radians) when the grab started.
    start_rotation: f32,
}

/// A drag on the Move tool's in-sketch selection gizmo (#306): the centred handle moves the
/// selection freely; the horizontal/vertical arrows constrain the move to that sketch axis.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SketchMoveDrag {
    /// `None` = free drag; `Some(0)` = horizontal (sketch u), `Some(1)` = vertical (sketch v).
    axis: Option<usize>,
    /// Grab point on the sketch plane (also the selection-drag session's anchor).
    anchor_uv: (f32, f32),
}

struct VertexDrag {
    point: ConstraintPoint,
}

/// A bezier control-point handle being dragged: `near_start` selects `line.bezier`'s handle
/// nearest `(x0,y0)` vs. nearest `(x1,y1)`.
/// An in-progress drag of a wrapped text box's width handle (#409).
struct TextWidthDrag {
    text: usize,
    /// True when the left edge handle is held (the origin shifts); false for the right.
    left: bool,
}

struct BezierHandleDrag {
    line: usize,
    near_start: bool,
}

/// What the viewport's right-click context menu should offer (#54/#75).
#[derive(Clone)]
enum ViewportContextMenu {
    ConvertVertexToBezier(ConstraintPoint),
    StraightenLine(usize),
    /// Right-clicked directly on a bezier handle: same underlying action as `StraightenLine`
    /// (there's no independent per-handle state to remove — see `selected_bezier_handle`), but
    /// worded as "delete" since that's what the user clicked on (#75).
    DeleteBezierHandle(usize),
}

#[derive(Clone, Debug, PartialEq)]
struct CommittedDimLayout {
    target: DimLabelTarget,
    geom: dimensions::LinearDimensionGeom,
    world_geom: dimensions::LinearDimensionWorldGeom,
    arc_geom: Option<dimensions::ArcDimensionGeom>,
    angle_display: Option<AngleConstraintDisplay>,
    angle_radius_world: f32,
    label: String,
    label_rect: egui::Rect,
    outward: egui::Vec2,
    offset: f32,
}

struct App {
    state: AppState,
    synthetic: SyntheticInput,
    script: Option<ScriptRunner>,
    exit_on_script_complete: bool,
    exit_after_startup: bool,
    exit_after_startup_sent: bool,
    show_commands: bool,
    last_viewport: Option<egui::Rect>,
    /// Whether we've asked the OS to lock+hide the cursor for FPS mode (#91), so
    /// enter/exit sends the viewport commands exactly once per change.
    fps_cursor_grabbed: bool,
    /// Elements-pane row under the cursor this frame (#161): the viewport highlights it so
    /// hovering a row shows what it is in 3D. Set while panes render, consumed by the
    /// viewport pass in the same frame.
    pane_hovered_element: Option<SceneElement>,
    /// Frames left to swallow mouse-look motion after entering FPS mode (#135): locking and
    /// pinning the cursor to the crosshair can report the warp (from wherever the OS cursor
    /// was, to the viewport center) as one huge pointer-motion delta, which would spin the
    /// view on entry. Entry must not move the view, so the first frames' motion is dropped.
    fps_look_warmup: u8,
    #[cfg(not(target_arch = "wasm32"))]
    native_menu: NativeMenu,
    /// Results of async browser file dialogs (open/import picks), drained each frame.
    #[cfg(target_arch = "wasm32")]
    web_io: WebIoQueue,
    /// Document JSON dialog (File ▸ Document JSON…): the whole document as pasteable
    /// JSON text for bug reports, and a paste-target to load one back. `None` = closed.
    json_dialog: Option<String>,
    dim_label_drag: Option<DimLabelDrag>,
    /// In-flight drag of a drawing dimension label (#294): `(drawing, view, edge key, start
    /// offset mm, drag-start pointer)`.
    drawing_dim_label_drag: Option<DrawingDimLabelDrag>,
    /// The Aligned-view tool's chosen **base** view (#296/#365): the view index within the open
    /// drawing whose projection a new aligned view lines up with. Seeded from a selected projection
    /// when the tool is entered, or picked from the "Base view" element picker / by clicking a
    /// projection.
    drawing_align_parent: Option<usize>,
    /// The tool active on the previous frame (#365), so entering the Aligned-view tool can seed its
    /// base from the current selection exactly once.
    prev_tool: Tool,
    /// Text tool press anchor in sketch-local coords (#282): set on press; on release a drag
    /// beyond a threshold creates a wrapped textbox of that width, a click a growing one.
    text_tool_anchor: Option<(f32, f32)>,
    /// Text tool press anchor on the drawing page as a page fraction (#312), for the same
    /// click-vs-drag placement of a page annotation.
    drawing_text_anchor: Option<(f32, f32)>,
    angle_gizmo_drag: Option<AngleGizmoDrag>,
    vertex_drag: Option<VertexDrag>,
    bezier_handle_drag: Option<BezierHandleDrag>,
    text_width_drag: Option<TextWidthDrag>,
    /// Bezier handle selected by a plain click (persists past the click, unlike
    /// `bezier_handle_drag`), so Delete/Backspace can remove it (#75). `(line, near_start)`.
    selected_bezier_handle: Option<(usize, bool)>,
    /// What the viewport's right-click context menu should offer, resolved from whatever was
    /// under the cursor when it was opened (remembered across frames since the menu content
    /// closure may run on a later frame than the click itself).
    viewport_context_menu: Option<ViewportContextMenu>,
    extrude_gizmo_drag: Option<ExtrudeGizmoDrag>,
    /// Object the extrude gizmo is currently snapped to (applied on release).
    pending_extrude_target: Option<model::ExtrudeTarget>,
    vertex_treatment_gizmo_drag: Option<VertexTreatmentGizmoDrag>,
    /// Push/pull gizmo drag state for the 3D edge chamfer/fillet tool (#77); parallel to
    /// `vertex_treatment_gizmo_drag`.
    edge_treatment_gizmo_drag: Option<EdgeTreatmentGizmoDrag>,
    /// In-flight revolve angle-handle drag: (start screen pos, start angle degrees).
    revolve_gizmo_drag: Option<(egui::Pos2, f32)>,
    /// In-flight Move translation-arrow drag (#215).
    move_gizmo_drag: Option<MoveGizmoDrag>,
    /// In-flight Move rotation-ring drag (#216).
    move_rotation_drag: Option<MoveRotationDrag>,
    /// In-flight sketch-text rotation drag on the Move tool's ring (#286).
    text_rotation_drag: Option<TextRotationDrag>,
    /// In-flight in-sketch selection move on the Move tool's gizmo (#306).
    sketch_move_drag: Option<SketchMoveDrag>,
    launch_maximize_frames_remaining: u8,
    gpu_viewport: bool,
    gpu_view_cube: bool,
    /// Persistent physics state for the Elements pane's force-directed Graph view (#94).
    /// Ephemeral view state (never persisted), like `AppState::hierarchy_view_mode`.
    graph_layout: hierarchy::GraphLayout,
    /// Collapsed component rows in the Elements pane (#423); UI-only state.
    collapsed_components: std::collections::HashSet<usize>,
    /// A selected calibration reference point (#424): `(image, point index)`; Delete
    /// removes it so a click can re-place it.
    selected_calibration_point: Option<(usize, usize)>,
    /// An in-progress drag of a calibration reference point (#424).
    calibration_point_drag: Option<(usize, usize)>,
    /// Auto-update state (#427): background release check + staged update progress.
    #[cfg(not(target_arch = "wasm32"))]
    update_state: updater::SharedUpdateState,
    /// Whether the browser fallback already opened for a failed staged update (#427).
    #[cfg(not(target_arch = "wasm32"))]
    update_fallback_opened: bool,
    /// Elements-pane type filter (#275) and whether its toggle panel is expanded. Ephemeral UI
    /// state; reset to the workbench default when the Model/Drawing workbench changes.
    element_filter: hierarchy::ElementFilter,
    element_filter_expanded: bool,
    element_filter_drawing_workbench: bool,
    /// Drawing-editor pan/zoom (#273): a scale about the pane centre and a pixel pan offset.
    /// Ephemeral view state; reset (fit) by the Zoom tool / `Z`.
    drawing_zoom: f32,
    drawing_pan: egui::Vec2,
    /// Whether the Parameters pane was visible before entering the Drawing workbench (#398):
    /// it hides by default there (still re-showable from the View menu) and this restores it
    /// on the way back to the model.
    params_visible_before_drawing: bool,
    /// A drawing popped out into its own OS window (#276), so it can sit beside the 3D view.
    drawing_window: Option<usize>,
    /// Set just before closing on an uncaught script error with `--exit` (#125), so
    /// `run_app` can translate it into a non-zero process exit code after the eframe
    /// event loop returns — a script failure must fail the process, not just the UI.
    script_failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// One completed async browser file-dialog interaction (web build): picked file bytes to
/// apply, or a status line from a finished save/export. Queued from `spawn_local` futures
/// and drained on the next frame.
#[cfg(target_arch = "wasm32")]
enum WebIoEvent {
    OpenedDocument { name: String, bytes: Vec<u8> },
    ImportStl { name: String, bytes: Vec<u8> },
    ImportStep { name: String, bytes: Vec<u8> },
    ImportImage { name: String, bytes: Vec<u8>, plane: Option<usize> },
    RunScript { bytes: Vec<u8> },
    Status(String),
}

#[cfg(target_arch = "wasm32")]
type WebIoQueue = std::rc::Rc<std::cell::RefCell<Vec<WebIoEvent>>>;

impl App {
    /// The status bar's update badge (#427): appears only when a newer release exists;
    /// clicking stages the update (Windows/Linux) or auto-downloads the installer in the
    /// browser (macOS), falling back to the releases page on failure.
    #[cfg(not(target_arch = "wasm32"))]
    fn show_update_badge(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let snapshot = self.update_state.lock().map(|s| s.clone()).ok();
        let Some(snapshot) = snapshot else { return };
        match (&snapshot.outcome, snapshot.in_progress, &snapshot.available) {
            (Some(Ok(updater::UpdateOutcome::StagedRestartToFinish { launch })), _, _) => {
                // The new version is in place — one click relaunches into it (#427).
                let restart = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("⟳ Restart BearCAD")
                                .color(egui::Color32::BLACK)
                                .size(12.0),
                        )
                        .fill(egui::Color32::from_rgb(90, 200, 120))
                        .corner_radius(4.0),
                    )
                    .on_hover_text("The update is installed — restart to run the new version");
                if restart.clicked() {
                    if let Err(e) = updater::restart_into(launch) {
                        self.state.status = format!("Restart failed: {e}");
                    }
                }
            }
            (Some(Ok(updater::UpdateOutcome::OpenedInBrowser)), _, _) => {
                ui.colored_label(
                    egui::Color32::from_rgb(90, 200, 120),
                    "Update downloading in your browser",
                );
            }
            (Some(Err(_)), _, Some(version)) => {
                // The staged update failed; the browser fallback already opened. Keep the
                // badge so the user can retry.
                if self.update_badge(ui, version) {
                    updater::spawn_update(self.update_state.clone(), ctx.clone());
                }
            }
            (None, true, _) => {
                ui.add(egui::Spinner::new().size(12.0));
                ui.colored_label(egui::Color32::from_gray(170), "Downloading update…");
            }
            (None, false, Some(version)) => {
                if self.update_badge(ui, version) {
                    updater::spawn_update(self.update_state.clone(), ctx.clone());
                }
            }
            _ => {}
        }
        // A dev/non-bundle run can't stage in place: auto-download in the browser once.
        if matches!(
            &snapshot.outcome,
            Some(Ok(updater::UpdateOutcome::OpenedInBrowser))
        ) && !self.update_fallback_opened
        {
            self.update_fallback_opened = true;
            ctx.open_url(egui::OpenUrl::new_tab(updater::platform_artifact_url()));
        }
        // A failed staged update falls back to the browser download once.
        if let Some(Err(err)) = &snapshot.outcome {
            if !self.update_fallback_opened {
                self.update_fallback_opened = true;
                self.state.status = format!("Update failed ({err}) — opening the releases page");
                ctx.open_url(egui::OpenUrl::new_tab(updater::releases_page_url()));
            }
        }
    }

    /// The bright badge button itself; returns true when clicked.
    #[cfg(not(target_arch = "wasm32"))]
    fn update_badge(&self, ui: &mut egui::Ui, version: &str) -> bool {
        ui.add(
            egui::Button::new(
                egui::RichText::new(format!("⬆ Update to v{version}"))
                    .color(egui::Color32::BLACK)
                    .size(12.0),
            )
            .fill(egui::Color32::from_rgb(90, 200, 120))
            .corner_radius(4.0),
        )
        .on_hover_text("A new BearCAD release is available — click to update")
        .clicked()
    }

    fn new(
        cc: &eframe::CreationContext<'_>,
        script: Option<ScriptRunner>,
        document_path: Option<String>,
        exit_on_script_complete: bool,
        show_commands: bool,
        #[cfg(not(target_arch = "wasm32"))] native_menu: NativeMenu,
        script_failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let status = if script.as_ref().is_some_and(|r| r.is_repl()) {
            "Lua REPL — enter commands in the terminal".to_string()
        } else if script.is_some() {
            "Running script…".to_string()
        } else if cfg!(target_arch = "wasm32") {
            // The web kernel is a separately-loaded module that can fail independently of
            // the app; surface its state at boot so users (and bug reports) see it.
            kernel::selftest()
        } else {
            String::new()
        };
        let mut state = AppState {
            status,
            ..AppState::default()
        };
        if let Some(path) = document_path {
            match state.apply(Action::Open { path }) {
                actions::ActionResult::Err(message) => state.status = message,
                _ => {}
            }
        }
        // Always record interactively so the session can be exported as a Lua script (#43);
        // `show_commands` only controls whether each instruction is also echoed to stdout.
        if script.is_none() {
            state.command_log = Some(std::cell::RefCell::new(
                command_log::CommandLog::new_recording(show_commands),
            ));
        }
        // Let the REPL's stdin reader thread wake the event loop when input arrives while
        // the app is idle (no repaints scheduled).
        if let Some(runner) = &script {
            runner.install_repaint_context(cc.egui_ctx.clone());
        }
        let exit_after_startup = exit_on_script_complete && script.is_none();
        Self {
            state,
            synthetic: SyntheticInput::default(),
            script,
            exit_on_script_complete,
            exit_after_startup,
            exit_after_startup_sent: false,
            show_commands,
            last_viewport: None,
            fps_cursor_grabbed: false,
            pane_hovered_element: None,
            fps_look_warmup: 0,
            #[cfg(not(target_arch = "wasm32"))]
            native_menu,
            #[cfg(target_arch = "wasm32")]
            web_io: WebIoQueue::default(),
            json_dialog: None,
            dim_label_drag: None,
            drawing_dim_label_drag: None,
            drawing_align_parent: None,
            prev_tool: Tool::Select,
            text_tool_anchor: None,
            drawing_text_anchor: None,
            angle_gizmo_drag: None,
            extrude_gizmo_drag: None,
            pending_extrude_target: None,
            vertex_treatment_gizmo_drag: None,
            edge_treatment_gizmo_drag: None,
            revolve_gizmo_drag: None,
            move_gizmo_drag: None,
            move_rotation_drag: None,
            text_rotation_drag: None,
            sketch_move_drag: None,
            vertex_drag: None,
            bezier_handle_drag: None,
            text_width_drag: None,
            selected_bezier_handle: None,
            viewport_context_menu: None,
            launch_maximize_frames_remaining: initial_launch_maximize_frames(),
            gpu_viewport: gpu_viewport::install(cc),
            gpu_view_cube: gpu_view_cube::install(cc),
            graph_layout: hierarchy::GraphLayout::default(),
            collapsed_components: std::collections::HashSet::new(),
            selected_calibration_point: None,
            calibration_point_drag: None,
            #[cfg(not(target_arch = "wasm32"))]
            update_state: {
                let state = updater::SharedUpdateState::default();
                updater::spawn_check(state.clone());
                state
            },
            #[cfg(not(target_arch = "wasm32"))]
            update_fallback_opened: false,
            element_filter: hierarchy::ElementFilter::default(),
            element_filter_expanded: false,
            element_filter_drawing_workbench: false,
            drawing_zoom: 1.0,
            drawing_pan: egui::Vec2::ZERO,
            params_visible_before_drawing: false,
            drawing_window: None,
            script_failed,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) {
        let start = rfd::FileDialog::new()
            .add_filter("BearCAD document", &["bearcad"])
            .set_file_name("untitled.bearcad");
        if let Some(path) = start.save_file() {
            let path = path.to_string_lossy().to_string();
            self.state.apply(Action::Save {
                path: Some(path),
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save(&mut self) {
        match self.state.apply(Action::Save { path: None }) {
            actions::ActionResult::NeedsDialog => self.save_as(),
            _ => {}
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Export all bodies to an STL file chosen via a save dialog (File → Export STL…).
    fn export_stl_all(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("STL mesh", &["stl"])
            .set_file_name("model.stl")
            .save_file();
        if let Some(path) = picked {
            self.state.apply(Action::ExportStl {
                path: path.to_string_lossy().to_string(),
                body: None,
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Import an STL file as a new body chosen via an open dialog (File → Import STL…).
    fn import_stl(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("STL mesh", &["stl"])
            .pick_file();
        if let Some(path) = picked {
            self.state.apply(Action::ImportStl {
                path: path.to_string_lossy().to_string(),
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Import a PNG/JPEG as a tracing image via an open dialog (File → Import Image…, #169).
    fn import_image(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            self.state.apply(Action::ImportImage {
                path: path.to_string_lossy().to_string(),
                plane: None,
            });
        }
    }

    /// Import a PNG/JPEG onto a *specific* construction plane (the Elements pane's
    /// right-click "Import image on this plane…", #175).
    #[cfg(not(target_arch = "wasm32"))]
    fn import_image_on_plane(&mut self, plane: usize) {
        let picked = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            self.state.apply(Action::ImportImage {
                path: path.to_string_lossy().to_string(),
                plane: Some(plane),
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Import a STEP file as a new body chosen via an open dialog (File → Import STEP…).
    fn import_step(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("STEP model", &["step", "stp"])
            .pick_file();
        if let Some(path) = picked {
            self.state.apply(Action::ImportStep {
                path: path.to_string_lossy().to_string(),
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Export all bodies to a STEP file chosen via a save dialog (File → Export STEP…).
    fn export_step_all(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("STEP model", &["step", "stp"])
            .set_file_name("model.step")
            .save_file();
        if let Some(path) = picked {
            self.state.apply(Action::ExportStep {
                path: path.to_string_lossy().to_string(),
                body: None,
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Export a single body (by index) to an STL file chosen via a save dialog.
    fn export_stl_body(&mut self, body: usize) {
        let default_name = self
            .state
            .doc
            .bodies
            .get(body)
            .and_then(|b| b.name.clone())
            .unwrap_or_else(|| format!("body-{body}"));
        let picked = rfd::FileDialog::new()
            .add_filter("STL mesh", &["stl"])
            .set_file_name(format!("{default_name}.stl"))
            .save_file();
        if let Some(path) = picked {
            self.state.apply(Action::ExportStlBody {
                path: path.to_string_lossy().to_string(),
                body,
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Export a single body (by index) to a STEP file chosen via a save dialog.
    fn export_step_body(&mut self, body: usize) {
        let default_name = self
            .state
            .doc
            .bodies
            .get(body)
            .and_then(|b| b.name.clone())
            .unwrap_or_else(|| format!("body-{body}"));
        let picked = rfd::FileDialog::new()
            .add_filter("STEP model", &["step", "stp"])
            .set_file_name(format!("{default_name}.step"))
            .save_file();
        if let Some(path) = picked {
            self.state.apply(Action::ExportStepBody {
                path: path.to_string_lossy().to_string(),
                body,
            });
        }
    }

    /// Open the Document JSON dialog pre-filled with the current document, serialized with
    /// the same JSON codec the web build saves with (`storage::to_json_bytes`). The text is
    /// meant to travel through a bug report: copy it out to attach a document state, or
    /// paste a reported state in and load it.
    fn open_json_dialog(&mut self) {
        match crate::storage::to_json_bytes(&self.state.doc) {
            Ok(bytes) => {
                self.json_dialog = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            Err(e) => self.state.status = format!("Could not serialize document: {e}"),
        }
    }

    /// Per-frame UI for the Document JSON dialog (see [`Self::open_json_dialog`]).
    fn show_json_dialog(&mut self, ctx: &egui::Context) {
        let Some(text) = self.json_dialog.as_mut() else {
            return;
        };
        let mut open = true;
        let mut copy = false;
        let mut load = false;
        let mut refresh = false;
        egui::Window::new("Document JSON")
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    "The whole document as JSON. Copy it into a bug report — or paste a \
                     reported document here and load it.",
                );
                ui.horizontal(|ui| {
                    copy = ui.button("Copy to clipboard").clicked();
                    refresh = ui.button("Refresh from document").clicked();
                    load = ui.button("Load into document").clicked();
                });
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(20),
                    );
                });
            });
        if copy {
            ctx.copy_text(text.clone());
            self.state.status = "Document JSON copied to clipboard".to_string();
        }
        if load {
            let bytes = text.clone().into_bytes();
            self.state.open_document_bytes(&bytes, "pasted JSON");
        }
        if refresh {
            self.open_json_dialog();
        } else if !open {
            self.json_dialog = None;
        }
    }

    /// Export everything done this session as a timestamped, replayable Lua script, chosen
    /// via a save dialog (Help → Export Session Commands…, and the command palette). See #43.
    #[cfg(not(target_arch = "wasm32"))]
    fn export_session_commands(&mut self) {
        let timestamp = command_log::utc_timestamp();
        let script = match &self.state.command_log {
            Some(log) if !log.borrow().is_empty() => log.borrow().session_lua_script(&timestamp),
            _ => {
                self.state.status = "No session commands to export yet".to_string();
                return;
            }
        };
        let picked = rfd::FileDialog::new()
            .add_filter("Lua script", &["lua"])
            .set_file_name(format!("bearcad-session-{timestamp}.lua"))
            .save_file();
        if let Some(path) = picked {
            match std::fs::write(&path, script) {
                Ok(()) => {
                    self.state.status =
                        format!("Exported session commands to {}", path.display());
                }
                Err(e) => {
                    self.state.status = format!("Failed to export session commands: {e}");
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("BearCAD document", &["bearcad"])
            .pick_file();
        if let Some(path) = picked {
            let path = path.to_string_lossy().to_string();
            self.state.apply(Action::Open { path });
        }
    }

    /// Apply the results of finished async browser dialogs (web build).
    #[cfg(target_arch = "wasm32")]
    fn drain_web_io(&mut self, ctx: &egui::Context) {
        let events: Vec<WebIoEvent> = self.web_io.borrow_mut().drain(..).collect();
        for event in events {
            match event {
                WebIoEvent::OpenedDocument { name, bytes } => {
                    self.state.open_document_bytes(&bytes, &name);
                }
                WebIoEvent::ImportStl { name, bytes } => {
                    self.state.import_stl_bytes(&name, &bytes);
                }
                WebIoEvent::ImportStep { name, bytes } => {
                    self.state.import_step_bytes(&name, &bytes);
                }
                WebIoEvent::ImportImage { name, bytes, plane } => {
                    self.state.import_image_bytes(&name, bytes, plane);
                }
                WebIoEvent::RunScript { bytes } => {
                    self.run_web_script(ctx, &bytes);
                }
                WebIoEvent::Status(message) => self.state.status = message,
            }
        }
    }

    /// Run a picked `.lua` script's bytes through the Lua interpreter module against the live
    /// document, then report the outcome in the status line.
    #[cfg(target_arch = "wasm32")]
    fn run_web_script(&mut self, ctx: &egui::Context, bytes: &[u8]) {
        let src = match std::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                self.state.status = "Script isn't valid UTF-8 text".to_string();
                return;
            }
        };
        self.state.status = match web_lua::run_script(
            &mut self.state,
            &mut self.synthetic,
            self.last_viewport,
            ctx,
            src,
        ) {
            Ok(()) => "Script complete".to_string(),
            Err(err) => format!("Script error: {err}"),
        };
    }

    /// Browser open dialog → queue the picked file's bytes as `make_event`'s event.
    #[cfg(target_arch = "wasm32")]
    fn web_pick_file(
        &self,
        filter_name: &'static str,
        extensions: &'static [&'static str],
        make_event: impl Fn(String, Vec<u8>) -> WebIoEvent + 'static,
    ) {
        let queue = self.web_io.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter(filter_name, extensions)
                .pick_file()
                .await
            {
                let name = file.file_name();
                let bytes = file.read().await;
                queue.borrow_mut().push(make_event(name, bytes));
            }
        });
    }

    /// Browser save dialog → write `bytes` (the browser downloads the file).
    #[cfg(target_arch = "wasm32")]
    fn web_save_bytes(
        &self,
        filter_name: &'static str,
        extensions: &'static [&'static str],
        suggested_name: String,
        bytes: Vec<u8>,
        done_status: String,
    ) {
        let queue = self.web_io.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(handle) = rfd::AsyncFileDialog::new()
                .add_filter(filter_name, extensions)
                .set_file_name(&suggested_name)
                .save_file()
                .await
            {
                let message = match handle.write(&bytes).await {
                    Ok(()) => done_status,
                    Err(e) => format!("Save failed: {e}"),
                };
                queue.borrow_mut().push(WebIoEvent::Status(message));
            }
        });
    }

    /// Web File-menu implementations: same names the shared menu dispatch calls, backed by
    /// browser dialogs and the byte-level document/import/export helpers (no filesystem).
    #[cfg(target_arch = "wasm32")]
    fn open(&mut self) {
        self.web_pick_file("BearCAD document", &["bearcad", "json"], |name, bytes| {
            WebIoEvent::OpenedDocument { name, bytes }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn save(&mut self) {
        match crate::storage::to_json_bytes(&self.state.doc) {
            Ok(bytes) => self.web_save_bytes(
                "BearCAD document",
                &["bearcad"],
                "untitled.bearcad".to_string(),
                bytes,
                "Saved document".to_string(),
            ),
            Err(e) => self.state.status = format!("Save failed: {e}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn save_as(&mut self) {
        self.save();
    }

    #[cfg(target_arch = "wasm32")]
    fn import_stl(&mut self) {
        self.web_pick_file("STL mesh", &["stl"], |name, bytes| WebIoEvent::ImportStl {
            name,
            bytes,
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn import_step(&mut self) {
        self.web_pick_file("STEP model", &["step", "stp"], |name, bytes| {
            WebIoEvent::ImportStep { name, bytes }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn import_image(&mut self) {
        self.web_pick_file("Image", &["png", "jpg", "jpeg"], |name, bytes| {
            WebIoEvent::ImportImage { name, bytes, plane: None }
        });
    }

    /// Import a PNG/JPEG onto a *specific* construction plane (the Elements pane's
    /// right-click "Import image on this plane…", #175).
    #[cfg(target_arch = "wasm32")]
    fn import_image_on_plane(&mut self, plane: usize) {
        self.web_pick_file("Image", &["png", "jpg", "jpeg"], move |name, bytes| {
            WebIoEvent::ImportImage { name, bytes, plane: Some(plane) }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn export_stl_all(&mut self) {
        match self.state.export_stl_bytes(None) {
            Ok(bytes) => self.web_save_bytes(
                "STL mesh",
                &["stl"],
                "model.stl".to_string(),
                bytes,
                "Exported STL".to_string(),
            ),
            Err(e) => self.state.status = format!("Export failed: {e}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_step_all(&mut self) {
        match self.state.export_step_bytes(None) {
            Ok(bytes) => self.web_save_bytes(
                "STEP model",
                &["step", "stp"],
                "model.step".to_string(),
                bytes,
                "Exported STEP".to_string(),
            ),
            Err(e) => self.state.status = format!("Export failed: {e}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_stl_body(&mut self, body: usize) {
        let name = self.web_body_export_name(body, "stl");
        match self.state.export_stl_bytes(Some(body)) {
            Ok(bytes) => {
                self.web_save_bytes("STL mesh", &["stl"], name, bytes, "Exported STL".to_string())
            }
            Err(e) => self.state.status = format!("Export failed: {e}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_step_body(&mut self, body: usize) {
        let name = self.web_body_export_name(body, "step");
        match self.state.export_step_bytes(Some(body)) {
            Ok(bytes) => self.web_save_bytes(
                "STEP model",
                &["step", "stp"],
                name,
                bytes,
                "Exported STEP".to_string(),
            ),
            Err(e) => self.state.status = format!("Export failed: {e}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn web_body_export_name(&self, body: usize, ext: &str) -> String {
        let stem = self
            .state
            .doc
            .bodies
            .get(body)
            .and_then(|b| b.name.clone())
            .unwrap_or_else(|| format!("body-{body}"));
        format!("{stem}.{ext}")
    }

    #[cfg(target_arch = "wasm32")]
    fn export_session_commands(&mut self) {
        let timestamp = command_log::utc_timestamp();
        let script = match &self.state.command_log {
            Some(log) if !log.borrow().is_empty() => log.borrow().session_lua_script(&timestamp),
            _ => {
                self.state.status = "No session commands to export yet".to_string();
                return;
            }
        };
        self.web_save_bytes(
            "Lua script",
            &["lua"],
            format!("bearcad-session-{timestamp}.lua"),
            script.into_bytes(),
            "Exported session commands".to_string(),
        );
    }

    /// Pick a `.lua` script and run it against the live document (File → Load Script…).
    /// The same runner the `--script` CLI flag uses, driven from the running GUI.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_script(&mut self) {
        if self.script.as_ref().is_some_and(|r| !r.done) {
            self.state.status = "A script is already running".to_string();
            return;
        }
        let picked = rfd::FileDialog::new()
            .add_filter("Lua script", &["lua"])
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        match ScriptRunner::from_file(&path) {
            Ok(runner) => {
                self.script = Some(runner);
                self.state.status = format!("Running script: {}", path.display());
            }
            Err(e) => {
                self.state.status = format!("Could not load script: {}", e.message);
            }
        }
    }

    /// Pick a `.lua` file and run it in the browser through the Lua interpreter module
    /// (todoer #179/#207). The picked bytes are queued and run in `drain_web_io`, where the
    /// egui context needed to drive instruction execution is in scope.
    #[cfg(target_arch = "wasm32")]
    fn load_script(&mut self) {
        if !web_lua::available() {
            self.state.status =
                "Lua scripting is unavailable — the interpreter module didn't load".to_string();
            return;
        }
        self.web_pick_file("Lua script", &["lua"], |_name, bytes| WebIoEvent::RunScript {
            bytes,
        });
    }

    /// Dispatch one menu command — shared by the native OS menu bar and the web build's
    /// in-window menu bar, so both frontends behave identically.
    fn handle_menu_command(&mut self, ctx: &egui::Context, command: MenuCommand) {
        match command {
            MenuCommand::Open => self.open(),
            MenuCommand::Save => self.save(),
            MenuCommand::SaveAs => self.save_as(),
            MenuCommand::ExportStl => self.export_stl_all(),
            MenuCommand::ExportStep => self.export_step_all(),
            MenuCommand::ImportStl => self.import_stl(),
            MenuCommand::ImportImage => self.import_image(),
            MenuCommand::ImportStep => self.import_step(),
            MenuCommand::ExportSessionCommands => self.export_session_commands(),
            MenuCommand::DocumentJson => self.open_json_dialog(),
            MenuCommand::LoadScript => self.load_script(),
            MenuCommand::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            MenuCommand::About => {
                self.state.status = format!(
                    "BearCAD — on-device parametric CAD (prototype) • {}",
                    kernel::selftest()
                );
            }
            MenuCommand::Licenses => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.state.status = match open_licenses_document() {
                        Ok(()) => "Opened licenses document in your browser".to_string(),
                        Err(err) => format!("Could not open licenses document: {err}"),
                    };
                }
                #[cfg(target_arch = "wasm32")]
                ctx.open_url(egui::OpenUrl::new_tab(
                    "https://github.com/iffy/BearCAD/blob/master/LICENSES-THIRD-PARTY.md",
                ));
            }
            MenuCommand::InstallCli => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.state.status = match cli_install::run_install() {
                        Ok(msg) => msg,
                        Err(err) => format!("Install CLI failed: {err}"),
                    };
                }
            }
            _ => {
                if let Some(action) = command.to_action() {
                    self.state.apply(action);
                }
            }
        }
    }

    /// Handle selections from the native OS menu bar.
    #[cfg(not(target_arch = "wasm32"))]
    fn handle_native_menu(&mut self, ctx: &egui::Context) {
        let events = self.native_menu.drain_events();
        for event in events {
            let Some(command) = native_menu::command_for_event(&event, &self.native_menu) else {
                continue;
            };
            self.handle_menu_command(ctx, command);
        }

        self.native_menu
            .sync_pane_checks(|pane| self.state.panes.is_visible(pane));
        self.native_menu.sync_fps_mode(self.state.fps.is_some());
    }

    fn dispatch_palette_outcome(&mut self, outcome: PaletteOutcome) {
        match outcome {
            PaletteOutcome::Action(action) => {
                self.state.apply(action);
            }
            PaletteOutcome::OpenFile => self.open(),
            PaletteOutcome::SaveFile => self.save(),
            PaletteOutcome::SaveFileAs => self.save_as(),
            PaletteOutcome::ExportSessionCommands => self.export_session_commands(),
            PaletteOutcome::DocumentJson => self.open_json_dialog(),
        }
        self.state.command_palette.close_palette();
    }

    /// Drive first-person mode (#91): cursor lock, Esc to leave, weapon-style tool
    /// switching (1-9 slots, wheel cycles), mouse look from raw pointer motion, and
    /// WASD/Space/Shift movement physics. Runs before `handle_keyboard_shortcuts`,
    /// which stands down on bare keys while FPS mode is active.
    fn tick_fps_mode(&mut self, ctx: &egui::Context, dt: f32) {
        let active = self.state.fps.is_some();
        if active != self.fps_cursor_grabbed {
            if active {
                // Entering: the cursor lock/pin below can surface the warp to the viewport
                // center as one huge motion delta (possibly split across a few frames by
                // pointer smoothing) — drop the first frames of mouse look so entry doesn't
                // spin the view (#135; see `fps_look_warmup`).
                self.fps_look_warmup = 10;
            }
            let grab = if active {
                // macOS gets *no* grab: `CursorGrab::Locked` freezes the pointer so egui reports
                // no motion and mouse-look dies (#121); instead we warp the visible cursor back to
                // the crosshair each frame (below) and derive the look delta from its offset.
                // Everywhere else, `Locked` is the real thing.
                #[cfg(target_os = "macos")]
                {
                    egui::viewport::CursorGrab::None
                }
                #[cfg(not(target_os = "macos"))]
                {
                    egui::viewport::CursorGrab::Locked
                }
            } else {
                egui::viewport::CursorGrab::None
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(grab));
            // On macOS, winit builds its hidden-cursor image by decoding a static GIF through
            // ImageIO the first time the view resets its cursor rects; that decode has been
            // observed to SIGBUS on entering FPS mode (#119). So skip `CursorVisible` there and
            // instead pin the still-visible arrow to the crosshair (below) — everywhere else the
            // cursor is genuinely hidden as SPEC promises.
            #[cfg(not(target_os = "macos"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!active));
            self.fps_cursor_grabbed = active;
        }
        if !active {
            return;
        }

        #[cfg(target_os = "macos")]
        if let Some(viewport) = self.last_viewport {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(viewport.center()));
        }

        if !keyboard_shortcuts_suppressed(ctx) {
            // Esc leaves FPS mode (consumed so it doesn't also cancel an operation).
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.state.apply(Action::ToggleFpsMode);
                return;
            }

            // Weapon slots: 1-9 pick a tool directly.
            const SLOT_KEYS: [egui::Key; 9] = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            for (slot, key) in SLOT_KEYS.iter().enumerate() {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, *key)) {
                    if let Some(tool) = fps::TOOL_SLOTS.get(slot) {
                        self.state.apply(Action::SetTool(*tool));
                    }
                }
            }

            // `[`/`]` shrink/grow the player (#120): eye height, move/jump speed, and
            // gravity all scale together, so mm-detail work and building-scale walkthroughs
            // are both comfortable without leaving FPS mode.
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)) {
                if let Some(player) = self.state.fps.as_mut() {
                    player.scale_by(1.0 / fps::SCALE_STEP);
                }
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)) {
                if let Some(player) = self.state.fps.as_mut() {
                    player.scale_by(fps::SCALE_STEP);
                }
            }

            // Delete still works so selections can be removed without leaving FPS mode.
            let delete_pressed = ctx.input(|i| {
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
            });
            if delete_pressed && !self.state.scene_selection.is_empty() {
                self.state.apply(Action::DeleteSelection);
            }
        }

        // Wheel cycles tools (the viewport skips zoom while in FPS mode).
        let scroll = ctx.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() >= 1.0 {
            let step = if scroll < 0.0 { 1 } else { -1 };
            self.state.apply(Action::SetTool(fps::cycle_tool(self.state.tool, step)));
        }

        // Mouse look. Off macOS the pointer is grabbed at the centre, so egui's frame-to-frame
        // motion is the look delta. On macOS the pointer isn't grabbed (see above); the delta is
        // instead its offset from the crosshair, which the per-frame warp resets to zero — so an
        // un-moved cursor contributes nothing and a moved one contributes exactly its motion.
        if self.fps_look_warmup > 0 {
            self.fps_look_warmup -= 1;
        } else {
            #[cfg(target_os = "macos")]
            let look = self.last_viewport.and_then(|viewport| {
                ctx.input(|i| i.pointer.latest_pos())
                    .map(|pos| pos - viewport.center())
                    .filter(|d| *d != egui::Vec2::ZERO)
            });
            #[cfg(not(target_os = "macos"))]
            let look = ctx.input(|i| i.pointer.motion());
            if let Some(look) = look {
                if let Some(player) = self.state.fps.as_mut() {
                    player.look_by_pixels(look.x, look.y);
                }
            }
        }

        // Movement physics — while a text field has focus (typing a dimension), the
        // player stands still, like an FPS with a menu open.
        let mut input = fps::FpsInput::default();
        if !keyboard_shortcuts_suppressed(ctx) {
            ctx.input(|i| {
                input.forward = i.key_down(egui::Key::W);
                input.back = i.key_down(egui::Key::S);
                input.left = i.key_down(egui::Key::A);
                input.right = i.key_down(egui::Key::D);
                input.jump_pressed = i.key_pressed(egui::Key::Space);
                input.ascend = i.key_down(egui::Key::Space);
                input.descend = i.modifiers.shift;
            });
        }
        if let Some(player) = self.state.fps.as_mut() {
            player.tick(dt, input);
            player.clone().apply_to_camera(&mut self.state.cam);
        }
        // Physics needs steady frames even with no input events pending.
        ctx.request_repaint();
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.state.command_palette.open {
            return;
        }

        // Y projects the selected body edges into the open sketch (#140).
        if self.state.sketch_session.is_some()
            && !keyboard_shortcuts_suppressed(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Y))
        {
            self.state.apply(Action::ProjectSelection);
        }

        // Cmd/Ctrl+B toggles curve mode (#127) even while the in-progress line's inline
        // length field has keyboard focus — unlike the plain-letter shortcuts below, which
        // intentionally stand down during text entry. Plain `B` used to double as this
        // shortcut, but that collided with typing a length expression containing the
        // letter b (e.g. a variable name), so it silently just typed "b" instead of
        // toggling. The modifier disambiguates it from ordinary typing.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.state.apply(Action::ToggleCurveMode);
        }

        // While any text field has focus, leave unmodified keys to the input (e.g. "bar" must not
        // switch tools on "r"). Modifier shortcuts (Cmd/Ctrl+P, etc.) use the OS menu layer.
        // FPS mode (#91) owns the bare keys entirely: WASD move, Space jumps, digits switch
        // tools — `tick_fps_mode` handles them before this runs.
        if !keyboard_shortcuts_suppressed(ctx) && self.state.fps.is_none() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.state.apply(Action::CancelOperation);
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::S))
            {
                if self.state.tool != Tool::Sketch {
                    self.state.apply(Action::SetTool(Tool::Sketch));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::R))
            {
                if self.state.tool != Tool::Rectangle {
                    self.state.apply(Action::SetTool(Tool::Rectangle));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::L))
            {
                if self.state.tool != Tool::Line {
                    self.state.apply(Action::SetTool(Tool::Line));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::C))
            {
                if self.state.tool == Tool::Constraint && !self.state.scene_selection.is_empty() {
                    let rows = crate::geometric_constraints::constraint_pane_rows(
                        &self.state.scene_selection,
                    );
                    if let Some(kind) =
                        crate::geometric_constraints::sole_enabled_constraint_type(&rows)
                    {
                        self.state.apply(Action::AddGeometricConstraint(kind));
                    }
                } else if self.state.tool != Tool::Constraint {
                    self.state.apply(Action::SetTool(Tool::Constraint));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::O))
            {
                if self.state.tool != Tool::Circle {
                    self.state.apply(Action::SetTool(Tool::Circle));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::P))
            {
                if self.state.tool != Tool::ConstructionPlane {
                    self.state.apply(Action::SetTool(Tool::ConstructionPlane));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::D))
            {
                self.state.apply(Action::SetTool(Tool::Dimension));
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::E))
            {
                if self.state.tool != Tool::Extrude {
                    self.state.apply(Action::SetTool(Tool::Extrude));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::K))
            {
                if self.state.tool != Tool::Chamfer {
                    self.state.apply(Action::SetTool(Tool::Chamfer));
                }
            }

            if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::F))
            {
                if self.state.tool != Tool::Fillet {
                    self.state.apply(Action::SetTool(Tool::Fillet));
                }
            }

            if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                self.state.apply(Action::ToggleConstruction);
            }

            // Z: zoom to fit — the selection if anything is selected, else everything (#279).
            // `consume_key(NONE, …)` requires no modifier, so it never catches Cmd/Ctrl+Z (undo).
            // In the Drawing workbench it fits the page instead (reset the drawing pan/zoom).
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Z)) {
                if self.state.editing_drawing.is_some() {
                    self.drawing_zoom = 1.0;
                    self.drawing_pan = egui::Vec2::ZERO;
                } else {
                    self.state.apply(Action::ZoomToFit);
                }
            }

            // `T` selects the Text tool (#311), except where it already means the Tangent
            // constraint: the Constraint tool (its mnemonic, handled below), while drawing a
            // line, or with a sketch vertex selected (#73 curve mode). It's also the mnemonic
            // for tangent in the Constraint tool.
            if self.state.tool != Tool::Constraint && ctx.input(|i| i.key_pressed(egui::Key::T)) {
                let tangent_context = self.state.creating_line.is_some()
                    || self.state.tool == Tool::Line
                    || self
                        .state
                        .scene_selection
                        .iter()
                        .any(|e| matches!(e, SceneElement::Point(_)));
                if tangent_context {
                    self.state.apply(Action::ToggleTangentConstraint);
                } else if self.state.tool != Tool::Text {
                    self.state.apply(Action::SetTool(Tool::Text));
                }
            }

            if ctx.input(|i| i.key_pressed(egui::Key::N)) {
                self.state.apply(Action::FocusElementName);
            }

            let delete_pressed = ctx.input(|i| i.key_pressed(egui::Key::Delete))
                || ctx.input(|i| i.key_pressed(egui::Key::Backspace));
            if delete_pressed && self.selected_calibration_point.is_some() {
                // #424: a selected calibration point deletes; the calibration re-opens with
                // the other point so a click re-places this one.
                if let Some((image, index)) = self.selected_calibration_point.take() {
                    self.state.apply(Action::RemoveCalibrationPoint { image, index });
                }
            } else if delete_pressed && self.selected_bezier_handle.is_some() {
                // #75: deleting a selected bezier handle straightens its line — there's no
                // independent per-handle state to remove (a curve is either both handles or
                // neither, see `Line::bezier`).
                if let Some((line, _)) = self.selected_bezier_handle.take() {
                    self.state.apply(Action::StraightenLine { line });
                }
            } else if self.state.creating_rect.is_none()
                && self.state.creating_line.is_none()
                && self.state.creating_circle.is_none()
                && self.state.creating_plane.is_none()
                && !self.state.scene_selection.is_empty()
                && delete_pressed
            {
                self.state.apply(Action::DeleteSelection);
            } else if delete_pressed
                && self.state.editing_drawing.is_some()
                && !ctx.wants_keyboard_input()
            {
                // Delete/Backspace removes every selected drawing element (#336/#346): placed
                // projections, text notes, and shown dimensions. The `wants_keyboard_input` guard
                // keeps Backspace editing the annotation textarea instead of deleting the note.
                let selected = self.state.selected_drawing_elements.clone();
                let mut views_to_remove: Vec<(usize, usize)> = Vec::new();
                for (drawing, element) in selected {
                    match element {
                        context::DrawingElementRef::Projection(view) => {
                            views_to_remove.push((drawing, view));
                        }
                        context::DrawingElementRef::Text(annotation) => {
                            self.state
                                .apply(Action::RemoveDrawingAnnotation { drawing, annotation });
                        }
                        context::DrawingElementRef::Dimension { view, a, b } => {
                            // A selected dimension deletes by hiding it (toggling it off).
                            self.state
                                .apply(Action::ToggleDrawingDimension { drawing, view, a, b });
                        }
                    }
                }
                // Remove views highest-index-first so earlier indices stay valid as later ones
                // shift down.
                views_to_remove.sort_by(|a, b| b.1.cmp(&a.1));
                for (drawing, view) in views_to_remove {
                    self.state.apply(Action::RemoveDrawingView { drawing, view });
                }
                self.state.clear_drawing_selection();
            }

            if self.state.tool == Tool::Constraint {
                // Digit shortcuts 1–7 for the constraint pane rows (#401, see
                // GeometricConstraintType::shortcut_label) — Constraint tool only.
                for (key, egui_key) in [
                    ('1', egui::Key::Num1),
                    ('2', egui::Key::Num2),
                    ('3', egui::Key::Num3),
                    ('4', egui::Key::Num4),
                    ('5', egui::Key::Num5),
                    ('6', egui::Key::Num6),
                    ('7', egui::Key::Num7),
                ] {
                    if ctx.input(|i| i.key_pressed(egui_key)) {
                        self.state.apply(Action::ApplyConstraintShortcut(key));
                    }
                }
            }
        }

        if self.state.tool != Tool::Rectangle || self.state.sketch_session.is_none() {
            self.state.creating_rect = None;
        }
        if self.state.tool != Tool::Line || self.state.sketch_session.is_none() {
            self.state.creating_line = None;
        }
        if self.state.tool != Tool::Circle || self.state.sketch_session.is_none() {
            self.state.creating_circle = None;
        }
        if self.state.tool != Tool::ConstructionPlane {
            self.state.creating_plane = None;
        }
        if !matches!(self.state.tool, Tool::Select | Tool::Dimension) {
            self.state.editing_committed_dim = None;
        }
    }

    fn process_screenshots(&mut self, ctx: &egui::Context) {
        let screenshots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        if screenshots.is_empty() {
            return;
        }

        if let Some(runner) = &mut self.script {
            for image in screenshots {
                if let Err(e) = runner.on_screenshot(&image) {
                    runner.error = Some(e);
                    runner.done = true;
                    self.state.status = format!("Script error: {}", runner.error.as_deref().unwrap_or(""));
                }
            }
        }
    }

    /// Extrude tool interaction: click faces to toggle inclusion, and drag the normal gizmo
    /// (rendered in the GPU scene) to set the distance, snapping to objects under the cursor.
    fn handle_extrude_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
    ) {
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

        // If the in-progress extrusion went away (committed or cancelled), stop following.
        if self.state.creating_extrusion.is_none() {
            self.extrude_gizmo_drag = None;
            self.pending_extrude_target = None;
        }

        // Snapshot the pending extrusion so we can mutate state without holding a borrow.
        let pending = self
            .state
            .creating_extrusion
            .as_ref()
            .filter(|ce| !ce.faces.is_empty())
            .map(|ce| (ce.faces.clone(), ce.evaluated_distance(&self.state.doc)));

        // The handle is a click-to-grab control: one click grabs it, then it follows
        // the cursor (no held button) until the next click, which finishes the extrude.
        let following = self.extrude_gizmo_drag.is_some();
        let mut gizmo_active = false;
        if let Some((faces, distance)) = &pending {
            if let Some((origin, normal)) = extrude::faces_anchor(&self.state.doc, faces) {
                let handle_offset = extrude_gizmo_display_offset(*distance);
                let hovered = pointer_screen.is_some_and(|pp| {
                    construction::offset_gizmo_hit(pp, project, origin, normal, handle_offset)
                });
                if !following && primary_pressed && hovered {
                    if let Some(pp) = pointer_screen {
                        self.extrude_gizmo_drag = Some(ExtrudeGizmoDrag {
                            start_screen: pp,
                            start_distance: *distance,
                        });
                        // Grabbing the gizmo hands distance control back to it,
                        // so the typed text resyncs to the dragged value.
                        if let Some(ce) = self.state.creating_extrusion.as_mut() {
                            ce.user_edited = false;
                        }
                        // Release the distance field's keyboard focus so a subsequent
                        // keystroke overwrites the dragged value rather than appending to it.
                        ui.ctx().memory_mut(|m| {
                            m.surrender_focus(egui::Id::new(EXTRUDE_DISTANCE_FIELD_ID))
                        });
                    }
                }
                // While following, track the cursor every frame (no button required).
                if let Some(drag) = self.extrude_gizmo_drag {
                    gizmo_active = true;
                    if let Some(pp) = pointer_screen {
                        if let Some((target, dist)) = pick_extrude_target(
                            pp,
                            project,
                            &self.state.doc,
                            origin,
                            normal,
                            faces,
                            self.state.cam.eye(),
                            self.state.creating_extrusion.as_ref().and_then(|ce| ce.edit_index),
                        ) {
                            self.pending_extrude_target = Some(target);
                            self.state.apply(Action::SetExtrudeDistance { distance: dist });
                        } else {
                            self.pending_extrude_target = None;
                            let new_distance = construction::offset_from_normal_drag(
                                origin,
                                normal,
                                project,
                                drag.start_distance,
                                drag.start_screen,
                                pp,
                            );
                            self.state
                                .apply(Action::SetExtrudeDistance { distance: new_distance });
                        }
                    }
                }
            }
        }

        // A click while following commits the extrusion, snapping to any pending target.
        if following && primary_pressed {
            let target = self.pending_extrude_target.take();
            self.state.apply(Action::SetExtrudeTarget { target });
            self.state.apply(Action::CommitExtrusion);
            self.extrude_gizmo_drag = None;
            return;
        }
        if gizmo_active {
            return;
        }

        // Click toggles the face under the cursor (highlighted via the GPU hover).
        if primary_pressed {
            if let Some(pp) = pointer_screen {
                // A sketch text under the cursor (#285): toggle all its glyph faces at once, so
                // the whole string extrudes/cuts as one selection.
                if let Some(faces) = self.text_glyph_faces_at(pp, cam, viewport, vp) {
                    let all_present = faces
                        .iter()
                        .all(|f| self.state.creating_extrusion.as_ref().is_some_and(|c| c.faces.contains(f)));
                    for face in faces {
                        // Toggle each; if all were present we remove, else the missing ones add.
                        let present = self
                            .state
                            .creating_extrusion
                            .as_ref()
                            .is_some_and(|c| c.faces.contains(&face));
                        if present == all_present {
                            self.state.apply(Action::ToggleExtrudeFace { face });
                        }
                    }
                    return;
                }
                if let Some(face) = pick_extrude_face(
                    pp,
                    project,
                    &self.state.doc,
                    self.state.cam.eye(),
                    cam,
                    viewport,
                    vp,
                ) {
                    self.state.apply(Action::ToggleExtrudeFace { face });
                } else if let Some(face_id) =
                    pick_extrude_body_face(pp, project, &self.state.doc, self.state.cam.eye())
                {
                    // A bare body face (#122): push/pull it directly, no separate sketch.
                    self.state.apply(Action::ExtrudeBodyFace { face_id });
                }
            }
        }
    }

    /// Floating distance field for the in-progress extrusion (Enter commits).
    fn show_extrude_distance_input(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let pos = {
            let Some(ce) = self.state.creating_extrusion.as_ref() else {
                return;
            };
            if ce.faces.is_empty() {
                return;
            }
            let handle_offset = extrude_gizmo_display_offset(ce.evaluated_distance(&self.state.doc));
            extrude::faces_anchor(&self.state.doc, &ce.faces)
                .map(|(o, n)| construction::offset_handle(o, n, handle_offset))
                .and_then(project)
                .map(|p| p + egui::vec2(14.0, -12.0))
        };
        let Some(pos) = pos else {
            return;
        };
        let ctx = ui.ctx();
        let id = egui::Id::new(EXTRUDE_DISTANCE_FIELD_ID);
        let mut commit = false;
        let mut flip = false;

        // Enter commits the extrusion even when the distance field is unfocused (e.g.
        // while driving depth with the pull handle), matching the other sketch tools.
        if !ctx.memory(|m| m.has_focus(id)) && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.state.apply(Action::CommitExtrusion);
            return;
        }

        // Typing while the field is unfocused grabs focus and overwrites the current value,
        // so the user can just start typing a depth. Any expression character is accepted —
        // not only digits — so a unit or parameter expression like `15mm` or `width=2` can be
        // typed from the first keystroke (#196).
        if !ctx.memory(|m| m.has_focus(id)) {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            let typed: String = typed
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "._-+*/()= ".contains(*c))
                .collect();
            if !typed.is_empty() {
                if let Some(ce) = self.state.creating_extrusion.as_mut() {
                    ce.text = typed;
                    ce.user_edited = true;
                    ce.pending_focus = true;
                }
            }
        }
        if let Some(ce) = self.state.creating_extrusion.as_mut() {
            let want_focus = ce.pending_focus;
            egui::Area::new(egui::Id::new("extrude_distance_area"))
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut ce.text)
                                .id(id)
                                .desired_width(64.0),
                        );
                        if resp.changed() {
                            ce.user_edited = true;
                        }
                        if want_focus {
                            resp.request_focus();
                            ce.pending_focus = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit = true;
                        }
                        // Flip which side of the sketch plane the profile extrudes to (#354):
                        // negating the distance reverses the direction while keeping the depth.
                        if ui
                            .button("Flip")
                            .on_hover_text("Extrude to the other side of the sketch")
                            .clicked()
                        {
                            flip = true;
                        }
                    });
                });
        }
        if flip {
            if let Some(d) = self
                .state
                .creating_extrusion
                .as_ref()
                .map(|ce| ce.evaluated_distance(&self.state.doc))
            {
                // A zero depth has no side to flip; nudge to a small default so the flip is visible.
                let d = if d.abs() < 1e-4 { actions::DEFAULT_EXTRUDE_DISTANCE } else { d };
                self.state.apply(Action::SetExtrudeDistance { distance: -d });
            }
        }
        if commit {
            self.state.apply(Action::CommitExtrusion);
        }
    }

    /// Chamfer/fillet tool interaction: click a two-line sketch vertex to start, then drag the
    /// gizmo (rendered in the GPU scene, reusing the extrude gizmo's mesh/hit-testing) or type
    /// an amount, mirroring [`Self::handle_extrude_tool`] closely.
    fn handle_vertex_treatment_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
    ) {
        let Some(session) = self.state.sketch_session else {
            self.state.creating_vertex_treatment = None;
            self.vertex_treatment_gizmo_drag = None;
            return;
        };
        let kind = match self.state.tool {
            Tool::Chamfer => VertexTreatmentKind::Chamfer,
            Tool::Fillet => VertexTreatmentKind::Fillet,
            _ => return,
        };
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

        // If the in-progress treatment went away (committed or cancelled), stop following.
        if self.state.creating_vertex_treatment.is_none() {
            self.vertex_treatment_gizmo_drag = None;
        }

        let anchor = self
            .state
            .creating_vertex_treatment
            .as_ref()
            .and_then(|cvt| vertex_treatment_anchor(&self.state.doc, session.sketch, cvt.point.clone()));

        let following = self.vertex_treatment_gizmo_drag.is_some();
        let mut gizmo_active = false;
        if let Some((origin, normal)) = anchor {
            let amount = self
                .state
                .creating_vertex_treatment
                .as_ref()
                .map(|cvt| cvt.evaluated_amount(&self.state.doc))
                .unwrap_or(0.0);
            let handle_offset = construction::gizmo_display_offset(amount);
            let hovered = pointer_screen.is_some_and(|pp| {
                construction::offset_gizmo_hit(pp, project, origin, normal, handle_offset)
            });
            if !following && primary_pressed && hovered {
                if let Some(pp) = pointer_screen {
                    self.vertex_treatment_gizmo_drag = Some(VertexTreatmentGizmoDrag {
                        start_screen: pp,
                        start_amount: amount,
                    });
                    if let Some(cvt) = self.state.creating_vertex_treatment.as_mut() {
                        cvt.user_edited = false;
                    }
                    ui.ctx().memory_mut(|m| {
                        m.surrender_focus(egui::Id::new(VERTEX_TREATMENT_AMOUNT_FIELD_ID))
                    });
                }
            }
            if let Some(drag) = self.vertex_treatment_gizmo_drag {
                gizmo_active = true;
                if let Some(pp) = pointer_screen {
                    let new_amount = construction::offset_from_normal_drag(
                        origin,
                        normal,
                        project,
                        drag.start_amount,
                        drag.start_screen,
                        pp,
                    )
                    .max(0.0);
                    if let Some(cvt) = self.state.creating_vertex_treatment.as_mut() {
                        cvt.amount_live = new_amount;
                        if !cvt.user_edited {
                            let unit = crate::model::effective_length_unit(
                                &self.state.doc,
                                session.sketch,
                            );
                            cvt.text = crate::value::format_length_display_in(new_amount, unit);
                        }
                    }
                }
            }
        }

        // A click while following commits the treatment.
        if following && primary_pressed {
            if let Some(mut cvt) = self.state.creating_vertex_treatment.take() {
                // #201: a typed amount can define a parameter (`name = expr`).
                let _ = actions::commit_inline_parameter_defs(&mut self.state.doc, [&mut cvt.text]);
                let amount = cvt.evaluated_amount(&self.state.doc);
                self.state.apply(Action::CommitVertexTreatment {
                    point: cvt.point,
                    kind: cvt.kind,
                    amount,
                });
            }
            self.vertex_treatment_gizmo_drag = None;
            return;
        }
        if gizmo_active {
            return;
        }

        // Click a vertex where exactly two plain lines meet to begin.
        if primary_pressed && self.state.creating_vertex_treatment.is_none() {
            if let Some(pp) = pointer_screen {
                if let Some((point, _)) =
                    nearest_sketch_point_in_sketch(pp, project, &self.state.doc, session.sketch)
                {
                    if vertex_incident_line_count(&self.state.doc, session.sketch, point.clone()) == 2 {
                        let unit = crate::model::effective_length_unit(
                            &self.state.doc,
                            session.sketch,
                        );
                        self.state.creating_vertex_treatment = Some(CreatingVertexTreatment {
                            point,
                            kind,
                            amount_live: DEFAULT_VERTEX_TREATMENT_AMOUNT,
                            text: crate::value::format_length_display_in(
                                DEFAULT_VERTEX_TREATMENT_AMOUNT,
                                unit,
                            ),
                            user_edited: false,
                            pending_focus: true,
                        });
                    }
                }
            }
        }
    }

    /// Floating amount field for the in-progress chamfer/fillet (Enter commits). Mirrors
    /// [`Self::show_extrude_distance_input`].
    fn show_vertex_treatment_amount_input(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let pos = {
            let Some(session) = self.state.sketch_session else {
                return;
            };
            let Some(cvt) = self.state.creating_vertex_treatment.as_ref() else {
                return;
            };
            let Some((origin, normal)) =
                vertex_treatment_anchor(&self.state.doc, session.sketch, cvt.point.clone())
            else {
                return;
            };
            let handle_offset =
                construction::gizmo_display_offset(cvt.evaluated_amount(&self.state.doc));
            project(construction::offset_handle(origin, normal, handle_offset))
                .map(|p| p + egui::vec2(14.0, -12.0))
        };
        let Some(pos) = pos else {
            return;
        };
        let ctx = ui.ctx();
        let id = egui::Id::new(VERTEX_TREATMENT_AMOUNT_FIELD_ID);
        let mut commit = false;

        // Enter commits even when the field is unfocused (e.g. while dragging the gizmo).
        if !ctx.memory(|m| m.has_focus(id)) && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            commit = true;
        }

        // Typing a number while unfocused grabs focus and overwrites the current value.
        if !ctx.memory(|m| m.has_focus(id)) {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            let typed: String = typed
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !typed.is_empty() {
                if let Some(cvt) = self.state.creating_vertex_treatment.as_mut() {
                    cvt.text = typed;
                    cvt.user_edited = true;
                    cvt.pending_focus = true;
                }
            }
        }
        if let Some(cvt) = self.state.creating_vertex_treatment.as_mut() {
            let label = match cvt.kind {
                VertexTreatmentKind::Chamfer => "mm",
                VertexTreatmentKind::Fillet => "mm r",
            };
            let want_focus = cvt.pending_focus;
            egui::Area::new(egui::Id::new("vertex_treatment_amount_area"))
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut cvt.text)
                                .id(id)
                                .desired_width(64.0),
                        );
                        if resp.changed() {
                            cvt.user_edited = true;
                        }
                        if want_focus {
                            resp.request_focus();
                            cvt.pending_focus = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit = true;
                        }
                        ui.label(label);
                    });
                });
        }
        if commit {
            if let Some(mut cvt) = self.state.creating_vertex_treatment.take() {
                // #201: a typed amount can define a parameter (`name = expr`).
                let _ = actions::commit_inline_parameter_defs(&mut self.state.doc, [&mut cvt.text]);
                let amount = cvt.evaluated_amount(&self.state.doc);
                self.state.apply(Action::CommitVertexTreatment {
                    point: cvt.point,
                    kind: cvt.kind,
                    amount,
                });
            }
        }
    }

    /// 3D edge chamfer/fillet tool interaction (#77): click an extrusion's analytic edge
    /// (vertical or side/cap — see `ExtrusionEdgeRef`) to start, then drag the gizmo or type an
    /// amount. Mirrors [`Self::handle_vertex_treatment_tool`] closely; active precisely when
    /// that one isn't (no sketch open), since the Chamfer/Fillet tool is shared between the 2D
    /// sketch-vertex case and this 3D solid-edge case.
    /// The output bodies of the operation currently being **edited** (#260), used as the seed for
    /// fading their descendants. Empty when nothing is being edited.
    /// World-space dashed ghost segments previewing the in-progress in-sketch repeat (#232):
    /// each picked line/circle copied to every computed offset along the repeat direction.
    fn sketch_repeat_ghost_segments(&self) -> Vec<(Vec3, Vec3)> {
        let Some(cr) = self.state.creating_sketch_repeat.as_ref() else {
            return Vec::new();
        };
        if !cr.has_targets() {
            return Vec::new();
        }
        let doc = &self.state.doc;
        let (du, dv) = cr.direction(doc);
        let probe = model::SketchRepeatOperation {
            sketch: cr.sketch,
            line_targets: cr.line_targets.clone(),
            circle_targets: cr.circle_targets.clone(),
            dir_u: du,
            dir_v: dv,
            mode: cr.mode,
            count: cr.count.clone(),
            spacing: cr.spacing.clone(),
            length: cr.length.clone(),
            line_outputs: Vec::new(),
            circle_outputs: Vec::new(),
            name: None,
            deleted: false,
        };
        let Some(offsets) = extrude::sketch_repeat_offsets(doc, &probe) else {
            return Vec::new();
        };
        let Some(frame) = crate::face::sketch_geometry_frame(doc, cr.sketch) else {
            return Vec::new();
        };
        let mut segs = Vec::new();
        for &off in &offsets {
            if off.abs() <= 1e-6 {
                continue; // the source position already renders; only ghost the copies
            }
            let (ou, ov) = (off * du, off * dv);
            for &li in &cr.line_targets {
                if let Some(l) = doc.lines.get(li).filter(|l| !l.deleted) {
                    let a = crate::face::local_to_world(&frame, l.x0 + ou, l.y0 + ov);
                    let b = crate::face::local_to_world(&frame, l.x1 + ou, l.y1 + ov);
                    segs.push((a, b));
                }
            }
            for &ci in &cr.circle_targets {
                if let Some(c) = doc.circles.get(ci).filter(|c| !c.deleted) {
                    const N: usize = 48;
                    let mut prev = None;
                    for k in 0..=N {
                        let t = k as f32 / N as f32 * std::f32::consts::TAU;
                        let p = crate::face::local_to_world(
                            &frame,
                            c.cx + ou + c.r * t.cos(),
                            c.cy + ov + c.r * t.sin(),
                        );
                        if let Some(q) = prev {
                            segs.push((q, p));
                        }
                        prev = Some(p);
                    }
                }
            }
        }
        segs
    }

    fn edited_operation_output_bodies(&self) -> Vec<usize> {
        let s = &self.state;
        // Editing an extrusion: its body (bodies whose source owns that extrusion).
        if let Some(ei) = s.creating_extrusion.as_ref().and_then(|ce| ce.edit_index) {
            return s
                .doc
                .bodies
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.deleted && b.source.owns_extrusion(ei))
                .map(|(bi, _)| bi)
                .collect();
        }
        if let Some(op) = s.creating_move.as_ref().and_then(|c| c.editing) {
            return s.doc.move_ops.get(op).map(|o| o.outputs.clone()).unwrap_or_default();
        }
        if let Some(op) = s.creating_boolean.as_ref().and_then(|c| c.editing) {
            return s.doc.boolean_ops.get(op).map(|o| o.outputs.clone()).unwrap_or_default();
        }
        if let Some(op) = s.creating_repeat.as_ref().and_then(|c| c.editing) {
            return s.doc.repeat_ops.get(op).map(|o| o.outputs.clone()).unwrap_or_default();
        }
        if let Some(op) = s.creating_slice.as_ref().and_then(|c| c.editing) {
            return s.doc.slice_ops.get(op).map(|o| o.outputs.clone()).unwrap_or_default();
        }
        if let Some(op) = s.creating_revolve.as_ref().and_then(|c| c.editing) {
            return s
                .doc
                .bodies
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.deleted && b.source == model::BodySource::Revolve(op))
                .map(|(bi, _)| bi)
                .collect();
        }
        Vec::new()
    }

    /// Live geometry for the descendants of the operation being edited (#260): a scratch clone of
    /// the document with the in-progress gizmo edit written into it, meshed per descendant body.
    /// Rendered in preview styling in place of the stale (faded) geometry so downstream bodies
    /// visibly follow the drag. Empty when the edit type isn't one we can replay onto a scratch
    /// doc (then the fade alone stands in).
    fn edit_preview_descendant_meshes(&self) -> std::collections::HashMap<usize, extrude::SolidMesh> {
        use std::collections::HashMap;
        let seeds = self.edited_operation_output_bodies();
        if seeds.is_empty() {
            return HashMap::new();
        }
        let descendants = extrude::descendant_bodies(&self.state.doc, &seeds);
        if descendants.is_empty() {
            return HashMap::new();
        }
        let mut scratch = self.state.doc.clone();
        if !self.apply_active_edit_to_scratch(&mut scratch) {
            return HashMap::new();
        }
        descendants
            .iter()
            .filter_map(|&bi| extrude::body_solid_mesh_uncached_pub(&scratch, bi).map(|m| (bi, m)))
            .collect()
    }

    /// Write the in-progress gizmo edit's geometry into `doc` (a scratch clone), for the edit
    /// types whose result is a pure function of a few op fields — extrude distance/faces, a move
    /// transform, a revolve angle. Returns `false` (leaving `doc` untouched) for edits we don't
    /// replay, so [`edit_preview_descendant_meshes`] falls back to the plain fade.
    fn apply_active_edit_to_scratch(&self, doc: &mut model::Document) -> bool {
        let s = &self.state;
        if let Some(ce) = s.creating_extrusion.as_ref() {
            if let Some(idx) = ce.edit_index {
                if let Some(ext) = doc.extrusions.get_mut(idx) {
                    ext.faces = ce.faces.clone();
                    ext.distance = ce.evaluated_distance(&s.doc);
                    ext.target = ce.target.clone();
                    return true;
                }
            }
        }
        if let Some(cm) = s.creating_move.as_ref() {
            if let Some(op) = cm.editing {
                if let Some(mv) = doc.move_ops.get_mut(op) {
                    mv.tx = cm.tx.clone();
                    mv.ty = cm.ty.clone();
                    mv.tz = cm.tz.clone();
                    mv.angle = cm.angle.clone();
                    mv.axis = cm.axis;
                    return true;
                }
            }
        }
        if let Some(cr) = s.creating_revolve.as_ref() {
            if let Some(op) = cr.editing {
                if let Some(rev) = doc.revolutions.get_mut(op) {
                    rev.angle_deg = cr.evaluated_angle_deg(&s.doc);
                    rev.symmetric = cr.symmetric;
                    if let Some(axis) = cr.axis {
                        rev.axis = axis;
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Frame for the revolve **arc** gizmo (#262): the axis point nearest the profile centroid
    /// (`center`), the sweep axis (`axis`), the unit radial from the axis to the profile
    /// (`zero_dir`, the 0° direction), and the arc `radius`. The gizmo draws an arc from
    /// `zero_dir` around `axis` through the current sweep angle with a push/pull disc handle at
    /// its far end, dragged around the arc to set the angle.
    fn revolve_arc_geom(&self) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let cr = self.state.creating_revolve.as_ref()?;
        let axis = cr.axis?;
        if cr.faces.is_empty() {
            return None;
        }
        let probe = model::Revolution {
            sketch: cr.sketch?,
            faces: cr.faces.clone(),
            axis,
            angle_deg: 360.0,
            symmetric: false,
            mode: model::RevolveMode::NewBody,
            name: None,
            deleted: false,
        };
        let (origin, dir) = extrude::revolve_axis_world(&self.state.doc, &probe)?;
        let dir = dir.normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            return None;
        }
        let mut centroid = Vec3::ZERO;
        let mut n = 0usize;
        for face in &cr.faces {
            if let Some((profile, _)) = extrude::face_profile_world(&self.state.doc, face) {
                centroid += profile.iter().copied().sum::<Vec3>();
                n += profile.len();
            }
        }
        if n == 0 {
            return None;
        }
        centroid /= n as f32;
        let center = origin + dir * (centroid - origin).dot(dir);
        let radial = centroid - center;
        let radius = radial.length();
        if radius < 1e-4 {
            return None;
        }
        Some((center, dir, radial / radius, radius))
    }

    /// Revolve tool (SPEC §3.5 Revolve): click coplanar profile faces, click an axis line
    /// (construction/projected lines and the global X/Y/Z all work), then set the sweep
    /// angle by dragging the tangent handle or typing (degrees by default, `rad` works).
    /// In Cut mode, clicking bodies toggles them into the cut set. Enter commits.
    #[allow(clippy::too_many_arguments)]
    fn handle_revolve_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        if self.state.sketch_session.is_some() {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_revolve
                .as_ref()
                .is_some_and(|c| !c.faces.is_empty() && c.axis.is_some())
        {
            self.state.apply(Action::CommitRevolve);
            return;
        }
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
        // Arc-handle drag (#262): the cursor's angular position around the arc sets the sweep.
        if self.revolve_gizmo_drag.is_some() {
            if let (Some(pp), Some((center, axis, zero_dir, radius))) =
                (pointer_screen, self.revolve_arc_geom())
            {
                let zero_world = center + zero_dir * radius;
                let sign_probe = revolve_arc_handle_world(center, axis, zero_dir, radius, 5.0);
                if let Some(angle) =
                    revolve_arc_angle_from_cursor(pp, center, zero_world, sign_probe, project)
                {
                    if let Some(cr) = self.state.creating_revolve.as_mut() {
                        cr.angle_live = angle;
                        if !cr.user_edited {
                            cr.text = format!("{angle:.0}");
                        }
                    }
                }
            }
            if primary_pressed {
                self.revolve_gizmo_drag = None;
            }
            return;
        }
        // Grab the arc handle?
        if primary_pressed {
            if let (Some(pp), Some((center, axis, zero_dir, radius))) =
                (pointer_screen, self.revolve_arc_geom())
            {
                let angle = self
                    .state
                    .creating_revolve
                    .as_ref()
                    .map(|c| c.evaluated_angle_deg(&self.state.doc))
                    .unwrap_or(360.0);
                let handle = revolve_arc_handle_world(center, axis, zero_dir, radius, angle);
                if project(handle).is_some_and(|hp| (hp - pp).length() <= REVOLVE_ARC_HANDLE_PICK_PX)
                {
                    self.revolve_gizmo_drag = Some((pp, angle));
                    if let Some(c) = self.state.creating_revolve.as_mut() {
                        c.user_edited = false;
                    }
                    ui.ctx().memory_mut(|m| {
                        m.surrender_focus(egui::Id::new(REVOLVE_ANGLE_FIELD_ID))
                    });
                    return;
                }
            }
        }
        if !primary_pressed {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        // 1) profile faces
        if let Some(face) = pick_extrude_face(
            pp,
            project,
            &self.state.doc,
            self.state.cam.eye(),
            cam,
            viewport,
            vp,
        ) {
            let sketch = actions::extrude_face_sketch(&self.state.doc, &face);
            let cr = self
                .state
                .creating_revolve
                .get_or_insert_with(actions::CreatingRevolve::default);
            if cr.sketch.is_some() && cr.sketch != sketch {
                self.state.status = "Revolve faces must share one sketch".to_string();
                return;
            }
            cr.sketch = sketch;
            if let Some(pos) = cr.faces.iter().position(|f| *f == face) {
                cr.faces.remove(pos);
            } else {
                cr.faces.push(face);
            }
            self.state.status = format!(
                "Revolve: {} face(s){}",
                cr.faces.len(),
                if cr.axis.is_none() { " — click an axis line" } else { "" }
            );
            return;
        }
        // 2) axis line / global axis / 3) cut bodies
        let gp = cam.ground_point(pp, viewport, vp);
        if let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        {
            match target.kind {
                construction::PickTargetKind::Line(li) => {
                    let cr = self
                        .state
                        .creating_revolve
                        .get_or_insert_with(actions::CreatingRevolve::default);
                    cr.axis = Some(model::RevolveAxis::Line(li));
                    self.state.status =
                        "Revolve: axis set — drag the handle or type an angle".to_string();
                    return;
                }
                construction::PickTargetKind::GlobalAxis(axis) => {
                    let cr = self
                        .state
                        .creating_revolve
                        .get_or_insert_with(actions::CreatingRevolve::default);
                    cr.axis = Some(match axis {
                        construction::GlobalAxis::X => model::RevolveAxis::X,
                        construction::GlobalAxis::Y => model::RevolveAxis::Y,
                        construction::GlobalAxis::Z => model::RevolveAxis::Z,
                    });
                    self.state.status =
                        "Revolve: axis set — drag the handle or type an angle".to_string();
                    return;
                }
                ref kind => {
                    // In Cut mode, clicking a body toggles it into the cut set.
                    if let Some(bi) = self.pick_whole_body(pp, project, cam, kind) {
                        if let Some(cr) = self.state.creating_revolve.as_mut() {
                            if cr.body_choice == actions::RevolveBodyChoice::Cut {
                                if let Some(pos) = cr.cut_bodies.iter().position(|b| *b == bi) {
                                    cr.cut_bodies.remove(pos);
                                } else {
                                    cr.cut_bodies.push(bi);
                                }
                                self.state.status =
                                    format!("Revolve: cutting {} body(ies)", cr.cut_bodies.len());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Combine tool (boolean operations): click bodies to toggle them into the active
    /// picker side (A, or B for the two-sided operations); Enter commits.
    #[allow(clippy::too_many_arguments)]
    fn handle_combine_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        if self.state.sketch_session.is_some() {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_boolean
                .as_ref()
                .is_some_and(|c| !c.a.is_empty())
        {
            self.state.apply(Action::CommitBoolean);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let gp = cam.ground_point(pp, viewport, vp);
        let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        else {
            return;
        };
        let Some(bi) = self.pick_whole_body(pp, project, cam, &target.kind) else {
            return;
        };
        if self
            .state
            .doc
            .bodies
            .get(bi)
            .is_some_and(|b| b.shadow)
        {
            self.state.status =
                "That body is already consumed by another operation".to_string();
            return;
        }
        let cb = self
            .state
            .creating_boolean
            .get_or_insert_with(actions::CreatingBoolean::default);
        let to_b = cb.picking_b && cb.kind != model::BooleanOpKind::Combine;
        // A body lives on at most one side; re-clicking it anywhere removes it.
        if let Some(pos) = cb.a.iter().position(|b| *b == bi) {
            cb.a.remove(pos);
        } else if let Some(pos) = cb.b.iter().position(|b| *b == bi) {
            cb.b.remove(pos);
        } else if to_b {
            cb.b.push(bi);
        } else {
            cb.a.push(bi);
        }
        self.state.status = format!(
            "{}: {} body(ies) on A, {} on B",
            cb.kind.label(),
            cb.a.len(),
            cb.b.len()
        );
    }

    /// Move tool (#176/#183): click bodies to toggle them into the move set; clicking a
    /// line picks it as the rotation axis. Enter commits.
    #[allow(clippy::too_many_arguments)]
    /// The Move tool's translation-arrow gizmo geometry (#215): the anchor (picked targets'
    /// bounding-box centre) and, per world axis, `(axis_index, gizmo name, world direction,
    /// current translation mm)`. `None` when nothing is picked (nothing to anchor to).
    fn move_gizmo_arrows(&self) -> Option<(Vec3, [(usize, &'static str, Vec3, f32); 3])> {
        let cm = self.state.creating_move.as_ref()?;
        if cm.targets.is_empty() {
            return None;
        }
        let doc = &self.state.doc;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for &bi in &cm.targets {
            if let Some((lo, hi)) = extrude::body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                min = min.min(lo);
                max = max.max(hi);
            }
        }
        if !min.is_finite() || !max.is_finite() {
            return None;
        }
        let mm = |s: &str| crate::value::eval_length_mm_in_doc(s, doc).unwrap_or(0.0);
        Some((
            (min + max) * 0.5,
            [
                (0, "move_x", Vec3::X, mm(&cm.tx)),
                (1, "move_y", Vec3::Y, mm(&cm.ty)),
                (2, "move_z", Vec3::Z, mm(&cm.tz)),
            ],
        ))
    }

    /// The whole body a viewport pick refers to (#218), for the body-set tools: a body edge,
    /// vertex, or face all identify their owning body. Used with a `pick_body_face` fallback so
    /// clicking anywhere on a body — edge, corner, or flat face — selects it.
    fn pick_whole_body(
        &self,
        pp: egui::Pos2,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        cam: &camera::Camera,
        target_kind: &construction::PickTargetKind,
    ) -> Option<usize> {
        body_index_from_pick(target_kind).or_else(|| {
            crate::face::pick_body_face(pp, project, &self.state.doc, cam.eye())
                .as_ref()
                .and_then(body_index_from_pick)
        })
    }

    /// The Move rotation-ring gizmo geometry (#216): `(centre, axis direction, radius)`, once a
    /// rotation axis and at least one body are picked. `None` otherwise.
    fn move_rotation_geom(&self) -> Option<(Vec3, Vec3, f32)> {
        let cm = self.state.creating_move.as_ref()?;
        let axis = cm.axis?;
        let (center, _) = self.move_gizmo_arrows()?; // shares the picked-targets centroid
        let doc = &self.state.doc;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for &bi in &cm.targets {
            if let Some((lo, hi)) = extrude::body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                min = min.min(lo);
                max = max.max(hi);
            }
        }
        let radius = ((max - min).length() * 0.5).max(1.0) * 1.15;
        let (_, dir) = extrude::axis_world(doc, axis)?;
        let dir = dir.normalize_or_zero();
        (dir != Vec3::ZERO).then_some((center, dir, radius))
    }

    /// The single selected tracing image (#424), if the selection is exactly one image.
    fn single_selected_tracing_image(&self) -> Option<usize> {
        let mut only = None;
        for element in self.state.scene_selection.iter() {
            match element {
                SceneElement::Image(i) if only.is_none() => only = Some(i),
                _ => return None,
            }
        }
        only.filter(|&i| self.state.doc.tracing_images.get(i).is_some_and(|t| !t.deleted))
    }

    /// The single selected sketch text (#286), if the selection is exactly one text: what the
    /// context text editor and the Move tool's text rotation ring operate on.
    fn single_selected_sketch_text(&self) -> Option<usize> {
        let mut only = None;
        for element in self.state.scene_selection.iter() {
            match element {
                SceneElement::SketchText(i) if only.is_none() => only = Some(i),
                _ => return None,
            }
        }
        only.filter(|&i| self.state.doc.sketch_texts.get(i).is_some_and(|t| !t.deleted))
    }

    /// Rotation-ring geometry for the selected sketch text (#286): `(text index, ring centre,
    /// sketch-plane normal, radius)`. The ring sits in the text's sketch plane, centred on the
    /// text origin — the point `rotation` turns about.
    fn text_rotation_geom(&self) -> Option<(usize, Vec3, Vec3, f32)> {
        let index = self.single_selected_sketch_text()?;
        let text = self.state.doc.sketch_texts.get(index)?;
        let frame = sketch_geometry_frame(&self.state.doc, text.sketch)?;
        let center = local_to_world(&frame, text.origin.0, text.origin.1);
        // Contours are baseline-relative (origin at (0,0)), so the farthest outline point puts
        // the ring just outside the glyphs.
        let mut reach: f32 = 0.0;
        for contour in &text.contours {
            for &(u, v) in contour {
                reach = reach.max(u.hypot(v));
            }
        }
        let radius = reach.max(1.0) * 1.15;
        Some((index, center, frame.normal, radius))
    }

    /// The in-sketch Move gizmo (#306): centred at the selected geometry's bbox centre on the
    /// active sketch plane. Returns `(centre_uv, frame)` when the Move tool is active in a
    /// sketch with a movable selection. The gizmo has a free-drag centre plus u/v arrows.
    fn sketch_move_gizmo(&self) -> Option<((f32, f32), face::SketchFrame)> {
        let session = self.state.sketch_session?;
        let frame = sketch_geometry_frame(&self.state.doc, session.sketch)?;
        let mut min = (f32::INFINITY, f32::INFINITY);
        let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut acc = |u: f32, v: f32| {
            min = (min.0.min(u), min.1.min(v));
            max = (max.0.max(u), max.1.max(v));
        };
        for element in self.state.scene_selection.iter() {
            match element {
                SceneElement::Line(li) => {
                    if let Some(l) =
                        self.state.doc.lines.get(li).filter(|l| !l.deleted && l.sketch == session.sketch)
                    {
                        acc(l.x0, l.y0);
                        acc(l.x1, l.y1);
                    }
                }
                SceneElement::Circle(ci) => {
                    if let Some(c) = self
                        .state
                        .doc
                        .circles
                        .get(ci)
                        .filter(|c| !c.deleted && c.sketch == session.sketch)
                    {
                        acc(c.cx, c.cy);
                    }
                }
                SceneElement::SketchText(ti) => {
                    if let Some(t) = self
                        .state
                        .doc
                        .sketch_texts
                        .get(ti)
                        .filter(|t| !t.deleted && t.sketch == session.sketch)
                    {
                        acc(t.origin.0, t.origin.1);
                    }
                }
                _ => {}
            }
        }
        if !min.0.is_finite() {
            return None;
        }
        Some((((min.0 + max.0) * 0.5, (min.1 + max.1) * 0.5), frame))
    }

    /// The in-sketch Move gizmo (#306): a free-drag centre handle plus horizontal/vertical
    /// push-pull arrows that constrain the move to a sketch axis. Runs before the Move tool's
    /// sketch-session gate. Returns true if it consumed the interaction this frame.
    fn handle_sketch_move_gizmo(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
    ) -> bool {
        let Some(session) = self.state.sketch_session else {
            self.sketch_move_drag = None;
            return false;
        };
        let Some((center_uv, frame)) = self.sketch_move_gizmo() else {
            if self.sketch_move_drag.is_some() {
                self.sketch_move_drag = None;
            }
            return false;
        };
        let cursor_uv = |pp: egui::Pos2| {
            sketch_plane_point(cam, viewport, vp, &self.state.doc, session, pp)
                .map(|w| world_to_local(&frame, w))
        };

        // Follow an in-flight drag.
        if let Some(drag) = self.sketch_move_drag {
            if ui.input(|i| i.pointer.primary_down()) {
                if let Some((mut u, mut v)) = pointer_screen.and_then(cursor_uv) {
                    // Axis arrows lock the free coordinate to the grab point.
                    match drag.axis {
                        Some(0) => v = drag.anchor_uv.1,
                        Some(1) => u = drag.anchor_uv.0,
                        _ => {}
                    }
                    self.state.apply(Action::DragSelection { u, v });
                }
            } else {
                self.state.apply(Action::EndSelectionDrag);
                self.sketch_move_drag = None;
            }
            return true;
        }

        // Grab a handle on press.
        if ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(pp) = pointer_screen {
                let center_w = local_to_world(&frame, center_uv.0, center_uv.1);
                let handle_len = SKETCH_MOVE_ARROW_MM;
                let u_tip = local_to_world(&frame, center_uv.0 + handle_len, center_uv.1);
                let v_tip = local_to_world(&frame, center_uv.0, center_uv.1 + handle_len);
                let hit = |w: Vec3, r: f32| project(w).is_some_and(|sp| (sp - pp).length() <= r);
                // Axis arrow tips first (they sit outside the centre handle), then the centre.
                let axis = if hit(u_tip, 12.0) {
                    Some(0)
                } else if hit(v_tip, 12.0) {
                    Some(1)
                } else if hit(center_w, 14.0) {
                    None
                } else {
                    return false;
                };
                if let Some((u, v)) = cursor_uv(pp) {
                    if matches!(
                        self.state.apply(Action::BeginSelectionDrag { anchor_u: u, anchor_v: v }),
                        actions::ActionResult::Ok
                    ) {
                        self.sketch_move_drag = Some(SketchMoveDrag { axis, anchor_uv: (u, v) });
                    }
                }
                return true;
            }
        }
        false
    }

    fn handle_move_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        // In-sketch selection gizmo (#306): a centred free-drag handle plus u/v arrows to
        // translate the whole selection. Runs before the text ring and the session gate.
        if self.handle_sketch_move_gizmo(ui, project, pointer_screen, cam, viewport, vp) {
            return;
        }
        // Rotation ring for a selected sketch text (#286): the Move tool turns the text about
        // its origin. The context pane's Rotation° field follows automatically since it reads
        // the model each frame. Runs before the sketch-session gate so it works in-sketch too.
        if let Some((index, center, axis, radius)) = self.text_rotation_geom() {
            let cursor_angle =
                |pp: egui::Pos2| project(center).map(|c| (pp.y - c.y).atan2(pp.x - c.x));
            // Screen angles run clockwise (y grows downward); sketch rotation runs
            // counter-clockwise around the plane normal. Viewing the front of the plane
            // flips the apparent direction.
            let sign = if axis.dot(cam.eye() - center) > 0.0 { -1.0 } else { 1.0 };
            if let Some(drag) = self.text_rotation_drag {
                if drag.index == index && ui.input(|i| i.pointer.primary_down()) {
                    if let (Some(angle), Some(existing)) = (
                        pointer_screen.and_then(cursor_angle),
                        self.state.doc.sketch_texts.get(index).cloned(),
                    ) {
                        let rotation =
                            drag.start_rotation + sign * (angle - drag.start_cursor_angle);
                        if rotation != existing.rotation {
                            self.state.apply(Action::EditSketchText {
                                index,
                                text: existing.text,
                                font_family: existing.font_family,
                                bold: existing.bold,
                                italic: existing.italic,
                                underline: existing.underline,
                                size: existing.size,
                                size_expr: existing.size_expr,
                                rotation,
                                wrap_width: existing.wrap_width,
                            });
                        }
                    }
                } else {
                    self.text_rotation_drag = None;
                }
                return;
            }
            if ui.input(|i| i.pointer.primary_pressed()) {
                if let Some(pp) = pointer_screen {
                    if rotation_ring_hit(pp, &project, center, axis, radius) {
                        if let Some(angle) = cursor_angle(pp) {
                            let start_rotation = self
                                .state
                                .doc
                                .sketch_texts
                                .get(index)
                                .map(|t| t.rotation)
                                .unwrap_or(0.0);
                            self.text_rotation_drag = Some(TextRotationDrag {
                                index,
                                start_cursor_angle: angle,
                                start_rotation,
                            });
                            return;
                        }
                    }
                }
            }
        } else if self.text_rotation_drag.is_some() {
            self.text_rotation_drag = None;
        }

        if self.state.sketch_session.is_some() {
            return;
        }

        // Translation-arrow gizmo (#215): follow an in-flight drag, or grab a handle on press.
        // Runs before the body-pick path so grabbing an arrow doesn't also toggle a target.
        if let Some((anchor, axes)) = self.move_gizmo_arrows() {
            if let Some(drag) = self.move_gizmo_drag {
                if ui.input(|i| i.pointer.primary_down()) {
                    if let (Some(pp), Some(&(_, name, dir, _))) =
                        (pointer_screen, axes.get(drag.axis))
                    {
                        let value = construction::offset_from_normal_drag(
                            anchor,
                            dir,
                            project,
                            drag.start_translation,
                            drag.start_screen,
                            pp,
                        );
                        crate::actions::set_gizmo(&mut self.state, name, value);
                    }
                } else {
                    self.move_gizmo_drag = None;
                }
                return;
            }
            if ui.input(|i| i.pointer.primary_pressed()) {
                if let Some(pp) = pointer_screen {
                    for &(axis, _, dir, translation) in &axes {
                        let handle_offset = extrude_gizmo_display_offset(translation);
                        if construction::offset_gizmo_hit(pp, project, anchor, dir, handle_offset) {
                            self.move_gizmo_drag = Some(MoveGizmoDrag {
                                axis,
                                start_translation: translation,
                                start_screen: pp,
                            });
                            return;
                        }
                    }
                }
            }
        }
        if self.move_gizmo_drag.is_some() {
            // Targets changed out from under an active drag; drop it.
            self.move_gizmo_drag = None;
        }

        // Rotation-ring gizmo (#216): the ring turns with the cursor's angle around its centre.
        if let Some((center, axis, radius)) = self.move_rotation_geom() {
            let cursor_angle = |pp: egui::Pos2| {
                project(center).map(|c| (pp.y - c.y).atan2(pp.x - c.x))
            };
            if let Some(drag) = self.move_rotation_drag {
                if ui.input(|i| i.pointer.primary_down()) {
                    if let Some(angle) = pointer_screen.and_then(cursor_angle) {
                        let delta_deg = (angle - drag.start_cursor_angle).to_degrees();
                        let new_deg = drag.start_angle_deg + delta_deg;
                        crate::actions::set_gizmo(&mut self.state, "move_angle", new_deg.to_radians());
                    }
                } else {
                    self.move_rotation_drag = None;
                }
                return;
            }
            if ui.input(|i| i.pointer.primary_pressed()) {
                if let Some(pp) = pointer_screen {
                    if rotation_ring_hit(pp, &project, center, axis, radius) {
                        if let Some(angle) = cursor_angle(pp) {
                            let start_deg = crate::actions::gizmo_value(&self.state, "move_angle")
                                .unwrap_or(0.0)
                                .to_degrees();
                            self.move_rotation_drag = Some(MoveRotationDrag {
                                start_cursor_angle: angle,
                                start_angle_deg: start_deg,
                            });
                            return;
                        }
                    }
                }
            }
        } else if self.move_rotation_drag.is_some() {
            self.move_rotation_drag = None;
        }

        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_move
                .as_ref()
                .is_some_and(|c| !c.targets.is_empty() || !c.plane_targets.is_empty() || !c.image_targets.is_empty())
            && !ui.ctx().wants_keyboard_input()
        {
            self.state.apply(Action::CommitMove);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let gp = cam.ground_point(pp, viewport, vp);
        let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        else {
            return;
        };
        if let construction::PickTargetKind::Line(li) = target.kind {
            if let Some(cm) = self.state.creating_move.as_mut() {
                cm.axis = Some(model::RevolveAxis::Line(li));
                self.state.status = "Move: rotation axis set".to_string();
            }
            return;
        }
        let Some(bi) = self.pick_whole_body(pp, project, cam, &target.kind) else {
            // No body under the cursor: a click on a tracing image's quad toggles the
            // image into the move set (#425) — images previously joined only from the
            // Elements pane.
            if let Some(ii) = self.pick_tracing_image(pp, viewport, vp, cam) {
                let cm = self
                    .state
                    .creating_move
                    .get_or_insert_with(actions::CreatingMove::default);
                if let Some(pos) = cm.image_targets.iter().position(|i| *i == ii) {
                    cm.image_targets.remove(pos);
                } else {
                    cm.image_targets.push(ii);
                }
                self.state.status =
                    format!("Move: {} image(s) picked", cm.image_targets.len());
            }
            return;
        };
        if self.state.doc.bodies.get(bi).is_some_and(|b| b.shadow) {
            self.state.status =
                "That body is already consumed by another operation".to_string();
            return;
        }
        let cm = self
            .state
            .creating_move
            .get_or_insert_with(actions::CreatingMove::default);
        if let Some(pos) = cm.targets.iter().position(|b| *b == bi) {
            cm.targets.remove(pos);
        } else {
            cm.targets.push(bi);
        }
        self.state.status = format!("Move: {} body(ies) picked", cm.targets.len());
    }

    /// The visible tracing image whose quad is under the cursor (#425), nearest plane hit
    /// first.
    fn pick_tracing_image(
        &self,
        pp: egui::Pos2,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        cam: &camera::Camera,
    ) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (ii, img) in self.state.doc.tracing_images.iter().enumerate() {
            if img.deleted
                || !self
                    .state
                    .element_visibility
                    .effective_visible(&self.state.doc, SceneElement::Image(ii))
            {
                continue;
            }
            let Some(frame) =
                face::sketch_frame(&self.state.doc, model::FaceId::ConstructionPlane(img.plane))
            else {
                continue;
            };
            let Some(hit) = cam.ray_plane_hit(pp, viewport, vp, frame.origin, frame.normal)
            else {
                continue;
            };
            let d = hit - frame.origin;
            let (u, v) = (d.dot(frame.u_axis), d.dot(frame.v_axis));
            let (ox, oy) = img.origin;
            if u >= ox && u <= ox + img.width_mm && v >= oy && v <= oy + img.height_mm {
                let dist = (hit - cam.eye()).length();
                if best.is_none_or(|(b, _)| dist < b) {
                    best = Some((dist, ii));
                }
            }
        }
        best.map(|(_, ii)| ii)
    }

    /// Repeat tool (#182): click bodies to toggle them into the repeat set; clicking a
    /// line picks it as the axis. Enter commits.
    #[allow(clippy::too_many_arguments)]
    fn handle_repeat_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        // With a sketch open, the Repeat tool repeats sketch entities instead of bodies (#232).
        if let Some(session) = self.state.sketch_session {
            self.handle_sketch_repeat_tool(ui, project, pointer_screen, session);
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_repeat
                .as_ref()
                .is_some_and(|c| {
                    !c.targets.is_empty()
                        || !c.plane_targets.is_empty()
                        || !c.sketch_targets.is_empty()
                        || !c.extrusion_targets.is_empty()
                })
            && !ui.ctx().wants_keyboard_input()
        {
            self.state.apply(Action::CommitRepeat);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let gp = cam.ground_point(pp, viewport, vp);
        let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        else {
            return;
        };
        if let construction::PickTargetKind::Line(li) = target.kind {
            if let Some(cr) = self.state.creating_repeat.as_mut() {
                cr.axis = model::RevolveAxis::Line(li);
                self.state.status = "Repeat: axis set".to_string();
            }
            return;
        }
        let Some(bi) = self.pick_whole_body(pp, project, cam, &target.kind) else {
            return;
        };
        if self.state.doc.bodies.get(bi).is_some_and(|b| b.shadow) {
            self.state.status =
                "That body is already consumed by another operation".to_string();
            return;
        }
        let cr = self
            .state
            .creating_repeat
            .get_or_insert_with(actions::CreatingRepeat::default);
        if let Some(pos) = cr.targets.iter().position(|b| *b == bi) {
            cr.targets.remove(pos);
        } else {
            cr.targets.push(bi);
        }
        self.state.status = format!("Repeat: {} body(ies) picked", cr.targets.len());
    }

    /// In-sketch Repeat tool (#232): click sketch lines/circles to toggle them into the repeat
    /// set; Shift+click a line to set the repeat **direction** from that edge; Enter commits a
    /// `SketchRepeatOperation` (like the 3D repeat, but in the sketch plane).
    fn handle_sketch_repeat_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        session: SketchSession,
    ) {
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !ui.ctx().wants_keyboard_input()
            && self
                .state
                .creating_sketch_repeat
                .as_ref()
                .is_some_and(|c| c.has_targets())
        {
            let cr = self.state.creating_sketch_repeat.take().unwrap();
            let (dir_u, dir_v) = cr.direction(&self.state.doc);
            let action = match cr.editing {
                Some(op) => Action::EditSketchRepeatOperation {
                    op,
                    line_targets: cr.line_targets,
                    circle_targets: cr.circle_targets,
                    dir_u,
                    dir_v,
                    mode: cr.mode,
                    count: cr.count,
                    spacing: cr.spacing,
                    length: cr.length,
                },
                None => Action::CreateSketchRepeatOperation {
                    sketch: cr.sketch,
                    line_targets: cr.line_targets,
                    circle_targets: cr.circle_targets,
                    dir_u,
                    dir_v,
                    mode: cr.mode,
                    count: cr.count,
                    spacing: cr.spacing,
                    length: cr.length,
                },
            };
            self.state.apply(action);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let shift = ui.input(|i| i.modifiers.shift);
        let Some(target) = resolve_pick_target(pp, project, None, &self.state.doc, None) else {
            return;
        };
        let cr = self
            .state
            .creating_sketch_repeat
            .get_or_insert_with(|| actions::CreatingSketchRepeat::new(session.sketch));
        match target.kind {
            construction::PickTargetKind::Line(li) if shift => {
                cr.dir_line = Some(li);
                self.state.status = "Repeat: direction set from edge".to_string();
            }
            construction::PickTargetKind::Line(li) => {
                if let Some(pos) = cr.line_targets.iter().position(|x| *x == li) {
                    cr.line_targets.remove(pos);
                } else {
                    cr.line_targets.push(li);
                }
                self.state.status = format!(
                    "Repeat: {} entities (Shift+click an edge for direction)",
                    cr.line_targets.len() + cr.circle_targets.len()
                );
            }
            construction::PickTargetKind::Circle(ci) => {
                if let Some(pos) = cr.circle_targets.iter().position(|x| *x == ci) {
                    cr.circle_targets.remove(pos);
                } else {
                    cr.circle_targets.push(ci);
                }
                self.state.status = format!(
                    "Repeat: {} entities (Shift+click an edge for direction)",
                    cr.line_targets.len() + cr.circle_targets.len()
                );
            }
            _ => {}
        }
    }

    /// Slice tool (#181): with the target picker active, click bodies to toggle them into
    /// the slice set; with the cutter picker active, click construction planes or planar
    /// body faces to toggle them as cutters. Enter commits.
    #[allow(clippy::too_many_arguments)]
    fn handle_slice_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        // With a sketch open, the Slice tool slices sketch entities/faces instead of bodies (#238).
        if let Some(session) = self.state.sketch_session {
            self.handle_sketch_slice_tool(ui, project, pointer_screen, cam, viewport, vp, session);
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_slice
                .as_ref()
                .is_some_and(|c| !c.targets.is_empty() && !c.cutters.is_empty())
            && !ui.ctx().wants_keyboard_input()
        {
            self.state.apply(Action::CommitSlice);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let picking_cutter = self
            .state
            .creating_slice
            .as_ref()
            .is_some_and(|c| c.picking_cutter);
        if picking_cutter {
            // A cutter is a construction plane or a planar body face.
            let Some(face) = pick_sketch_face(pp, project, &self.state.doc, cam.eye()) else {
                return;
            };
            let cs = self
                .state
                .creating_slice
                .get_or_insert_with(actions::CreatingSlice::default);
            if let Some(pos) = cs.cutters.iter().position(|c| *c == face) {
                cs.cutters.remove(pos);
            } else {
                cs.cutters.push(face);
            }
            self.state.status = format!("Slice: {} cutter(s) picked", cs.cutters.len());
            return;
        }
        let gp = cam.ground_point(pp, viewport, vp);
        let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        else {
            return;
        };
        let Some(bi) = self.pick_whole_body(pp, project, cam, &target.kind) else {
            return;
        };
        if self.state.doc.bodies.get(bi).is_some_and(|b| b.shadow) {
            self.state.status =
                "That body is already consumed by another operation".to_string();
            return;
        }
        let cs = self
            .state
            .creating_slice
            .get_or_insert_with(actions::CreatingSlice::default);
        if let Some(pos) = cs.targets.iter().position(|b| *b == bi) {
            cs.targets.remove(pos);
        } else {
            cs.targets.push(bi);
        }
        self.state.status = format!("Slice: {} body(ies) picked", cs.targets.len());
    }

    /// In-sketch Slice tool (#238): pick target lines/circles/faces and cutter lines with two
    /// roles (like the Combine tool's side-A/side-B pickers — `picking_cutter` chooses which the
    /// next click feeds). Clicking a line/circle toggles it as a target; clicking empty space
    /// inside a face toggles that face; while picking cutters, a click toggles a cutter line.
    /// Enter commits a `SketchSliceOperation`.
    fn handle_sketch_slice_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        session: SketchSession,
    ) {
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !ui.ctx().wants_keyboard_input()
            && self
                .state
                .creating_sketch_slice
                .as_ref()
                .is_some_and(|c| c.has_targets() && c.has_cutters())
        {
            let cs = self.state.creating_sketch_slice.take().unwrap();
            let action = match cs.editing {
                Some(op) => Action::EditSketchSliceOperation {
                    op,
                    line_targets: cs.line_targets,
                    circle_targets: cs.circle_targets,
                    face_targets: cs.face_targets,
                    cutter_lines: cs.cutter_lines,
                },
                None => Action::CreateSketchSliceOperation {
                    sketch: cs.sketch,
                    line_targets: cs.line_targets,
                    circle_targets: cs.circle_targets,
                    face_targets: cs.face_targets,
                    cutter_lines: cs.cutter_lines,
                },
            };
            self.state.apply(action);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let picking_cutter = self
            .state
            .creating_sketch_slice
            .as_ref()
            .is_some_and(|c| c.picking_cutter);
        let target = resolve_pick_target(pp, project, None, &self.state.doc, None);
        let cs = self
            .state
            .creating_sketch_slice
            .get_or_insert_with(|| actions::CreatingSketchSlice::new(session.sketch));
        if picking_cutter {
            // Cutters are lines only.
            if let Some(construction::PickTargetKind::Line(li)) = target.map(|t| t.kind) {
                toggle(&mut cs.cutter_lines, li);
                self.state.status = format!("Slice: {} cutter line(s)", cs.cutter_lines.len());
            }
            return;
        }
        match target.map(|t| t.kind) {
            Some(construction::PickTargetKind::Line(li)) => {
                toggle(&mut cs.line_targets, li);
                self.state.status = format!("Slice: {} line target(s)", cs.line_targets.len());
            }
            Some(construction::PickTargetKind::Circle(ci)) => {
                toggle(&mut cs.circle_targets, ci);
                self.state.status = format!("Slice: {} circle target(s)", cs.circle_targets.len());
            }
            _ => {
                // Empty space: pick the face whose interior is under the cursor.
                if let Some(world) =
                    sketch_plane_point(cam, viewport, vp, &self.state.doc, session, pp)
                {
                    if let Some(loop_lines) =
                        face_loop_at_world(&self.state.doc, session.sketch, world)
                    {
                        if let Some(pos) = cs.face_targets.iter().position(|f| *f == loop_lines) {
                            cs.face_targets.remove(pos);
                        } else {
                            cs.face_targets.push(loop_lines);
                        }
                        self.state.status =
                            format!("Slice: {} face target(s)", cs.face_targets.len());
                    }
                }
            }
        }
    }

    /// If the cursor is over a sketch text's glyph (#285), return that text's glyph faces (one
    /// `ExtrudeFace::TextGlyph` per glyph) — what the Extrude tool toggles to extrude/cut the
    /// whole string. Hit-tests each text on its own sketch plane.
    fn text_glyph_faces_at(
        &self,
        pp: egui::Pos2,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
    ) -> Option<Vec<model::ExtrudeFace>> {
        for (ti, text) in self.state.doc.sketch_texts.iter().enumerate() {
            if text.deleted {
                continue;
            }
            let Some(frame) = crate::face::sketch_geometry_frame(&self.state.doc, text.sketch) else {
                continue;
            };
            let Some(world) = cam.ray_plane_hit(pp, viewport, vp, frame.origin, frame.normal) else {
                continue;
            };
            let (wu, wv) = world_to_local(&frame, world);
            // Undo the text's origin/rotation to test against baseline-space glyph regions.
            let (sin, cos) = text.rotation.sin_cos();
            let (du, dv) = (wu - text.origin.0, wv - text.origin.1);
            let local = (du * cos + dv * sin, -du * sin + dv * cos);
            let regions = crate::text::group_glyphs(&text.contours);
            let hit = regions.iter().any(|r| {
                crate::polygon::point_in_polygon_2d(local, &r.outer)
                    && !r.holes.iter().any(|h| crate::polygon::point_in_polygon_2d(local, h))
            });
            if hit {
                return Some(
                    (0..regions.len())
                        .map(|glyph| model::ExtrudeFace::TextGlyph { text: ti, glyph })
                        .collect(),
                );
            }
        }
        None
    }

    /// Text tool (#282): click in a sketch to drop a text element. Its glyph outlines are baked
    /// from a default system font; the string, font, size, and style are then editable in the
    /// context pane (#286). The new element is selected so its context control opens immediately.
    /// A drag narrower than `TEXT_DRAG_MIN_WIDTH_MM` counts as a click.
    fn handle_text_tool(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
    ) {
        let Some(session) = self.state.sketch_session else {
            self.text_tool_anchor = None;
            return;
        };
        let Some(frame) = crate::face::sketch_geometry_frame(&self.state.doc, session.sketch) else {
            return;
        };
        let uv_at = |pp: egui::Pos2, this: &Self| {
            sketch_plane_point(cam, viewport, vp, &this.state.doc, session, pp)
                .map(|w| world_to_local(&frame, w))
        };

        // Press anchors the placement (#282): a drag from here defines a wrap width, a click
        // (release near the anchor) makes a growing textbox.
        if ui.input(|i| i.pointer.primary_pressed()) {
            self.text_tool_anchor = pointer_screen.and_then(|pp| uv_at(pp, self));
            return;
        }
        if !ui.input(|i| i.pointer.primary_released()) {
            // While the press is held, rubber-band the dragged text box as a dashed
            // rectangle so the drag-to-wrap gesture is visible (#407).
            if let (true, Some((au, av)), Some((ru, rv))) = (
                ui.input(|i| i.pointer.primary_down()),
                self.text_tool_anchor,
                pointer_screen.and_then(|pp| uv_at(pp, self)),
            ) {
                if (ru - au).abs() >= TEXT_DRAG_MIN_WIDTH_MM {
                    let corners = [(au, av), (ru, av), (ru, rv), (au, rv)];
                    for i in 0..4 {
                        let (u0, v0) = corners[i];
                        let (u1, v1) = corners[(i + 1) % 4];
                        draw_world_segment_dashed(
                            painter,
                            project,
                            local_to_world(&frame, u0, v0),
                            local_to_world(&frame, u1, v1),
                            construction::PICK_HOVER_RGBA,
                            1.2,
                        );
                    }
                }
            }
            return;
        }
        let Some((au, av)) = self.text_tool_anchor.take() else {
            return;
        };
        let (u, v, wrap_width) = match pointer_screen.and_then(|pp| uv_at(pp, self)) {
            // A drag wide enough (in mm) wraps to that width; the origin is the left edge and
            // the top of the dragged box (text grows downward from the baseline of line 1).
            Some((ru, rv)) if (ru - au).abs() >= TEXT_DRAG_MIN_WIDTH_MM => {
                (au.min(ru), av.max(rv), Some((ru - au).abs()))
            }
            _ => (au, av, None),
        };
        let Some(family) = default_text_font() else {
            self.state.status = "No usable system font found for text".to_string();
            return;
        };
        let before = self.state.doc.sketch_texts.len();
        self.state.apply(Action::CreateSketchText {
            sketch: session.sketch,
            text: "Text".to_string(),
            font_family: family,
            bold: false,
            italic: false,
            underline: false,
            size: 10.0,
            size_expr: "10".to_string(),
            origin: (u, v),
            rotation: 0.0,
            wrap_width,
        });
        // Select the new text so its context editor opens right away.
        if self.state.doc.sketch_texts.len() > before {
            let idx = self.state.doc.sketch_texts.len() - 1;
            self.state.apply(Action::ClickSceneElement {
                element: SceneElement::SketchText(idx),
                additive: false,
            });
        }
    }

    /// Floating angle field for the in-progress revolve (Enter commits). Mirrors the
    /// extrude distance input.
    fn show_revolve_angle_input(&mut self, ui: &egui::Ui, project: &impl Fn(Vec3) -> Option<egui::Pos2>) {
        let Some((center, axis, zero_dir, radius)) = self.revolve_arc_geom() else {
            return;
        };
        let angle = self
            .state
            .creating_revolve
            .as_ref()
            .map(|c| c.evaluated_angle_deg(&self.state.doc))
            .unwrap_or(360.0);
        let handle = revolve_arc_handle_world(center, axis, zero_dir, radius, angle);
        let Some(pos) = project(handle).map(|p| p + egui::vec2(14.0, -12.0)) else {
            return;
        };
        let mut commit = false;
        egui::Area::new(egui::Id::new("revolve_angle_input"))
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let Some(cr) = self.state.creating_revolve.as_mut() else {
                            return;
                        };
                        let field = egui::TextEdit::singleline(&mut cr.text)
                            .id(egui::Id::new(REVOLVE_ANGLE_FIELD_ID))
                            .desired_width(64.0)
                            .hint_text("360");
                        let response = ui.add(field);
                        if cr.pending_focus {
                            response.request_focus();
                            cr.pending_focus = false;
                        }
                        if response.changed() {
                            cr.user_edited = true;
                        }
                        ui.label("deg");
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.lost_focus() {
                            commit = true;
                        }
                    });
                });
            });
        if commit {
            self.state.apply(Action::CommitRevolve);
        }
    }

    /// Loft tool (SPEC §3.5): click closed sketch profiles (circles or line loops) to
    /// collect cross sections; Enter blends them into a lofted solid. The picked set shows
    /// in the context-pane selection picker (#167), where rows can be removed.
    fn handle_loft_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
        cam: &camera::Camera,
        viewport: egui::Rect,
        vp: &glam::Mat4,
        pick_occlusion: Option<&construction::PickOcclusion>,
    ) {
        if self.state.sketch_session.is_some() {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter))
            && self
                .state
                .creating_loft
                .as_ref()
                .is_some_and(|cl| cl.sections.len() >= 2)
        {
            self.state.apply(Action::CommitLoft);
            return;
        }
        if !ui.input(|i| i.pointer.primary_pressed()) {
            return;
        }
        let Some(pp) = pointer_screen else {
            return;
        };
        let gp = cam.ground_point(pp, viewport, vp);
        let Some(target) = resolve_pick_target(pp, project, gp, &self.state.doc, pick_occlusion)
        else {
            return;
        };
        let element = match target.kind {
            construction::PickTargetKind::Circle(ci) => Some(SceneElement::Circle(ci)),
            construction::PickTargetKind::Line(li) => Some(SceneElement::Line(li)),
            _ => None,
        };
        if let Some(section) =
            element.and_then(|el| extrude::loft_section_from_element(&self.state.doc, el))
        {
            self.state.apply(Action::ToggleLoftSection { section });
        }
    }

    fn handle_edge_treatment_tool(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        pointer_screen: Option<egui::Pos2>,
    ) {
        if self.state.sketch_session.is_some() {
            self.state.creating_edge_treatment = None;
            self.edge_treatment_gizmo_drag = None;
            return;
        }
        let kind = match self.state.tool {
            Tool::Chamfer => VertexTreatmentKind::Chamfer,
            Tool::Fillet => VertexTreatmentKind::Fillet,
            _ => return,
        };
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

        // If the in-progress treatment went away (committed or cancelled), stop following.
        if self.state.creating_edge_treatment.is_none() {
            self.edge_treatment_gizmo_drag = None;
        }

        let anchor = self.state.creating_edge_treatment.as_ref().and_then(|cet| {
            let (extrusion, edge) = cet.primary()?;
            crate::extrude::extrusion_edge_anchor(&self.state.doc, extrusion, edge)
        });

        let following = self.edge_treatment_gizmo_drag.is_some();
        let mut gizmo_active = false;
        if let Some((origin, normal)) = anchor {
            let amount = self
                .state
                .creating_edge_treatment
                .as_ref()
                .map(|cet| cet.evaluated_amount(&self.state.doc))
                .unwrap_or(0.0);
            let handle_offset = construction::gizmo_display_offset(amount);
            let hovered = pointer_screen.is_some_and(|pp| {
                construction::offset_gizmo_hit(pp, project, origin, normal, handle_offset)
            });
            if !following && primary_pressed && hovered {
                if let Some(pp) = pointer_screen {
                    self.edge_treatment_gizmo_drag = Some(EdgeTreatmentGizmoDrag {
                        start_screen: pp,
                        start_amount: amount,
                    });
                    if let Some(cet) = self.state.creating_edge_treatment.as_mut() {
                        cet.user_edited = false;
                    }
                    ui.ctx().memory_mut(|m| {
                        m.surrender_focus(egui::Id::new(EDGE_TREATMENT_AMOUNT_FIELD_ID))
                    });
                }
            }
            if let Some(drag) = self.edge_treatment_gizmo_drag {
                gizmo_active = true;
                if let Some(pp) = pointer_screen {
                    let new_amount = construction::offset_from_normal_drag(
                        origin,
                        normal,
                        project,
                        drag.start_amount,
                        drag.start_screen,
                        pp,
                    )
                    .max(0.0);
                    if let Some(cet) = self.state.creating_edge_treatment.as_mut() {
                        cet.amount_live = new_amount;
                        if !cet.user_edited {
                            cet.text = crate::value::format_length_display(new_amount);
                        }
                    }
                }
            }
        }

        // A click while following commits the treatment set.
        if following && primary_pressed {
            if let Some(mut cet) = self.state.creating_edge_treatment.take() {
                // #201: a typed amount can define a parameter (`name = expr`).
                let _ = actions::commit_inline_parameter_defs(&mut self.state.doc, [&mut cet.text]);
                let amount = cet.evaluated_amount(&self.state.doc);
                self.state.apply(Action::CommitEdgeTreatments {
                    edges: cet.edges.clone(),
                    kind: cet.kind,
                    amount,
                });
            }
            self.edge_treatment_gizmo_drag = None;
            return;
        }
        if gizmo_active {
            return;
        }

        // Click a treatable analytic edge (vertical or side/cap) to begin; with a treatment
        // already in progress, shift/⌘+click toggles the edge in the set (#166) and a plain
        // click on another edge restarts with just that edge.
        if primary_pressed {
            if let Some(pp) = pointer_screen {
                if let Some((extrusion, edge, _, _, _)) =
                    construction::nearest_treatable_edge(pp, project, &self.state.doc)
                {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    match self.state.creating_edge_treatment.as_mut() {
                        Some(cet) if additive => cet.toggle_edge((extrusion, edge)),
                        _ => {
                            self.state.creating_edge_treatment = Some(CreatingEdgeTreatment {
                                edges: vec![(extrusion, edge)],
                                kind,
                                amount_live: DEFAULT_VERTEX_TREATMENT_AMOUNT,
                                text: crate::value::format_length_display(
                                    DEFAULT_VERTEX_TREATMENT_AMOUNT,
                                ),
                                user_edited: false,
                                pending_focus: true,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Floating amount field for the in-progress 3D edge chamfer/fillet (Enter commits).
    /// Mirrors [`Self::show_vertex_treatment_amount_input`].
    fn show_edge_treatment_amount_input(
        &mut self,
        ui: &egui::Ui,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let pos = {
            let Some(cet) = self.state.creating_edge_treatment.as_ref() else {
                return;
            };
            let Some((extrusion, edge)) = cet.primary() else {
                return;
            };
            let Some((origin, normal)) =
                crate::extrude::extrusion_edge_anchor(&self.state.doc, extrusion, edge)
            else {
                return;
            };
            let handle_offset =
                construction::gizmo_display_offset(cet.evaluated_amount(&self.state.doc));
            project(construction::offset_handle(origin, normal, handle_offset))
                .map(|p| p + egui::vec2(14.0, -12.0))
        };
        let Some(pos) = pos else {
            return;
        };
        let ctx = ui.ctx();
        let id = egui::Id::new(EDGE_TREATMENT_AMOUNT_FIELD_ID);
        let mut commit = false;

        if !ctx.memory(|m| m.has_focus(id)) && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            commit = true;
        }

        if !ctx.memory(|m| m.has_focus(id)) {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            let typed: String = typed
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !typed.is_empty() {
                if let Some(cet) = self.state.creating_edge_treatment.as_mut() {
                    cet.text = typed;
                    cet.user_edited = true;
                    cet.pending_focus = true;
                }
            }
        }
        if let Some(cet) = self.state.creating_edge_treatment.as_mut() {
            let label = match cet.kind {
                VertexTreatmentKind::Chamfer => "mm",
                VertexTreatmentKind::Fillet => "mm r",
            };
            let want_focus = cet.pending_focus;
            egui::Area::new(egui::Id::new("edge_treatment_amount_area"))
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut cet.text)
                                .id(id)
                                .desired_width(64.0),
                        );
                        if resp.changed() {
                            cet.user_edited = true;
                        }
                        if want_focus {
                            resp.request_focus();
                            cet.pending_focus = false;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit = true;
                        }
                        ui.label(label);
                    });
                });
        }
        if commit {
            if let Some(mut cet) = self.state.creating_edge_treatment.take() {
                // #201: a typed amount can define a parameter (`name = expr`).
                let _ = actions::commit_inline_parameter_defs(&mut self.state.doc, [&mut cet.text]);
                let amount = cet.evaluated_amount(&self.state.doc);
                self.state.apply(Action::CommitEdgeTreatments {
                    edges: cet.edges.clone(),
                    kind: cet.kind,
                    amount,
                });
            }
        }
    }

    fn tick_exit_after_startup(&mut self, ctx: &egui::Context) {
        if !self.exit_after_startup || self.exit_after_startup_sent {
            return;
        }
        if self.launch_maximize_frames_remaining > 0 {
            return;
        }
        self.exit_after_startup_sent = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn tick_script(&mut self, ctx: &egui::Context) {
        if self.script.as_ref().is_some_and(|r| !r.done) {
            self.state.command_log = None;
        } else if self.state.command_log.is_none() {
            self.state.command_log = Some(std::cell::RefCell::new(
                command_log::CommandLog::new_recording(self.show_commands),
            ));
        }
        let needs_repaint = if let Some(runner) = &mut self.script {
            if runner.done {
                if let Some(err) = &runner.error {
                    self.state.status = format!("Script error: {err}");
                } else if !runner.should_quit {
                    let complete_status = if runner.is_repl() { "REPL ended" } else { "Script complete" };
                    self.state.status = complete_status.to_string();
                }
                let action = script_finished_close_action(
                    runner.error.is_some(),
                    runner.should_quit,
                    self.exit_on_script_complete,
                );
                if action.fail_process {
                    self.script_failed.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if action.close {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                false
            } else {
                let repaint = runner.tick(
                    &mut self.state,
                    &mut self.synthetic,
                    self.last_viewport,
                    ctx,
                );
                if let Some(err) = &runner.error {
                    self.state.status = format!("Script error: {err}");
                }
                repaint
            }
        } else {
            false
        };

        if needs_repaint || self.script.as_ref().is_some_and(|r| r.is_waiting()) {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        tick_launch_maximize(&mut self.launch_maximize_frames_remaining, ctx);
        theme::apply(ctx);

        let dt = ctx.input(|i| i.stable_dt);
        let transition_active = self.state.cam.tick_transition(dt);
        if transition_active {
            ctx.request_repaint();
        } else if let Some(log) = &self.state.command_log {
            log.borrow_mut()
                .on_transition_complete(&self.state.cam);
        }

        self.process_screenshots(ctx);
        self.tick_script(ctx);
        self.tick_exit_after_startup(ctx);
        self.synthetic.inject(ctx);

        self.tick_fps_mode(ctx, dt);
        self.handle_keyboard_shortcuts(ctx);

        // Aligned-view tool base (#365): on entering the tool, seed its base from a lone selected
        // projection (so you needn't re-pick it); leaving the tool clears it.
        if self.state.tool == Tool::DrawingAlign {
            if self.prev_tool != Tool::DrawingAlign {
                self.drawing_align_parent = match self.state.selected_drawing_view() {
                    Some((d, v)) if Some(d) == self.state.editing_drawing => Some(v),
                    _ => None,
                };
            }
        } else {
            self.drawing_align_parent = None;
        }
        self.prev_tool = self.state.tool;

        #[cfg(not(target_arch = "wasm32"))]
        self.handle_native_menu(ctx);
        #[cfg(target_arch = "wasm32")]
        {
            let panes = self.state.panes.clone();
            if let Some(command) = web_menu::bar(ctx, |pane| panes.is_visible(pane)) {
                self.handle_menu_command(ctx, command);
            }
            self.drain_web_io(ctx);
        }

        egui::TopBottomPanel::top("toolbar")
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Workbench toolbars (#254/#271): the Drawing workbench (a drawing open) shows
                // only the tools that apply to drawings — Select and Dimension (#295: no Move;
                // the Select tool drags projections directly).
                if self.state.editing_drawing.is_some() {
                    // Back to the model (#318): left of Select, replacing Esc-to-exit. The arrow
                    // is a bundled SVG icon, not a font glyph (#325) — the old "← Back" text
                    // showed an empty box wherever the font lacked the arrow.
                    if ui
                        .add(egui::Button::image_and_text(
                            icons::sized_texture(ui.ctx(), icons::IconId::Back),
                            "Back",
                        ))
                        .on_hover_text("Return to the 3D model")
                        .clicked()
                    {
                        self.state.apply(Action::EditDrawing { drawing: None });
                    }
                    ui.separator();
                    for (icon, tool, label) in [
                        (icons::IconId::Select, Tool::Select, "Select"),
                        (icons::IconId::Plus, Tool::DrawingAdd, "Add view"),
                        (icons::IconId::Projection, Tool::DrawingAlign, "Aligned view"),
                        (icons::IconId::Dimension, Tool::Dimension, "Dimension"),
                        (icons::IconId::Text, Tool::Text, "Text"),
                    ] {
                        if icons::selectable_icon_button(
                            ui,
                            icon,
                            self.state.tool == tool,
                            shortcuts::compact_label(label, shortcuts::tool_shortcut(tool)),
                        )
                        .clicked()
                        {
                            self.state.apply(Action::SetTool(tool));
                        }
                    }
                    ui.separator();
                    if icons::selectable_icon_button(ui, icons::IconId::Zoom, false, "Zoom to fit (Z)")
                        .clicked()
                    {
                        // In the Drawing workbench, fit = reset the page pan/zoom (#273/#279).
                        self.drawing_zoom = 1.0;
                        self.drawing_pan = egui::Vec2::ZERO;
                    }
                    // Export (#348): one toolbar icon whose popup picks the format.
                    if let Some(dwg) = self.state.editing_drawing {
                        ui.separator();
                        let tex = icons::sized_texture(ui.ctx(), icons::IconId::Export);
                        ui.menu_image_button(tex, |ui| {
                            if ui.button("Export SVG…").clicked() {
                                self.export_drawing_svg(dwg);
                                ui.close();
                            }
                            if ui.button("Export PDF…").clicked() {
                                self.export_drawing_pdf(dwg);
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text("Export the drawing (SVG or PDF)");
                    }
                    return;
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Select,
                    self.state.tool == Tool::Select,
                    shortcuts::compact_label("Select", shortcuts::tool_shortcut(Tool::Select)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Select));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Sketch,
                    self.state.tool == Tool::Sketch,
                    shortcuts::compact_label("Sketch", shortcuts::tool_shortcut(Tool::Sketch)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Sketch));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Rectangle,
                    self.state.tool == Tool::Rectangle,
                    shortcuts::compact_label(
                        "Rectangle",
                        shortcuts::tool_shortcut(Tool::Rectangle),
                    ),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Rectangle));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Line,
                    self.state.tool == Tool::Line,
                    shortcuts::compact_label("Line", shortcuts::tool_shortcut(Tool::Line)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Line));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Circle,
                    self.state.tool == Tool::Circle,
                    shortcuts::compact_label("Circle", shortcuts::tool_shortcut(Tool::Circle)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Circle));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Fillet,
                    self.state.tool == Tool::Fillet,
                    shortcuts::compact_label("Fillet", shortcuts::tool_shortcut(Tool::Fillet)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Fillet));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Chamfer,
                    self.state.tool == Tool::Chamfer,
                    shortcuts::compact_label("Chamfer", shortcuts::tool_shortcut(Tool::Chamfer)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Chamfer));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Text,
                    self.state.tool == Tool::Text,
                    shortcuts::compact_label("Text", shortcuts::tool_shortcut(Tool::Text)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Text));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Plane,
                    self.state.tool == Tool::ConstructionPlane,
                    shortcuts::compact_label(
                        "Plane",
                        shortcuts::tool_shortcut(Tool::ConstructionPlane),
                    ),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::ConstructionPlane));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Extrude,
                    self.state.tool == Tool::Extrude,
                    shortcuts::compact_label("Extrude", shortcuts::tool_shortcut(Tool::Extrude)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Extrude));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Loft,
                    self.state.tool == Tool::Loft,
                    shortcuts::compact_label("Loft", shortcuts::tool_shortcut(Tool::Loft)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Loft));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Revolve,
                    self.state.tool == Tool::Revolve,
                    shortcuts::compact_label("Revolve", shortcuts::tool_shortcut(Tool::Revolve)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Revolve));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Combine,
                    self.state.tool == Tool::Combine,
                    shortcuts::compact_label("Combine", shortcuts::tool_shortcut(Tool::Combine)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Combine));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Move,
                    self.state.tool == Tool::Move,
                    shortcuts::compact_label("Move", shortcuts::tool_shortcut(Tool::Move)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Move));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Repeat,
                    self.state.tool == Tool::Repeat,
                    shortcuts::compact_label("Repeat", shortcuts::tool_shortcut(Tool::Repeat)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Repeat));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Slice,
                    self.state.tool == Tool::Slice,
                    shortcuts::compact_label("Slice", shortcuts::tool_shortcut(Tool::Slice)),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Slice));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Dimension,
                    self.state.tool == Tool::Dimension,
                    shortcuts::compact_label(
                        "Dimension",
                        shortcuts::tool_shortcut(Tool::Dimension),
                    ),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Dimension));
                }
                if icons::selectable_icon_button(
                    ui,
                    icons::IconId::Constraint,
                    self.state.tool == Tool::Constraint,
                    shortcuts::compact_label(
                        "Constraint",
                        shortcuts::tool_shortcut(Tool::Constraint),
                    ),
                )
                .clicked()
                {
                    self.state.apply(Action::SetTool(Tool::Constraint));
                }
                ui.separator();
                if icons::selectable_icon_button(ui, icons::IconId::Zoom, false, "Zoom to fit (Z)")
                    .clicked()
                {
                    self.state.apply(Action::ZoomToFit);
                }
                // Import/Export toolbar buttons (#352): the same actions as the File menu, grouped
                // under a popup on each icon.
                ui.separator();
                ui.menu_image_button(icons::sized_texture(ui.ctx(), icons::IconId::Import), |ui| {
                    if ui.button("Import STL…").clicked() {
                        self.import_stl();
                        ui.close();
                    }
                    if ui.button("Import STEP…").clicked() {
                        self.import_step();
                        ui.close();
                    }
                    if ui.button("Import Image…").clicked() {
                        self.import_image();
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Import STL, STEP, or an image");
                ui.menu_image_button(icons::sized_texture(ui.ctx(), icons::IconId::Export), |ui| {
                    if ui.button("Export STL…").clicked() {
                        self.export_stl_all();
                        ui.close();
                    }
                    if ui.button("Export STEP…").clicked() {
                        self.export_step_all();
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Export all bodies as STL or STEP");
                if let Some(session) = self.state.sketch_session {
                    ui.separator();
                    ui.label(sketch_label(&self.state.doc, session.sketch));
                }
                ui.separator();
                if ui.button("Clear").clicked() {
                    self.state.apply(Action::Clear);
                }
            });
        });

        self.show_json_dialog(ctx);

        if self.state.command_palette.open {
            let commands = commands_for_state(&self.state);
            let matches = filter_commands(&self.state.command_palette.query, &commands);
            let mut outcome = None;
            egui::TopBottomPanel::bottom("command_palette")
                .resizable(false)
                .exact_height(280.0)
                .frame(
                    egui::Frame::default()
                        .fill(theme::palette_console_fill())
                        .inner_margin(egui::Margin::symmetric(12, 8)),
                )
                .show(ctx, |ui| {
                    outcome = show_palette(ui, &mut self.state.command_palette, &matches);
                });
            if let Some(chosen) = outcome {
                self.dispatch_palette_outcome(chosen);
            }
        }

        egui::TopBottomPanel::bottom("status")
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
            let name = self
                .state
                .path
                .clone()
                .unwrap_or_else(|| "(unsaved)".to_string());
            let status = self.state.status.clone();
            ui.horizontal(|ui| {
                ui.label(name);
                ui.separator();
                ui.label(status);
                // Update badge (#427): a bright button in the bottom-right corner when a
                // newer release exists; unobtrusive — no popup, no interruption.
                #[cfg(not(target_arch = "wasm32"))]
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.show_update_badge(ui, ctx);
                });
            });
        });

        if self.state.panes.is_visible(Pane::Hierarchy) {
            // On a Model/Drawing workbench switch, reset the element filter to that workbench's
            // default (Drawing shows only sketches + bodies) (#275).
            let drawing_workbench = self.state.editing_drawing.is_some();
            if drawing_workbench != self.element_filter_drawing_workbench {
                self.element_filter = if drawing_workbench {
                    hierarchy::ElementFilter::for_drawing_workbench()
                } else {
                    hierarchy::ElementFilter::default()
                };
                // The Parameters pane hides by default in the Drawing workbench (#398) — the
                // View menu can still show it (#378) — and its visibility restores on the
                // way back to the model.
                if drawing_workbench {
                    self.params_visible_before_drawing =
                        self.state.panes.is_visible(Pane::Parameters);
                    self.state.panes.set(Pane::Parameters, false);
                } else if self.params_visible_before_drawing {
                    self.state.panes.set(Pane::Parameters, true);
                }
                // Open each drawing fit-to-pane (#273).
                self.drawing_zoom = 1.0;
                self.drawing_pan = egui::Vec2::ZERO;
                self.element_filter_drawing_workbench = drawing_workbench;
            }
            let mut edit_sketch: Option<SketchId> = None;
            let mut edit_plane: Option<usize> = None;
            let mut import_image_on_plane: Option<usize> = None;
            let mut edit_extrusion: Option<usize> = None;
            let mut edit_edge_treatment: Option<(usize, usize)> = None;
            let mut edit_drawing: Option<usize> = None;
            let mut select_drawing_element: Option<hierarchy::HierarchyNode> = None;
            let mut hover_drawing_element: Option<hierarchy::HierarchyNode> = None;
            // Which drawing element is currently selected, as a hierarchy leaf, so its Elements-
            // pane row shows the selected style (#341).
            let selected_drawing_leaf: Option<hierarchy::HierarchyNode> = self
                .state
                .selected_drawing_view()
                .map(|(d, v)| hierarchy::HierarchyNode::DrawingProjection { drawing: d, view: v })
                .or_else(|| {
                    self.state.selected_drawing_annotation().map(|(d, a)| {
                        hierarchy::HierarchyNode::DrawingAnnotation { drawing: d, annotation: a }
                    })
                })
                .or_else(|| {
                    self.state.selected_drawing_dimension().map(|(d, v, a, b)| {
                        hierarchy::HierarchyNode::DrawingDimension { drawing: d, view: v, a, b }
                    })
                });
            let mut export_body: Option<usize> = None;
            let mut export_body_step: Option<usize> = None;
            let mut click_element: Option<(SceneElement, bool)> = None;
            let mut delete_element: Option<SceneElement> = None;
            let mut add_to_drawing: Option<SceneElement> = None;
            let mut rename_drawing: Option<(usize, String)> = None;
            let mut pane_hovered_element: Option<SceneElement> = None;
            let mut add_component: Option<Option<usize>> = None;
            let mut move_to_component: Option<(SceneElement, Option<usize>)> = None;
            egui::SidePanel::left("tree")
                .resizable(true)
                .default_width(220.0)
                .frame(theme::panel_frame())
                .show(ctx, |ui| {
                    let mut queue_edit_sketch = |sketch: SketchId| {
                        edit_sketch = Some(sketch);
                    };
                    let mut queue_edit_plane = |index: usize| {
                        edit_plane = Some(index);
                    };
                    let mut queue_import_image_on_plane = |index: usize| {
                        import_image_on_plane = Some(index);
                    };
                    let mut queue_edit_extrusion = |index: usize| {
                        edit_extrusion = Some(index);
                    };
                    let mut queue_edit_edge_treatment = |extrusion: usize, index: usize| {
                        edit_edge_treatment = Some((extrusion, index));
                    };
                    let mut queue_edit_drawing = |index: usize| {
                        edit_drawing = Some(index);
                    };
                    let mut queue_select_drawing_element = |node: hierarchy::HierarchyNode| {
                        select_drawing_element = Some(node);
                    };
                    let mut queue_hover_drawing_element =
                        |node: Option<hierarchy::HierarchyNode>| {
                            if let Some(n) = node {
                                hover_drawing_element = Some(n);
                            }
                        };
                    let mut queue_rename_drawing = |index: usize, name: String| {
                        rename_drawing = Some((index, name));
                    };
                    let mut queue_export_body = |index: usize| {
                        export_body = Some(index);
                    };
                    let mut queue_export_body_step = |index: usize| {
                        export_body_step = Some(index);
                    };
                    let mut noop_visibility = |_: SceneElement, _: bool| {};
                    let mut queue_click = |element: SceneElement, additive: bool| {
                        click_element = Some((element, additive));
                    };
                    let mut queue_hover = |element: SceneElement| {
                        pane_hovered_element = Some(element);
                    };
                    let mut queue_add_to_drawing = |element: SceneElement| {
                        add_to_drawing = Some(element);
                    };
                    let mut queue_delete = |element: SceneElement| {
                        delete_element = Some(element);
                    };
                    // Highlight the elements that use the variable focused in the Parameters pane.
                    let highlight_elements = parameters::focused_parameter_name(ctx, &self.state.doc)
                        .map(|name| parameters::elements_using_parameter(&self.state.doc, &name))
                        .unwrap_or_default();
                    let mut queue_add_component = |parent: Option<usize>| {
                        add_component = Some(parent);
                    };
                    let mut queue_move_to_component =
                        |element: SceneElement, component: Option<usize>| {
                            move_to_component = Some((element, component));
                        };
                    hierarchy::show_pane(
                        ui,
                        &self.state.doc,
                        self.state.sketch_session,
                        &mut self.state.element_visibility,
                        &self.state.scene_selection,
                        &self.state.document_health,
                        &mut self.state.hierarchy_view_mode,
                        &mut self.graph_layout,
                        &mut self.element_filter,
                        &mut self.element_filter_expanded,
                        &mut queue_edit_sketch,
                        &mut queue_edit_plane,
                        &mut queue_import_image_on_plane,
                        &mut queue_edit_extrusion,
                        &mut queue_edit_edge_treatment,
                        &mut queue_edit_drawing,
                        &mut queue_select_drawing_element,
                        &mut queue_hover_drawing_element,
                        selected_drawing_leaf,
                        &mut queue_rename_drawing,
                        &mut queue_export_body,
                        &mut queue_export_body_step,
                        &mut noop_visibility,
                        &mut queue_click,
                        &mut queue_hover,
                        &mut queue_delete,
                        self.state.editing_drawing,
                        &mut queue_add_to_drawing,
                        &highlight_elements,
                        &mut self.collapsed_components,
                        &mut queue_add_component,
                        &mut queue_move_to_component,
                    );
                });
            self.pane_hovered_element = pane_hovered_element;
            if let Some(parent) = add_component {
                self.state.apply(Action::CreateComponent { name: None, parent });
            }
            if let Some((element, component)) = move_to_component {
                self.state.apply(Action::MoveToComponent { element, component });
            }
            if let Some(element) = delete_element {
                self.state.apply(Action::DeleteElement { element });
            }
            if let (Some(element), Some(drawing)) = (add_to_drawing, self.state.editing_drawing) {
                let orientation = model::DrawingOrientation::default();
                match element {
                    SceneElement::Body(body) => {
                        self.state.apply(Action::AddDrawingView { drawing, body, orientation });
                    }
                    SceneElement::Sketch(sketch) => {
                        self.state
                            .apply(Action::AddDrawingSketchView { drawing, sketch, orientation });
                    }
                    _ => {}
                }
            }
            if let Some((drawing, name)) = rename_drawing {
                self.state.apply(Action::RenameDrawing { drawing, name });
            }
            if let Some((element, additive)) = click_element {
                self.state.apply(Action::ClickSceneElement { element, additive });
            }
            if let Some(sketch) = edit_sketch {
                self.state.apply(Action::OpenSketch {
                    sketch,
                    viewport: self.last_viewport,
                });
            }
            if let Some(index) = import_image_on_plane {
                self.import_image_on_plane(index);
            }
            if let Some(index) = edit_plane {
                self.state.apply(Action::BeginEditConstructionPlane { index });
            }
            if let Some(index) = edit_extrusion {
                self.state.apply(Action::EditExtrusion { index });
            }
            if let Some(drawing) = edit_drawing {
                self.state.apply(Action::EditDrawing { drawing: Some(drawing) });
            }
            // Hovering a drawing-element row highlights it on the page (#341); cleared each frame
            // so it only shows while actually hovering.
            self.state.hovered_drawing_element = hover_drawing_element.and_then(|n| match n {
                hierarchy::HierarchyNode::DrawingProjection { view, .. } => {
                    Some(context::DrawingElementRef::Projection(view))
                }
                hierarchy::HierarchyNode::DrawingAnnotation { annotation, .. } => {
                    Some(context::DrawingElementRef::Text(annotation))
                }
                hierarchy::HierarchyNode::DrawingDimension { view, a, b, .. } => {
                    Some(context::DrawingElementRef::Dimension { view, a, b })
                }
                _ => None,
            });
            // Selecting a drawing element from the Elements pane (#341): mirror the on-page
            // selection so its context editor opens and it highlights.
            if let Some(node) = select_drawing_element {
                use context::DrawingElementRef as R;
                let picked = match node {
                    hierarchy::HierarchyNode::DrawingProjection { drawing, view } => {
                        Some((drawing, R::Projection(view)))
                    }
                    hierarchy::HierarchyNode::DrawingAnnotation { drawing, annotation } => {
                        Some((drawing, R::Text(annotation)))
                    }
                    hierarchy::HierarchyNode::DrawingDimension { drawing, view, a, b } => {
                        Some((drawing, R::Dimension { view, a, b }))
                    }
                    _ => None,
                };
                if let Some((drawing, element)) = picked {
                    // While the Aligned-view tool is active, clicking a projection picks it as the
                    // base to align to (#365) rather than changing the Select tool's selection.
                    if self.state.tool == Tool::DrawingAlign {
                        if let context::DrawingElementRef::Projection(view) = element {
                            if Some(drawing) == self.state.editing_drawing {
                                self.drawing_align_parent = Some(view);
                            }
                        }
                    } else {
                        // Clicking a drawing element in the Elements pane updates the Select tool's
                        // multi-selection (#346), mirroring the model tool: a plain click replaces,
                        // Cmd/Ctrl-click adds/removes.
                        let additive =
                            ctx.input(|i| selection::additive_click_modifiers(&i.modifiers));
                        if additive {
                            self.state.toggle_drawing_element(drawing, element);
                        } else {
                            self.state.select_drawing_only(drawing, element);
                        }
                    }
                }
            }
            if let Some((extrusion, index)) = edit_edge_treatment {
                // Re-open the chamfer/fillet with its push/pull gizmo + amount input (#259).
                self.state.apply(Action::EditEdgeTreatment { extrusion, index });
            }
            if let Some(index) = export_body {
                self.export_stl_body(index);
            }
            if let Some(index) = export_body_step {
                self.export_step_body(index);
            }
        }

        // The Parameters pane follows its View-menu visibility in every workbench — including
        // Drawings (#378), where editing a parameter rebuilds the model and the open drawing's
        // views update live.
        if self.state.panes.is_visible(Pane::Parameters) {
            egui::SidePanel::right("parameters")
                .resizable(true)
                .default_width(240.0)
                .frame(theme::panel_frame())
                .show(ctx, |ui| {
                    parameters::show_pane(ui, &mut self.state);
                });
        }

        if self.state.panes.is_visible(Pane::Context) {
            let context_input = context::ContextInput {
                doc: &self.state.doc,
                selection: &self.state.scene_selection,
                tool: self.state.tool,
                in_drawing_workbench: self.state.editing_drawing.is_some(),
                draw_rect_construction: self.state.rect_draw_construction_mode(),
                draw_line_construction: self.state.line_draw_construction_mode(),
                draw_circle_construction: self.state.circle_draw_construction_mode(),
                draw_line_curve_mode: self.state.line_curve_mode(),
                draw_line_tangent_constraint: self.state.line_tangent_constraint(),
                in_sketch: self.state.sketch_session.is_some(),
                snapping_enabled: self.state.snapping_enabled,
                extrude_merge_candidate: self
                    .state
                    .creating_extrusion
                    .as_ref()
                    .and_then(|ce| ce.merge_candidate),
                extrude_body_mode: self
                    .state
                    .creating_extrusion
                    .as_ref()
                    .map(|ce| ce.body_mode),
                // Extrude face element picker rows (#268): one per picked profile face.
                extrude_faces: (self.state.tool == Tool::Extrude).then(|| {
                    self.state
                        .creating_extrusion
                        .as_ref()
                        .map(|ce| {
                            ce.faces
                                .iter()
                                .enumerate()
                                .map(|(n, f)| {
                                    let kind = match f {
                                        model::ExtrudeFace::Circle(_) => "Circle",
                                        model::ExtrudeFace::Polygon(_) => "Loop",
                                        model::ExtrudeFace::Boolean { .. } => "Region",
                                        model::ExtrudeFace::TextGlyph { .. } => "Glyph",
                                    };
                                    format!("{kind} {}", n + 1)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }),
                // #157/#167: the Chamfer/Fillet selection picker — rows for the in-progress
                // edge set (empty rows still show the picker with its pick hint).
                edge_treatment_rows: (matches!(self.state.tool, Tool::Chamfer | Tool::Fillet)
                    && self.state.sketch_session.is_none())
                .then(|| {
                    self.state
                        .creating_edge_treatment
                        .as_ref()
                        .map(|cet| {
                            cet.edges
                                .iter()
                                .map(|(ei, edge)| {
                                    context::edge_treatment_row_label(&self.state.doc, *ei, *edge)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }),
                // Loft tool: one picker row per picked cross section.
                loft_rows: (self.state.tool == Tool::Loft
                    && self.state.sketch_session.is_none())
                .then(|| {
                    self.state
                        .creating_loft
                        .as_ref()
                        .map(|cl| {
                            cl.sections
                                .iter()
                                .map(|sec| context::loft_section_row_label(&self.state.doc, sec))
                                .collect()
                        })
                        .unwrap_or_default()
                }),
                // #171: "Calibrate scale" shows when exactly one tracing image and one
                // line (on the image's host plane) are selected — the line is the
                // reference segment drawn over a known image feature.
                calibrate_image: {
                    // Guided flow (#163): both reference points placed — the length field.
                    let guided = self.state.creating_calibration.as_ref().and_then(|cal| {
                        (cal.points.len() == 2).then(|| context::CalibrateImageControl {
                            image: cal.image,
                            a: cal.points[0],
                            b: cal.points[1],
                        })
                    });
                    // Legacy selection flow (#171): exactly one image + one line on the
                    // image's host plane selected.
                    let mut image = None;
                    let mut line = None;
                    let mut extras = false;
                    for element in self.state.scene_selection.iter() {
                        match element {
                            SceneElement::Image(i) if image.is_none() => image = Some(i),
                            SceneElement::Line(li) if line.is_none() => line = Some(li),
                            _ => extras = true,
                        }
                    }
                    // A selected calibrated image re-opens its length for editing (#424):
                    // the stored marker span is the reference segment.
                    let recalibrate = self.single_selected_tracing_image().and_then(|i| {
                        let img = self.state.doc.tracing_images.get(i)?;
                        let cal = img.calibration.as_ref()?;
                        let (ox, oy) = img.origin;
                        let (w, h) = (img.width_mm.max(1e-6), img.height_mm.max(1e-6));
                        Some(context::CalibrateImageControl {
                            image: i,
                            a: (ox + cal.u0 * w, oy + cal.v0 * h),
                            b: (ox + cal.u1 * w, oy + cal.v1 * h),
                        })
                    });
                    guided.or(recalibrate).or(match (image, line, extras) {
                        (Some(image), Some(li), false) => self
                            .state
                            .doc
                            .tracing_images
                            .get(image)
                            .filter(|img| !img.deleted)
                            .zip(self.state.doc.lines.get(li).filter(|l| !l.deleted))
                            .filter(|(img, line)| {
                                self.state.doc.sketch_face(line.sketch)
                                    == Some(model::FaceId::ConstructionPlane(img.plane))
                            })
                            .map(|(_, line)| context::CalibrateImageControl {
                                image,
                                a: (line.x0, line.y0),
                                b: (line.x1, line.y1),
                            }),
                        _ => None,
                    })
                },
                // "Calibrate scale" button (#163): one tracing image selected, nothing
                // else, no calibration already running.
                boolean_op: (self.state.tool == Tool::Combine).then(|| {
                    let cb = self.state.creating_boolean.as_ref();
                    let kind = cb.map(|c| c.kind).unwrap_or(model::BooleanOpKind::Combine);
                    let a_len = cb.map(|c| c.a.len()).unwrap_or(0);
                    let b_len = cb.map(|c| c.b.len()).unwrap_or(0);
                    context::BooleanControl {
                        kind,
                        a: cb.map(|c| c.a.clone()).unwrap_or_default(),
                        b: cb.map(|c| c.b.clone()).unwrap_or_default(),
                        picking_b: cb.map(|c| c.picking_b).unwrap_or(false),
                        keep_b: cb.map(|c| c.keep_b).unwrap_or(false),
                        editing: cb.map(|c| c.editing.is_some()).unwrap_or(false),
                        can_commit: match kind {
                            model::BooleanOpKind::Combine => a_len >= 2,
                            _ => a_len >= 1 && b_len >= 1,
                        },
                    }
                }),
                boolean_edit_start: (self.state.tool != Tool::Combine)
                    .then(|| {
                        let mut only = None;
                        for element in self.state.scene_selection.iter() {
                            match (element, only) {
                                (SceneElement::BooleanOp(i), None) => only = Some(i),
                                _ => return None,
                            }
                        }
                        only.filter(|&i| {
                            self.state
                                .doc
                                .boolean_ops
                                .get(i)
                                .is_some_and(|o| !o.deleted)
                        })
                    })
                    .flatten(),
                // The body-move controls are hidden inside a sketch (#306): there, the Move
                // tool is the in-sketch selection gizmo, not the whole-body move.
                move_op: (self.state.tool == Tool::Move
                    && self.state.sketch_session.is_none())
                .then(|| {
                    let cm = self.state.creating_move.as_ref();
                    context::MoveControl {
                        targets: cm.map(|c| c.targets.clone()).unwrap_or_default(),
                        tx: cm.map(|c| c.tx.clone()).unwrap_or_default(),
                        ty: cm.map(|c| c.ty.clone()).unwrap_or_default(),
                        tz: cm.map(|c| c.tz.clone()).unwrap_or_default(),
                        axis_label: cm.and_then(|c| c.axis).map(|a| match a {
                            model::RevolveAxis::Line(li) => names::element_name(
                                &self.state.doc,
                                SceneElement::Line(li),
                            )
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| format!("line {li}")),
                            model::RevolveAxis::X => "the X axis".to_string(),
                            model::RevolveAxis::Y => "the Y axis".to_string(),
                            model::RevolveAxis::Z => "the Z axis".to_string(),
                        }),
                        angle: cm.map(|c| c.angle.clone()).unwrap_or_default(),
                        editing: cm.map(|c| c.editing.is_some()).unwrap_or(false),
                        can_commit: cm
                            .map(|c| !c.targets.is_empty() || !c.plane_targets.is_empty() || !c.image_targets.is_empty())
                            .unwrap_or(false),
                    }
                }),
                move_edit_start: (self.state.tool != Tool::Move)
                    .then(|| {
                        let mut only = None;
                        for element in self.state.scene_selection.iter() {
                            match (element, only) {
                                (SceneElement::MoveOp(i), None) => only = Some(i),
                                _ => return None,
                            }
                        }
                        only.filter(|&i| {
                            self.state.doc.move_ops.get(i).is_some_and(|o| !o.deleted)
                        })
                    })
                    .flatten(),
                repeat_op: (self.state.tool == Tool::Repeat
                    && self.state.sketch_session.is_none())
                .then(|| {
                    let cr = self.state.creating_repeat.as_ref();
                    let preview = cr.and_then(|c| {
                        let probe = model::RepeatOperation {
                            targets: c.targets.clone(),
                            plane_targets: c.plane_targets.clone(),
                            extrusion_targets: c.extrusion_targets.clone(),
                            sketch_targets: c.sketch_targets.clone(),
                            sketch_plane_outputs: Vec::new(),
                            sketch_outputs: Vec::new(),
                            axis: c.axis,
                            mode: c.mode,
                            count: c.count.clone(),
                            spacing: c.spacing.clone(),
                            length: c.length.clone(),
                            length_target: None,
                            outputs: Vec::new(),
                            plane_outputs: Vec::new(),
                            name: None,
                            deleted: false,
                        };
                        (!c.targets.is_empty() || !c.plane_targets.is_empty() || !c.sketch_targets.is_empty() || !c.extrusion_targets.is_empty())
                            .then(|| crate::extrude::repeat_offsets(&self.state.doc, &probe))
                            .flatten()
                            .map(|offsets| offsets.len() + 1)
                    });
                    // The value of the computed variable (#257): derived from the offsets + the
                    // targets' along-axis extent L.
                    let computed_value = cr.and_then(|c| {
                        let probe = model::RepeatOperation {
                            targets: c.targets.clone(),
                            plane_targets: c.plane_targets.clone(),
                            extrusion_targets: c.extrusion_targets.clone(),
                            sketch_targets: c.sketch_targets.clone(),
                            sketch_plane_outputs: Vec::new(),
                            sketch_outputs: Vec::new(),
                            axis: c.axis,
                            mode: c.mode,
                            count: c.count.clone(),
                            spacing: c.spacing.clone(),
                            length: c.length.clone(),
                            length_target: None,
                            outputs: Vec::new(),
                            plane_outputs: Vec::new(),
                            name: None,
                            deleted: false,
                        };
                        let offsets = crate::extrude::repeat_offsets(&self.state.doc, &probe)?;
                        let l = crate::extrude::repeat_extent(&self.state.doc, &probe)?;
                        let unit = self.state.doc.default_length_unit;
                        let fmt = |v: f32| crate::value::format_length_display_in(v, unit);
                        Some(match c.computed_var() {
                            model::RepeatVar::Count => (offsets.len() + 1).to_string(),
                            model::RepeatVar::Gap => {
                                let step = offsets.first().copied().unwrap_or(0.0);
                                fmt(if c.gap_is_offset { step } else { step - l })
                            }
                            model::RepeatVar::Distance => {
                                let last = offsets.last().copied().unwrap_or(0.0);
                                fmt(if c.distance_is_end { last + l } else { last })
                            }
                        })
                    });
                    context::RepeatControl {
                        targets: cr.map(|c| c.targets.clone()).unwrap_or_default(),
                        plane_targets: cr.map(|c| c.plane_targets.clone()).unwrap_or_default(),
                        sketch_targets: cr.map(|c| c.sketch_targets.clone()).unwrap_or_default(),
                        extrusion_targets: cr.map(|c| c.extrusion_targets.clone()).unwrap_or_default(),
                        axis_label: cr
                            .map(|c| match c.axis {
                                model::RevolveAxis::Line(li) => names::element_name(
                                    &self.state.doc,
                                    SceneElement::Line(li),
                                )
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| format!("line {li}")),
                                model::RevolveAxis::X => "the X axis".to_string(),
                                model::RevolveAxis::Y => "the Y axis".to_string(),
                                model::RevolveAxis::Z => "the Z axis".to_string(),
                            })
                            .unwrap_or_else(|| "the X axis".to_string()),
                        mode: cr.map(|c| c.mode).unwrap_or(model::RepeatMode::CountGap),
                        count: cr.map(|c| c.count.clone()).unwrap_or_default(),
                        spacing: cr.map(|c| c.spacing.clone()).unwrap_or_default(),
                        length: cr.map(|c| c.length.clone()).unwrap_or_default(),
                        computed_var: cr.map(|c| c.computed_var()).unwrap_or(model::RepeatVar::Distance),
                        gap_is_offset: cr.map(|c| c.gap_is_offset).unwrap_or(false),
                        distance_is_end: cr.map(|c| c.distance_is_end).unwrap_or(true),
                        computed_value,
                        preview_instances: preview,
                        editing: cr.map(|c| c.editing.is_some()).unwrap_or(false),
                        can_commit: cr
                            .map(|c| !c.targets.is_empty() || !c.plane_targets.is_empty() || !c.sketch_targets.is_empty() || !c.extrusion_targets.is_empty())
                            .unwrap_or(false)
                            && preview.is_some_and(|n| n > 1),
                    }
                }),
                sketch_repeat: self.state.creating_sketch_repeat.as_ref().map(|c| {
                    let direction_is_edge = c.dir_line.is_some();
                    let direction_label = match c.dir_line {
                        Some(li) => names::element_name(&self.state.doc, SceneElement::Line(li))
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| format!("edge {li}")),
                        None => "the U axis".to_string(),
                    };
                    context::SketchRepeatControl {
                        entity_count: c.line_targets.len() + c.circle_targets.len(),
                        direction_label,
                        direction_is_edge,
                        count: c.count.clone(),
                        spacing: c.spacing.clone(),
                        length: c.length.clone(),
                        computed_var: c.computed_var(),
                        gap_is_offset: c.gap_is_offset,
                        distance_is_end: c.distance_is_end,
                        can_commit: c.has_targets(),
                        editing: c.editing.is_some(),
                    }
                }),
                sketch_slice: (self.state.tool == Tool::Slice
                    && self.state.sketch_session.is_some())
                .then(|| {
                    let c = self.state.creating_sketch_slice.as_ref();
                    let mut target_rows: Vec<String> = Vec::new();
                    let mut cutter_rows: Vec<String> = Vec::new();
                    let (mut picking_cutter, mut editing, mut has_t, mut has_c) =
                        (false, false, false, false);
                    if let Some(c) = c {
                        for &li in &c.line_targets {
                            target_rows.push(format!("Line {li}"));
                        }
                        for &ci in &c.circle_targets {
                            target_rows.push(format!("Circle {ci}"));
                        }
                        for n in 0..c.face_targets.len() {
                            target_rows.push(format!("Face {}", n + 1));
                        }
                        cutter_rows = c.cutter_lines.iter().map(|li| format!("Line {li}")).collect();
                        picking_cutter = c.picking_cutter;
                        editing = c.editing.is_some();
                        has_t = c.has_targets();
                        has_c = c.has_cutters();
                    }
                    context::SketchSliceControl {
                        target_rows,
                        cutter_rows,
                        picking_cutter,
                        editing,
                        can_commit: has_t && has_c,
                    }
                }),
                sketch_text: {
                    // A single selected sketch text opens its editor (#286).
                    self.single_selected_sketch_text()
                        .and_then(|i| self.state.doc.sketch_texts.get(i).map(|t| (i, t)))
                        .map(|(i, t)| context::SketchTextControl {
                            index: i,
                            text: t.text.clone(),
                            font_family: t.font_family.clone(),
                            families: crate::text::system_font_families(),
                            bold: t.bold,
                            italic: t.italic,
                            underline: t.underline,
                            size_expr: if t.size_expr.is_empty() {
                                format!("{}", t.size)
                            } else {
                                t.size_expr.clone()
                            },
                            size_mm: t.size,
                            rotation_deg: format!("{:.0}", t.rotation.to_degrees()),
                            wrap: t.wrap_width.map(|w| format!("{w:.0}")).unwrap_or_default(),
                        })
                },
                drawing_view: {
                    // The selected projection on the open drawing page (#289).
                    self.state
                        .selected_drawing_view()
                        .filter(|(d, _)| self.state.editing_drawing == Some(*d))
                        .and_then(|(d, v)| {
                            let view = self
                                .state
                                .doc
                                .drawings
                                .get(d)
                                .filter(|dr| !dr.deleted)
                                .and_then(|dr| dr.views.get(v))?;
                            let source = match view.sketch {
                                Some(si) => crate::names::node_label(
                                    &self.state.doc,
                                    hierarchy::HierarchyNode::Sketch(si),
                                ),
                                None => crate::names::node_label(
                                    &self.state.doc,
                                    hierarchy::HierarchyNode::Body(view.body),
                                ),
                            };
                            let aligned = view.aligned_parent.is_some();
                            // Aligned children show their inherited scale (#296/#300).
                            let scale = if aligned {
                                crate::drawing::resolved_view_scale(&self.state.doc, d, v)
                                    .unwrap_or_default()
                            } else {
                                view.scale.clone().unwrap_or_default()
                            };
                            // The orientations an aligned child may switch between while staying in
                            // line with its base (#332).
                            let inline_orientations = match (view.aligned_parent, view.aligned_dir) {
                                (Some(p), Some(dir)) => self
                                    .state
                                    .doc
                                    .drawings
                                    .get(d)
                                    .and_then(|dr| dr.views.get(p))
                                    .map(|pv| {
                                        crate::drawing::aligned_inline_orientations(
                                            pv.orientation,
                                            dir,
                                        )
                                    })
                                    .unwrap_or_default(),
                                _ => Vec::new(),
                            };
                            let scale_suffix = crate::drawing::resolved_view_scale(
                                &self.state.doc,
                                d,
                                v,
                            )
                            .map(|s| format!(" ({s})"))
                            .unwrap_or_default();
                            let auto_label = format!(
                                "{source} — {}{scale_suffix}",
                                view.orientation.label()
                            );
                            Some(context::DrawingViewControl {
                                view: v,
                                source,
                                orientation: view.orientation,
                                scale,
                                aligned,
                                align_lines: view.align_lines,
                                inline_orientations,
                                style: view.style,
                                label_hidden: view.label_hidden,
                                label_pos: view.label_pos,
                                label_text: view.label_text.clone().unwrap_or_default(),
                                auto_label,
                            })
                        })
                },
                drawing_annotation: self
                    .state
                    .selected_drawing_annotation()
                    .filter(|(d, _)| self.state.editing_drawing == Some(*d))
                    .and_then(|(d, a)| {
                        self.state
                            .doc
                            .drawings
                            .get(d)
                            .and_then(|dr| dr.annotations.get(a))
                            .filter(|ann| !ann.deleted)
                            .map(|ann| context::DrawingAnnotationControl { text: ann.text.clone() })
                    }),
                drawing_selection: self
                    .state
                    .selected_drawing_elements
                    .iter()
                    .map(|(d, element)| {
                        let node = match element {
                            context::DrawingElementRef::Projection(view) => {
                                hierarchy::HierarchyNode::DrawingProjection { drawing: *d, view: *view }
                            }
                            context::DrawingElementRef::Text(annotation) => {
                                hierarchy::HierarchyNode::DrawingAnnotation {
                                    drawing: *d,
                                    annotation: *annotation,
                                }
                            }
                            context::DrawingElementRef::Dimension { view, a, b } => {
                                hierarchy::HierarchyNode::DrawingDimension {
                                    drawing: *d,
                                    view: *view,
                                    a: *a,
                                    b: *b,
                                }
                            }
                        };
                        (*d, *element, crate::names::node_label(&self.state.doc, node))
                    })
                    .collect(),
                drawing_add_active: self.state.tool == Tool::DrawingAdd
                    && self.state.editing_drawing.is_some(),
                drawing_align_active: self.state.tool == Tool::DrawingAlign
                    && self.state.editing_drawing.is_some(),
                drawing_align_base: self.drawing_align_parent.and_then(|v| {
                    let d = self.state.editing_drawing?;
                    Some((v, crate::names::node_label(
                        &self.state.doc,
                        hierarchy::HierarchyNode::DrawingProjection { drawing: d, view: v },
                    )))
                }),
                repeat_edit_start: (self.state.tool != Tool::Repeat)
                    .then(|| {
                        let mut only = None;
                        for element in self.state.scene_selection.iter() {
                            match (element, only) {
                                (SceneElement::RepeatOp(i), None) => only = Some(i),
                                _ => return None,
                            }
                        }
                        only.filter(|&i| {
                            self.state.doc.repeat_ops.get(i).is_some_and(|o| !o.deleted)
                        })
                    })
                    .flatten(),
                slice_op: (self.state.tool == Tool::Slice).then(|| {
                    let cs = self.state.creating_slice.as_ref();
                    let target_rows = cs
                        .map(|c| {
                            c.targets
                                .iter()
                                .map(|&bi| {
                                    names::element_name(&self.state.doc, SceneElement::Body(bi))
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| format!("Body {bi}"))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let cutter_rows = cs
                        .map(|c| {
                            c.cutters
                                .iter()
                                .map(|f| crate::face::face_label(&self.state.doc, f.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    context::SliceControl {
                        target_rows,
                        cutter_rows,
                        picking_cutter: cs.map(|c| c.picking_cutter).unwrap_or(false),
                        extend_infinite: cs.map(|c| c.extend_infinite).unwrap_or(true),
                        editing: cs.map(|c| c.editing.is_some()).unwrap_or(false),
                        can_commit: cs
                            .map(|c| !c.targets.is_empty() && !c.cutters.is_empty())
                            .unwrap_or(false),
                    }
                }),
                slice_edit_start: (self.state.tool != Tool::Slice)
                    .then(|| {
                        let mut only = None;
                        for element in self.state.scene_selection.iter() {
                            match (element, only) {
                                (SceneElement::SliceOp(i), None) => only = Some(i),
                                _ => return None,
                            }
                        }
                        only.filter(|&i| {
                            self.state.doc.slice_ops.get(i).is_some_and(|o| !o.deleted)
                        })
                    })
                    .flatten(),
                revolve_edit_start: (self.state.tool != Tool::Revolve)
                    .then(|| {
                        let mut only = None;
                        for element in self.state.scene_selection.iter() {
                            match (element, only) {
                                (SceneElement::Revolution(i), None) => only = Some(i),
                                _ => return None,
                            }
                        }
                        only.filter(|&i| {
                            self.state.doc.revolutions.get(i).is_some_and(|r| !r.deleted)
                        })
                    })
                    .flatten(),
                revolve: (self.state.tool == Tool::Revolve).then(|| {
                    let cr = self.state.creating_revolve.as_ref();
                    context::RevolveControl {
                        face_count: cr.map(|c| c.faces.len()).unwrap_or(0),
                        face_rows: cr
                            .map(|c| {
                                c.faces
                                    .iter()
                                    .enumerate()
                                    .map(|(n, f)| {
                                        let kind = match f {
                                            model::ExtrudeFace::Circle(_) => "Circle",
                                            model::ExtrudeFace::Polygon(_) => "Loop",
                                            model::ExtrudeFace::Boolean { .. } => "Region",
                                            model::ExtrudeFace::TextGlyph { .. } => "Glyph",
                                        };
                                        format!("{kind} {}", n + 1)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        axis_label: cr.and_then(|c| c.axis).map(|a| match a {
                            model::RevolveAxis::Line(li) => names::element_name(
                                &self.state.doc,
                                SceneElement::Line(li),
                            )
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| format!("line {li}")),
                            model::RevolveAxis::X => "the X axis".to_string(),
                            model::RevolveAxis::Y => "the Y axis".to_string(),
                            model::RevolveAxis::Z => "the Z axis".to_string(),
                        }),
                        // Exactly one picker shows the focus ring (#304): Axis once a
                        // profile is picked but no axis yet, Profile otherwise.
                        axis_focused: cr
                            .is_some_and(|c| !c.faces.is_empty() && c.axis.is_none()),
                        symmetric: cr.map(|c| c.symmetric).unwrap_or(false),
                        body_choice: cr.map(|c| c.body_choice).unwrap_or_default(),
                        cut_bodies: cr.map(|c| c.cut_bodies.clone()).unwrap_or_default(),
                    }
                }),
                calibrate_start: (self.state.creating_calibration.is_none()).then(|| {
                    let mut only_image = None;
                    for element in self.state.scene_selection.iter() {
                        match (element, only_image) {
                            (SceneElement::Image(i), None) => only_image = Some(i),
                            _ => return None,
                        }
                    }
                    only_image.filter(|&i| {
                        self.state.doc.tracing_images.get(i).is_some_and(|img| !img.deleted)
                    })
                }).flatten(),
                calibrate_pending: self
                    .state
                    .creating_calibration
                    .as_ref()
                    .filter(|cal| cal.points.len() < 2)
                    .map(|cal| cal.points.len()),
            };
            let content = context::context_pane_content(&context_input);
            context::sync_name_draft(&mut self.state.context_pane, &self.state.doc, &content);
            context::sync_calibrate_draft(&mut self.state.context_pane, &self.state.doc, &content);
            let mut construction_change: Option<bool> = None;
            let mut curve_mode_change: Option<bool> = None;
            let mut tangent_constraint_change: Option<bool> = None;
            let mut name_commit: Option<(SceneElement, String)> = None;
            let mut constraint_apply: Option<crate::geometric_constraints::GeometricConstraintType> =
                None;
            let mut snapping_change: Option<bool> = None;
            let mut extrude_body_mode_change: Option<actions::ExtrudeBodyMode> = None;
            let mut extrude_face_remove: Option<Option<usize>> = None;
            let mut units_change: Option<context::UnitsChoice> = None;
            let mut edge_picker_edit: Option<Option<usize>> = None;
            let mut selection_edit: Option<context::SelectionEdit> = None;
            let mut tool_picker_edit: Option<(context::PickerTarget, context::ToolPickerAction)> =
                None;
            let mut calibrate_apply: Option<(context::CalibrateImageControl, String)> = None;
            let mut calibrate_begin: Option<usize> = None;
            let mut revolve_edit: Option<context::RevolveEdit> = None;
            let mut boolean_edit: Option<context::BooleanEdit> = None;
            let mut boolean_edit_begin: Option<usize> = None;
            let mut move_edit: Option<context::MoveEdit> = None;
            let mut move_edit_begin: Option<usize> = None;
            let mut repeat_edit: Option<context::RepeatEdit> = None;
            let mut sketch_repeat_edit: Option<context::SketchRepeatEdit> = None;
            let mut sketch_slice_edit: Option<context::SketchSliceEdit> = None;
            let mut sketch_text_edit: Option<context::SketchTextEdit> = None;
            let mut drawing_view_edit: Option<context::DrawingViewEdit> = None;
            let mut drawing_annotation_edit: Option<context::DrawingAnnotationEdit> = None;
            let mut drawing_selection_edit: Option<context::DrawingSelectionEdit> = None;
            let mut drawing_align_clear = false;
            let mut repeat_edit_begin: Option<usize> = None;
            let mut slice_edit: Option<context::SliceEdit> = None;
            let mut slice_edit_begin: Option<usize> = None;
            let mut revolve_edit_begin: Option<usize> = None;
            egui::SidePanel::right("context")
                .resizable(true)
                .default_width(200.0)
                .max_width(280.0)
                .frame(theme::panel_frame())
                .show(ctx, |ui| {
                    context::show_pane(
                        ui,
                        ctx,
                        &content,
                        &mut self.state.context_pane,
                        &self.state.document_health,
                        &self.state.scene_selection,
                        &self.state.doc,
                        &mut |element, name| name_commit = Some((element, name)),
                        &mut |curve_mode| {
                            curve_mode_change = Some(curve_mode);
                        },
                        &mut |tangent_constraint| {
                            tangent_constraint_change = Some(tangent_constraint);
                        },
                        &mut |construction| {
                            construction_change = Some(construction);
                        },
                        &mut |kind| constraint_apply = Some(kind),
                        &mut |enabled| snapping_change = Some(enabled),
                        &mut |mode| extrude_body_mode_change = Some(mode),
                        &mut |remove| extrude_face_remove = Some(remove),
                        &mut |choice| units_change = Some(choice),
                        &mut |edit| edge_picker_edit = Some(edit),
                        &mut |edit| selection_edit = Some(edit),
                        &mut |target, edit| tool_picker_edit = Some((target, edit)),
                        &mut |edit| revolve_edit = Some(edit),
                        &mut |edit| boolean_edit = Some(edit),
                        &mut |op| boolean_edit_begin = Some(op),
                        &mut |edit| move_edit = Some(edit),
                        &mut |op| move_edit_begin = Some(op),
                        &mut |edit| repeat_edit = Some(edit),
                        &mut |edit| sketch_repeat_edit = Some(edit),
                        &mut |edit| sketch_slice_edit = Some(edit),
                        &mut |edit| sketch_text_edit = Some(edit),
                        &mut |edit| drawing_view_edit = Some(edit),
                        &mut |edit| drawing_annotation_edit = Some(edit),
                        &mut |edit| drawing_selection_edit = Some(edit),
                        &mut || drawing_align_clear = true,
                        &mut |op| repeat_edit_begin = Some(op),
                        &mut |edit| slice_edit = Some(edit),
                        &mut |op| slice_edit_begin = Some(op),
                        &mut |op| revolve_edit_begin = Some(op),
                        &mut |image| calibrate_begin = Some(image),
                        &mut |control, text| calibrate_apply = Some((control, text)),
                    );
                });
            if let Some(edit) = revolve_edit {
                let cr = self
                    .state
                    .creating_revolve
                    .get_or_insert_with(actions::CreatingRevolve::default);
                match edit {
                    context::RevolveEdit::Symmetric(v) => cr.symmetric = v,
                    context::RevolveEdit::BodyChoice(choice) => cr.body_choice = choice,
                    context::RevolveEdit::RemoveFace(Some(i)) => {
                        if i < cr.faces.len() {
                            cr.faces.remove(i);
                        }
                        if cr.faces.is_empty() {
                            cr.sketch = None;
                        }
                    }
                    context::RevolveEdit::RemoveFace(None) => {
                        cr.faces.clear();
                        cr.sketch = None;
                    }
                    context::RevolveEdit::ClearAxis => cr.axis = None,
                }
            }
            if let Some(remove) = extrude_face_remove {
                // Removing a face from the picker toggles it back off (#268).
                if let Some(ce) = self.state.creating_extrusion.as_ref() {
                    let faces: Vec<model::ExtrudeFace> = match remove {
                        Some(i) => ce.faces.get(i).cloned().into_iter().collect(),
                        None => ce.faces.clone(),
                    };
                    for face in faces {
                        self.state.apply(Action::ToggleExtrudeFace { face });
                    }
                }
            }
            if let Some(edit) = boolean_edit {
                match edit {
                    context::BooleanEdit::Commit => {
                        self.state.apply(Action::CommitBoolean);
                    }
                    edit => {
                        let cb = self
                            .state
                            .creating_boolean
                            .get_or_insert_with(actions::CreatingBoolean::default);
                        match edit {
                            context::BooleanEdit::Kind(kind) => {
                                cb.kind = kind;
                                if kind == model::BooleanOpKind::Combine {
                                    // Combine has a single picker: fold B into A.
                                    let b = std::mem::take(&mut cb.b);
                                    for bi in b {
                                        if !cb.a.contains(&bi) {
                                            cb.a.push(bi);
                                        }
                                    }
                                    cb.picking_b = false;
                                }
                            }
                            context::BooleanEdit::KeepB(v) => cb.keep_b = v,
                            context::BooleanEdit::Commit => unreachable!(),
                        }
                    }
                }
            }
            if let Some(edit) = move_edit {
                match edit {
                    context::MoveEdit::Commit => {
                        self.state.apply(Action::CommitMove);
                    }
                    edit => {
                        let cm = self
                            .state
                            .creating_move
                            .get_or_insert_with(actions::CreatingMove::default);
                        match edit {
                            context::MoveEdit::Tx(v) => cm.tx = v,
                            context::MoveEdit::Ty(v) => cm.ty = v,
                            context::MoveEdit::Tz(v) => cm.tz = v,
                            context::MoveEdit::Angle(v) => cm.angle = v,
                            context::MoveEdit::Axis(a) => cm.axis = a,
                            context::MoveEdit::Commit => unreachable!(),
                        }
                    }
                }
            }
            if let Some(op) = move_edit_begin {
                if let Some(existing) = self.state.doc.move_ops.get(op).cloned() {
                    self.state.creating_move = Some(actions::CreatingMove {
                        targets: existing.targets,
                        plane_targets: existing.plane_targets,
                        image_targets: existing.image_targets,
                        tx: existing.tx,
                        ty: existing.ty,
                        tz: existing.tz,
                        axis: existing.axis,
                        angle: existing.angle,
                        editing: Some(op),
                    });
                    self.state.apply(Action::SetTool(Tool::Move));
                }
            }
            if let Some(edit) = repeat_edit {
                match edit {
                    context::RepeatEdit::Commit => {
                        self.state.apply(Action::CommitRepeat);
                    }
                    edit => {
                        let cr = self
                            .state
                            .creating_repeat
                            .get_or_insert_with(actions::CreatingRepeat::default);
                        use model::RepeatVar;
                        match edit {
                            context::RepeatEdit::Axis(a) => cr.axis = a,
                            context::RepeatEdit::Count(v) => {
                                cr.count = v;
                                cr.touch_var(RepeatVar::Count);
                            }
                            context::RepeatEdit::Gap(v) => {
                                cr.spacing = v;
                                cr.touch_var(RepeatVar::Gap);
                            }
                            context::RepeatEdit::Distance(v) => {
                                cr.length = v;
                                cr.touch_var(RepeatVar::Distance);
                            }
                            context::RepeatEdit::ToggleGapOffset => {
                                cr.gap_is_offset = !cr.gap_is_offset;
                                cr.recompute_mode();
                            }
                            context::RepeatEdit::ToggleDistanceEnd => {
                                cr.distance_is_end = !cr.distance_is_end;
                                cr.recompute_mode();
                            }
                            context::RepeatEdit::Commit => unreachable!(),
                        }
                    }
                }
            }
            if let Some(edit) = sketch_repeat_edit {
                use model::RepeatVar;
                if let context::SketchRepeatEdit::Commit = edit {
                    if let (Some(cr), true) = (
                        self.state.creating_sketch_repeat.as_ref(),
                        self.state.creating_sketch_repeat.as_ref().is_some_and(|c| c.has_targets()),
                    ) {
                        let (dir_u, dir_v) = cr.direction(&self.state.doc);
                        let action = match cr.editing {
                            Some(op) => Action::EditSketchRepeatOperation {
                                op,
                                line_targets: cr.line_targets.clone(),
                                circle_targets: cr.circle_targets.clone(),
                                dir_u,
                                dir_v,
                                mode: cr.mode,
                                count: cr.count.clone(),
                                spacing: cr.spacing.clone(),
                                length: cr.length.clone(),
                            },
                            None => Action::CreateSketchRepeatOperation {
                                sketch: cr.sketch,
                                line_targets: cr.line_targets.clone(),
                                circle_targets: cr.circle_targets.clone(),
                                dir_u,
                                dir_v,
                                mode: cr.mode,
                                count: cr.count.clone(),
                                spacing: cr.spacing.clone(),
                                length: cr.length.clone(),
                            },
                        };
                        self.state.creating_sketch_repeat = None;
                        self.state.apply(action);
                    }
                } else if let Some(cr) = self.state.creating_sketch_repeat.as_mut() {
                    match edit {
                        context::SketchRepeatEdit::Count(v) => {
                            cr.count = v;
                            cr.touch_var(RepeatVar::Count);
                        }
                        context::SketchRepeatEdit::Gap(v) => {
                            cr.spacing = v;
                            cr.touch_var(RepeatVar::Gap);
                        }
                        context::SketchRepeatEdit::Distance(v) => {
                            cr.length = v;
                            cr.touch_var(RepeatVar::Distance);
                        }
                        context::SketchRepeatEdit::ToggleGapOffset => {
                            cr.gap_is_offset = !cr.gap_is_offset;
                            cr.recompute_mode();
                        }
                        context::SketchRepeatEdit::ToggleDistanceEnd => {
                            cr.distance_is_end = !cr.distance_is_end;
                            cr.recompute_mode();
                        }
                        context::SketchRepeatEdit::ClearDirection => cr.dir_line = None,
                        context::SketchRepeatEdit::Commit => {}
                    }
                }
            }
            if let Some(edit) = sketch_slice_edit {
                match edit {
                    context::SketchSliceEdit::Commit => {
                        if let Some(cs) = self
                            .state
                            .creating_sketch_slice
                            .as_ref()
                            .filter(|c| c.has_targets() && c.has_cutters())
                            .cloned()
                        {
                            let action = match cs.editing {
                                Some(op) => Action::EditSketchSliceOperation {
                                    op,
                                    line_targets: cs.line_targets,
                                    circle_targets: cs.circle_targets,
                                    face_targets: cs.face_targets,
                                    cutter_lines: cs.cutter_lines,
                                },
                                None => Action::CreateSketchSliceOperation {
                                    sketch: cs.sketch,
                                    line_targets: cs.line_targets,
                                    circle_targets: cs.circle_targets,
                                    face_targets: cs.face_targets,
                                    cutter_lines: cs.cutter_lines,
                                },
                            };
                            self.state.creating_sketch_slice = None;
                            self.state.apply(action);
                        }
                    }
                    other => {
                        // Role toggles may fire before the first pick, so seed the draft from the
                        // active sketch session.
                        if let Some(session) = self.state.sketch_session {
                            let cs = self
                                .state
                                .creating_sketch_slice
                                .get_or_insert_with(|| {
                                    actions::CreatingSketchSlice::new(session.sketch)
                                });
                            match other {
                                context::SketchSliceEdit::PickingCutter(v) => cs.picking_cutter = v,
                                context::SketchSliceEdit::ClearTargets => {
                                    cs.line_targets.clear();
                                    cs.circle_targets.clear();
                                    cs.face_targets.clear();
                                }
                                context::SketchSliceEdit::ClearCutters => cs.cutter_lines.clear(),
                                context::SketchSliceEdit::Commit => {}
                            }
                        }
                    }
                }
            }
            if let Some(edit) = sketch_text_edit {
                // Re-resolve the single selected text rather than trusting a stale control index.
                let selected = self
                    .single_selected_sketch_text()
                    .and_then(|i| self.state.doc.sketch_texts.get(i).cloned().map(|t| (i, t)));
                if let Some((index, existing)) = selected {
                    let mut text = existing.text.clone();
                    let mut font_family = existing.font_family.clone();
                    let mut bold = existing.bold;
                    let mut italic = existing.italic;
                    let mut underline = existing.underline;
                    let mut size = existing.size;
                    let mut size_expr = existing.size_expr.clone();
                    let mut rotation = existing.rotation;
                    let mut wrap_width = existing.wrap_width;
                    let mut valid = true;
                    match edit {
                        context::SketchTextEdit::Text(v) => text = v,
                        context::SketchTextEdit::Font(v) => font_family = v,
                        context::SketchTextEdit::Bold(v) => bold = v,
                        context::SketchTextEdit::Italic(v) => italic = v,
                        context::SketchTextEdit::Underline(v) => underline = v,
                        // The raw expression is stored either way so typing isn't clobbered;
                        // the evaluated size only moves once the expression is valid.
                        context::SketchTextEdit::Size(v) => {
                            size_expr = v.clone();
                            if let Some(s) = crate::value::eval_length_mm_in_doc(&v, &self.state.doc)
                                .filter(|s| *s > 0.0)
                            {
                                size = s;
                            }
                        }
                        context::SketchTextEdit::Rotation(v) => match v.trim().parse::<f32>() {
                            Ok(deg) => rotation = deg.to_radians(),
                            Err(_) => valid = false,
                        },
                        // Empty clears wrapping; a positive number wraps to that width (#282).
                        context::SketchTextEdit::Wrap(v) => {
                            let t = v.trim();
                            if t.is_empty() {
                                wrap_width = None;
                            } else {
                                match crate::value::eval_length_mm_in_doc(t, &self.state.doc)
                                    .filter(|w| *w > 0.0)
                                {
                                    Some(w) => wrap_width = Some(w),
                                    None => valid = false,
                                }
                            }
                        }
                    }
                    if valid && !text.trim().is_empty() {
                        self.state.apply(Action::EditSketchText {
                            index,
                            text,
                            font_family,
                            bold,
                            italic,
                            underline,
                            size,
                            size_expr,
                            rotation,
                            wrap_width,
                        });
                    }
                }
            }
            if let Some(edit) = drawing_view_edit {
                if let Some((drawing, view)) = self.state.selected_drawing_view() {
                    match edit {
                        context::DrawingViewEdit::Orientation(orientation) => {
                            self.state.apply(Action::SetDrawingViewOrientation {
                                drawing,
                                view,
                                orientation,
                            });
                        }
                        context::DrawingViewEdit::Scale(scale) => {
                            self.state
                                .apply(Action::SetDrawingViewScale { drawing, view, scale });
                        }
                        context::DrawingViewEdit::Style(style) => {
                            self.state
                                .apply(Action::SetDrawingViewStyle { drawing, view, style });
                        }
                        context::DrawingViewEdit::SetAllDimensions(show) => {
                            self.state.apply(Action::SetAllDrawingDimensions {
                                drawing,
                                view,
                                show,
                            });
                        }
                        context::DrawingViewEdit::UseCurrentView => {
                            // Set the projection to the 3D viewport's current angle (#366): a free
                            // orientation whose basis matches what the camera is showing.
                            let (right, up) = crate::view_cube::free_basis(&self.state.cam);
                            self.state.apply(Action::SetDrawingViewOrientation {
                                drawing,
                                view,
                                orientation: model::DrawingOrientation::Free { right, up },
                            });
                        }
                        context::DrawingViewEdit::AlignLines(show) => {
                            self.state.apply(Action::SetDrawingViewAlignLines {
                                drawing,
                                view,
                                show,
                            });
                        }
                        context::DrawingViewEdit::LabelHidden(hidden) => {
                            self.state.apply(Action::SetDrawingViewLabel {
                                drawing,
                                view,
                                hidden: Some(hidden),
                                pos: None,
                                text: None,
                            });
                        }
                        context::DrawingViewEdit::LabelPos(pos) => {
                            self.state.apply(Action::SetDrawingViewLabel {
                                drawing,
                                view,
                                hidden: None,
                                pos: Some(pos),
                                text: None,
                            });
                        }
                        context::DrawingViewEdit::LabelText(text) => {
                            self.state.apply(Action::SetDrawingViewLabel {
                                drawing,
                                view,
                                hidden: None,
                                pos: None,
                                text: Some(text),
                            });
                        }
                        context::DrawingViewEdit::Remove => {
                            self.state.apply(Action::RemoveDrawingView { drawing, view });
                        }
                    }
                }
            }
            if let Some(edit) = drawing_annotation_edit {
                if let Some((drawing, annotation)) = self.state.selected_drawing_annotation() {
                    match edit {
                        context::DrawingAnnotationEdit::Text(text) => {
                            self.state.apply(Action::EditDrawingAnnotationText {
                                drawing,
                                annotation,
                                text,
                            });
                        }
                        context::DrawingAnnotationEdit::Remove => {
                            self.state
                                .apply(Action::RemoveDrawingAnnotation { drawing, annotation });
                        }
                    }
                }
            }
            if let Some(edit) = drawing_selection_edit {
                match edit {
                    context::DrawingSelectionEdit::Remove(drawing, element) => {
                        self.state.deselect_drawing_element(drawing, element);
                    }
                    context::DrawingSelectionEdit::Clear => {
                        self.state.clear_drawing_selection();
                    }
                }
            }
            if drawing_align_clear {
                self.drawing_align_parent = None;
            }
            if let Some(op) = repeat_edit_begin {
                if let Some(existing) = self.state.doc.repeat_ops.get(op).cloned() {
                    let (computed, gap_is_offset, distance_is_end) = existing.mode.to_repeat_ui();
                    self.state.creating_repeat = Some(actions::CreatingRepeat {
                        targets: existing.targets,
                        plane_targets: existing.plane_targets,
                        extrusion_targets: existing.extrusion_targets,
                        sketch_targets: existing.sketch_targets,
                        axis: existing.axis,
                        mode: existing.mode,
                        count: existing.count,
                        spacing: existing.spacing,
                        length: existing.length,
                        gap_is_offset,
                        distance_is_end,
                        var_mru: computed.as_mru(),
                        editing: Some(op),
                    });
                    self.state.apply(Action::SetTool(Tool::Repeat));
                }
            }
            if let Some(edit) = slice_edit {
                match edit {
                    context::SliceEdit::Commit => {
                        self.state.apply(Action::CommitSlice);
                    }
                    edit => {
                        let cs = self
                            .state
                            .creating_slice
                            .get_or_insert_with(actions::CreatingSlice::default);
                        match edit {
                            context::SliceEdit::PickingCutter(v) => cs.picking_cutter = v,
                            context::SliceEdit::ExtendInfinite(v) => cs.extend_infinite = v,
                            context::SliceEdit::RemoveTarget(Some(i)) => {
                                if i < cs.targets.len() {
                                    cs.targets.remove(i);
                                }
                            }
                            context::SliceEdit::RemoveTarget(None) => cs.targets.clear(),
                            context::SliceEdit::RemoveCutter(Some(i)) => {
                                if i < cs.cutters.len() {
                                    cs.cutters.remove(i);
                                }
                            }
                            context::SliceEdit::RemoveCutter(None) => cs.cutters.clear(),
                            context::SliceEdit::Commit => unreachable!(),
                        }
                    }
                }
            }
            if let Some(op) = slice_edit_begin {
                if let Some(existing) = self.state.doc.slice_ops.get(op).cloned() {
                    self.state.creating_slice = Some(actions::CreatingSlice {
                        targets: existing.targets,
                        cutters: existing.cutters,
                        picking_cutter: false,
                        extend_infinite: existing.extend_infinite,
                        editing: Some(op),
                    });
                    self.state.apply(Action::SetTool(Tool::Slice));
                }
            }
            if let Some(op) = boolean_edit_begin {
                if let Some(existing) = self.state.doc.boolean_ops.get(op).cloned() {
                    self.state.creating_boolean = Some(actions::CreatingBoolean {
                        kind: existing.kind,
                        a: existing.a,
                        b: existing.b,
                        picking_b: false,
                        keep_b: existing.keep_b,
                        editing: Some(op),
                    });
                    self.state.apply(Action::SetTool(Tool::Combine));
                }
            }
            if let Some(op) = revolve_edit_begin {
                if let Some(existing) = self.state.doc.revolutions.get(op).cloned() {
                    let (body_choice, cut_bodies) = match &existing.mode {
                        model::RevolveMode::NewBody => {
                            (actions::RevolveBodyChoice::NewBody, Vec::new())
                        }
                        model::RevolveMode::AddTo(_) => {
                            (actions::RevolveBodyChoice::AddTouching, Vec::new())
                        }
                        model::RevolveMode::Cut(b) => {
                            (actions::RevolveBodyChoice::Cut, b.clone())
                        }
                    };
                    self.state.creating_revolve = Some(actions::CreatingRevolve {
                        sketch: Some(existing.sketch),
                        faces: existing.faces,
                        axis: Some(existing.axis),
                        angle_live: existing.angle_deg,
                        text: format!("{:.0}", existing.angle_deg),
                        user_edited: true,
                        pending_focus: false,
                        symmetric: existing.symmetric,
                        body_choice,
                        cut_bodies,
                        editing: Some(op),
                    });
                    self.state.apply(Action::SetTool(Tool::Revolve));
                }
            }
            if let Some(image) = calibrate_begin {
                self.state.apply(Action::BeginImageCalibration { image });
            }
            if let Some((control, mut text)) = calibrate_apply {
                // #201: a typed length can define a parameter (`name = expr`).
                let _ = actions::commit_inline_parameter_defs(&mut self.state.doc, [&mut text]);
                match crate::value::eval_parameter_in_doc(&text, &self.state.doc) {
                    Some(crate::value::EvaluatedParameter::LengthMm(length)) if length > 0.0 => {
                        self.state.apply(Action::CalibrateImage {
                            image: control.image,
                            a: control.a,
                            b: control.b,
                            length,
                        });
                    }
                    _ => {
                        self.state.status = format!("Not a usable length: {text}");
                    }
                }
            }
            if let Some((target, edit)) = tool_picker_edit {
                // Remove one row (or clear) from a tool-owned element picker (#213).
                match target {
                    context::PickerTarget::RevolveCut => {
                        if let Some(cr) = self.state.creating_revolve.as_mut() {
                            remove_or_clear(&mut cr.cut_bodies, edit);
                        }
                    }
                    context::PickerTarget::MoveTargets => {
                        if let Some(cm) = self.state.creating_move.as_mut() {
                            remove_or_clear(&mut cm.targets, edit);
                        }
                    }
                    context::PickerTarget::RepeatTargets => {
                        if let Some(cr) = self.state.creating_repeat.as_mut() {
                            remove_or_clear(&mut cr.targets, edit);
                        }
                    }
                    // Combine's A/B sides: clicking a picker (Focus) makes it the active side
                    // the next viewport click lands on; otherwise remove/clear that side.
                    context::PickerTarget::CombineA => {
                        if let Some(cb) = self.state.creating_boolean.as_mut() {
                            if edit == context::ToolPickerAction::Focus {
                                cb.picking_b = false;
                            } else {
                                remove_or_clear(&mut cb.a, edit);
                            }
                        }
                    }
                    context::PickerTarget::CombineB => {
                        if let Some(cb) = self.state.creating_boolean.as_mut() {
                            if edit == context::ToolPickerAction::Focus {
                                cb.picking_b = true;
                            } else {
                                remove_or_clear(&mut cb.b, edit);
                            }
                        }
                    }
                }
            }
            if let Some(edit) = edge_picker_edit {
                // Remove one row (or clear the set) from the active tool's picked set
                // (#167); dropping the last edge cancels the treatment entirely.
                if self.state.tool == Tool::Loft {
                    match edit {
                        Some(index) => {
                            if let Some(cl) = self.state.creating_loft.as_mut() {
                                if index < cl.sections.len() {
                                    cl.sections.remove(index);
                                }
                            }
                        }
                        None => self.state.creating_loft = None,
                    }
                } else {
                    match edit {
                        Some(index) => {
                            if let Some(cet) = self.state.creating_edge_treatment.as_mut() {
                                if index < cet.edges.len() {
                                    cet.edges.remove(index);
                                }
                                if cet.edges.is_empty() {
                                    self.state.creating_edge_treatment = None;
                                }
                            }
                        }
                        None => self.state.creating_edge_treatment = None,
                    }
                }
            }
            if let Some(edit) = selection_edit {
                // The unified selection picker (#213, Select/Constraint tools): its rows mirror
                // the live selection, so removing one deselects that element (toggle-off) and
                // Clear empties the selection.
                match edit {
                    context::SelectionEdit::Remove(element) => {
                        selection::click_scene_selection(
                            &mut self.state.scene_selection,
                            element,
                            true,
                        );
                    }
                    context::SelectionEdit::Clear => self.state.scene_selection.clear(),
                }
            }
            if let Some(enabled) = snapping_change {
                self.state.apply(Action::SetSnapping(enabled));
            }
            if let Some(kind) = constraint_apply {
                self.state.apply(Action::AddGeometricConstraint(kind));
            }
            if let Some((element, name)) = name_commit {
                self.state
                    .apply(Action::CommitElementName { element, name });
            }
            if let Some(construction) = construction_change {
                self.state
                    .apply(Action::ApplyConstruction { construction });
            }
            if let Some(curve_mode) = curve_mode_change {
                self.state.apply(Action::ApplyCurveMode { curve_mode });
            }
            if let Some(tangent_constraint) = tangent_constraint_change {
                self.state.apply(Action::ApplyTangentConstraint { tangent_constraint });
            }
            if let Some(mode) = extrude_body_mode_change {
                self.state.apply(Action::SetExtrudeBodyMode { mode });
            }
            if let Some(choice) = units_change {
                match choice {
                    context::UnitsChoice::Document { length, angle } => {
                        self.state.apply(Action::SetDocumentUnits { length, angle });
                    }
                    context::UnitsChoice::Sketch { sketch, length, angle } => {
                        self.state
                            .apply(Action::SetSketchUnits { sketch, length, angle });
                    }
                    context::UnitsChoice::Component { component, length, angle } => {
                        self.state
                            .apply(Action::SetComponentUnits { component, length, angle });
                    }
                }
            }
        }

        let render_state = frame.wgpu_render_state();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // A technical drawing open (#180) takes over the central area with its
                // white-on-black editor sheet (#254); otherwise the 3D viewport renders as usual.
                match self.state.editing_drawing {
                    Some(di) if self.state.doc.drawings.get(di).is_some_and(|d| !d.deleted) => {
                        self.draw_drawing_pane(ui, di);
                    }
                    _ => {
                        self.state.editing_drawing = None;
                        self.draw_viewport(ui, render_state);
                    }
                }
            });

        // A popped-out drawing (#276) renders in its own OS window so it can sit beside the 3D
        // view. Uses an *immediate* viewport so the render closure can borrow `self`.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(di) = self.drawing_window {
            if self.state.doc.drawings.get(di).is_none_or(|d| d.deleted) {
                self.drawing_window = None;
            } else {
                let title =
                    crate::names::node_label(&self.state.doc, hierarchy::HierarchyNode::Drawing(di));
                let builder = egui::ViewportBuilder::default()
                    .with_title(format!("Drawing — {title}"))
                    .with_inner_size([900.0, 700.0]);
                let mut close = false;
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("drawing_popout"),
                    builder,
                    |vctx, _class| {
                        theme::apply(vctx);
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(vctx, |ui| {
                                self.draw_drawing_pane(ui, di);
                            });
                        if vctx.input(|i| i.viewport().close_requested()) {
                            close = true;
                        }
                    },
                );
                if close {
                    self.drawing_window = None;
                }
            }
        }
    }
}

/// Suppress unmodified keyboard shortcuts while a [`egui::TextEdit`] (or other focused text input)
/// is active.
fn keyboard_shortcuts_suppressed(ctx: &egui::Context) -> bool {
    ctx.wants_keyboard_input()
}

/// What to do once a script/REPL run has finished, decided independent of the live `egui`
/// context and process so it's unit-testable (#125).
struct ScriptFinishedAction {
    /// Send `ViewportCommand::Close`.
    close: bool,
    /// Also fail the process (non-zero exit) once the event loop returns — an uncaught
    /// script error under `--exit` must fail CI, not hang or silently exit 0.
    fail_process: bool,
}

/// A script/REPL run just finished; `error` is whether it ended on an uncaught error,
/// `should_quit` is whether the script itself called `bearcad.quit()`, and
/// `exit_on_script_complete` is whether `--exit` was passed on the command line.
fn script_finished_close_action(
    error: bool,
    should_quit: bool,
    exit_on_script_complete: bool,
) -> ScriptFinishedAction {
    if error {
        // An uncaught error must not leave the app running forever under `--exit`
        // (previously it would: only the success/`quit()` paths below closed the window).
        ScriptFinishedAction {
            close: exit_on_script_complete,
            fail_process: exit_on_script_complete,
        }
    } else if should_quit {
        ScriptFinishedAction { close: true, fail_process: false }
    } else {
        ScriptFinishedAction { close: exit_on_script_complete, fail_process: false }
    }
}

/// FPS-mode crosshair (#91): the cursor is locked at the viewport center, so this marks
/// where clicks land.
fn draw_fps_crosshair(painter: &egui::Painter, viewport: egui::Rect) {
    let c = viewport.center();
    let arm = 7.0;
    let gap = 2.5;
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0f32)] {
        painter.line_segment(
            [
                c + egui::vec2(dx * gap, dy * gap),
                c + egui::vec2(dx * arm, dy * arm),
            ],
            stroke,
        );
    }
}

fn next_rect_focus_axis(focused: usize) -> RectAxis {
    if focused == 0 {
        RectAxis::Height
    } else {
        RectAxis::Width
    }
}

fn next_plane_focus_dim(focused: PlaneDim) -> PlaneDim {
    if focused == PlaneDim::Offset {
        PlaneDim::Angle
    } else {
        PlaneDim::Offset
    }
}

/// URL of the in-repo third-party open-source licenses document (#86). Opened by
/// Help ▸ Licenses.
const LICENSES_DOC_URL: &str =
    "https://github.com/iffy/BearCAD/blob/master/THIRD_PARTY_LICENSES.md";

/// Open the third-party licenses document in the user's default browser, without
/// pulling in a URL-opening crate.
fn open_licenses_document() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(LICENSES_DOC_URL);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", LICENSES_DOC_URL]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(LICENSES_DOC_URL);
        c
    };
    cmd.spawn().map(|_| ())
}

/// Colours used in the viewport.
mod col {
    use egui::Color32;
    pub const BG: Color32 = Color32::from_gray(28);
    pub const GRID: Color32 = Color32::from_gray(55);
    pub const GRID_AXIS: Color32 = Color32::from_gray(90);
    pub const X_AXIS: Color32 = Color32::from_rgb(200, 70, 70);
    pub const Y_AXIS: Color32 = Color32::from_rgb(70, 190, 90);
    /// Matches the view-cube Z triad (`view_cube::AXES`).
    pub const Z_AXIS: Color32 = Color32::from_rgb(80, 140, 230);
    /// Shared stroke color for all solid sketch shape edges (lines, rect edges, circles).
    pub const RECT_LINE: Color32 = Color32::from_rgb(120, 170, 240);
    pub const PREVIEW: Color32 = Color32::from_rgb(240, 200, 120);
    /// Viewport border while a sketch is open (#74) — an unmissable mode indicator distinct
    /// from every other viewport accent color in this palette.
    pub const SKETCH_MODE_BORDER: Color32 = Color32::from_rgb(255, 140, 30);
    /// Pivot shown while right-dragging to orbit the camera.
    pub const ORBIT_PIVOT: Color32 = Color32::from_rgb(255, 105, 180);
    /// Drop line from the orbit pivot to the ground plane.
    pub const ORBIT_PIVOT_DROP: Color32 = Color32::from_rgba_premultiplied(255, 105, 180, 70);
    pub const DIM_INPUT_BG: Color32 = Color32::from_rgb(22, 24, 30);
    pub const DIM_INPUT_BG_FOCUS: Color32 = Color32::from_rgb(34, 36, 44);
    pub const DIM_INPUT_BORDER: Color32 = Color32::from_rgb(110, 118, 136);
    pub const DIM_INPUT_BORDER_FOCUS: Color32 = Color32::from_rgb(255, 186, 84);
    pub const DIM_INPUT_TEXT: Color32 = Color32::from_rgb(232, 235, 242);
    pub const DIM_INPUT_TEXT_FOCUS: Color32 = Color32::from_rgb(255, 255, 255);
    /// Faint highlight so selected digits stay readable on the dark input background.
    pub const DIM_INPUT_SELECTION: Color32 = Color32::from_rgba_premultiplied(36, 26, 12, 36);
    /// Highlight for the dimension edge/segment tied to the focused input.
    pub const DIM_EDGE_HIGHLIGHT: Color32 = DIM_INPUT_BORDER_FOCUS;
    /// Committed sketch dimension lines and labels in edit mode.
    pub const DIM_ANNOTATION: Color32 = Color32::from_rgb(180, 188, 204);
    /// All construction geometry (planes, etc.) shares this colour.
    pub const CONSTRUCTION: Color32 = crate::construction::CONSTRUCTION_RGBA;
    /// Fully-constrained solid lines (#172): no remaining degrees of freedom.
    pub const RECT_LINE_CONSTRAINED: Color32 = Color32::from_rgb(225, 228, 235);
    /// Associative projections (#140): dashed like construction, in their own teal.
    pub const PROJECTION: Color32 = Color32::from_rgb(70, 200, 190);
    /// Faded appearance for geometry outside the active sketch face.
    pub const SKETCH_DIMMED: f32 = crate::gpu_viewport::SKETCH_DIMMED;
}

const GRID_EXTENT: f32 = gpu_viewport::GRID_EXTENT;
const GRID_STEP: f32 = gpu_viewport::GRID_STEP;
/// Width of the sketch-mode viewport border (#74).
const SKETCH_MODE_BORDER_WIDTH: f32 = 3.0;

/// Screen-space height of a floating dimension input (frame + text field).
const DIM_INPUT_HEIGHT: f32 = 26.0;
/// Radial outset (px, beyond the arc/gizmo ring) for the angle dimension's editable
/// input box. Pushed far enough out along the angle bisector that the box clears the
/// angle gizmo's grab handle (which sits on the ring, off the bisector), so the handle
/// isn't hidden behind the text field (#40). Sized from the handle hit radius plus the
/// full input height plus a small margin so even the box's near corner clears the
/// handle's grab circle for typical short live values.
const ANGLE_DIM_INPUT_GIZMO_CLEARANCE_PX: f32 =
    AXIS_GIZMO_HANDLE_HIT_RADIUS_PX + DIM_INPUT_HEIGHT + 4.0;
/// Horizontal padding inside the dimension input frame (inner margin × 2).
const DIM_INPUT_FRAME_H_PAD: f32 = 10.0;
/// Minimum text-edit width (fits short live values like `80.0`).
const DIM_INPUT_MIN_TEXT_WIDTH: f32 = 48.0;
/// Approximate monospace glyph width at 13pt (used for layout sizing).
const DIM_INPUT_CHAR_WIDTH: f32 = 7.8;

fn build_gpu_dimension_labels(
    ctx: &egui::Context,
    layouts: &[CommittedDimLayout],
    view: &PlanarLabelView,
    cam: &camera::Camera,
    viewport: egui::Rect,
    view_proj: &glam::Mat4,
    project: &impl Fn(glam::Vec3) -> Option<egui::Pos2>,
    skip_constraint: Option<DimLabelTarget>,
    health: &document_health::DocumentHealth,
) -> Vec<gpu_viewport::ViewportDimLabel> {
    layouts
        .iter()
        .filter(|layout| layout.arc_geom.is_none())
        .map(|layout| {
            let color = document_health::constraint_annotation_color(
                health,
                layout.target,
                col::DIM_ANNOTATION,
            );
            let (text_vertices, text_indices) = if skip_constraint == Some(layout.target) {
                (Vec::new(), Vec::new())
            } else {
                gpu_viewport::build_planar_label_mesh(
                    ctx,
                    &layout.world_geom,
                    view,
                    &layout.label,
                    color,
                    cam,
                    viewport,
                    view_proj,
                    project,
                )
            };
            gpu_viewport::ViewportDimLabel {
                world_geom: layout.world_geom,
                color,
                text_vertices,
                text_indices,
                draw_dimension_lines: layout.arc_geom.is_none(),
            }
        })
        .collect()
}

const SIDE_PANEL_IDS: &[&str] = &["tree", "parameters", "context"];

/// True while the pointer is on a side-panel resize grip (don't override its cursor).
/// Whether the cursor is near the Move rotation ring's projected circle (#216): sample the
/// circle and test the cursor's distance to the projected polyline.
fn rotation_ring_hit(
    pp: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    center: Vec3,
    axis: Vec3,
    radius: f32,
) -> bool {
    const TOL_PX: f32 = 8.0;
    let n = axis.normalize_or_zero();
    if n == Vec3::ZERO {
        return false;
    }
    let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = n.cross(reference).normalize_or_zero();
    let v = n.cross(u);
    const SEG: usize = 48;
    let mut prev: Option<egui::Pos2> = None;
    for i in 0..=SEG {
        let t = i as f32 / SEG as f32 * std::f32::consts::TAU;
        let world = center + (u * t.cos() + v * t.sin()) * radius;
        let sp = project(world);
        if let (Some(a), Some(b)) = (prev, sp) {
            if dist_point_to_segment(pp, a, b) < TOL_PX {
                return true;
            }
        }
        prev = sp;
    }
    false
}

/// The body index a pick target identifies, if it's a body sub-element (#218): an edge, vertex,
/// or face all belong to one body.
fn body_index_from_pick(kind: &construction::PickTargetKind) -> Option<usize> {
    match kind {
        construction::PickTargetKind::BodyEdge { body, .. }
        | construction::PickTargetKind::BodyVertex { body, .. }
        | construction::PickTargetKind::BodyFace { body, .. } => Some(*body),
        _ => None,
    }
}

/// Apply a tool-owned element picker's row action (#213) to its backing body-index vector:
/// `Remove(i)` drops row `i`, `Clear` empties the set, `Focus` is a no-op here (the caller
/// handles active-picker switching for multi-picker tools).
fn remove_or_clear(bodies: &mut Vec<usize>, action: context::ToolPickerAction) {
    match action {
        context::ToolPickerAction::Focus => {}
        context::ToolPickerAction::Remove(index) if index < bodies.len() => {
            bodies.remove(index);
        }
        context::ToolPickerAction::Remove(_) => {}
        context::ToolPickerAction::Clear => bodies.clear(),
    }
}

fn side_panel_resize_active(ctx: &egui::Context) -> bool {
    SIDE_PANEL_IDS.iter().any(|id| {
        ctx.read_response(egui::Id::new(*id).with("__resize"))
            .is_some_and(|r| r.dragged() || r.hovered())
    })
}

/// Set a viewport cursor only when the viewport owns the pointer this frame.
fn set_viewport_cursor(
    ctx: &egui::Context,
    response: &egui::Response,
    viewport_owns_pointer: bool,
    icon: egui::CursorIcon,
) {
    if side_panel_resize_active(ctx) {
        return;
    }
    if viewport_owns_pointer || response.hovered() {
        ctx.set_cursor_icon(icon);
    }
}

/// Pointer in viewport coordinates for hit-testing and drags.
fn viewport_pointer_pos(
    response: &egui::Response,
    viewport_owns_pointer: bool,
) -> Option<egui::Pos2> {
    response
        .hover_pos()
        .or(viewport_owns_pointer.then_some(response.interact_pointer_pos()).flatten())
}

/// True while orbiting/panning or dragging sketch geometry — pick hover is distracting then.
fn suppress_viewport_pick_hover(
    ui: &egui::Ui,
    response: &egui::Response,
    vertex_drag_active: bool,
    line_drag_active: bool,
    dim_label_drag_active: bool,
    angle_gizmo_drag_active: bool,
    plane_gizmo_drag_active: bool,
    bezier_handle_drag_active: bool,
) -> bool {
    ui.input(|i| i.pointer.secondary_down())
        || response.dragged_by(egui::PointerButton::Secondary)
        || ui.input(|i| i.pointer.middle_down())
        || response.dragged_by(egui::PointerButton::Middle)
        || vertex_drag_active
        || line_drag_active
        || dim_label_drag_active
        || angle_gizmo_drag_active
        || plane_gizmo_drag_active
        || bezier_handle_drag_active
}

fn resolve_viewport_hover_highlight(
    suppress_hover: bool,
    tool: Tool,
    sketch_session: Option<SketchSession>,
    creating_plane: bool,
    editing_committed_dim: bool,
    over_committed_dim_label: bool,
    dim_label_drag: bool,
    pointer_screen: Option<egui::Pos2>,
    cam: &camera::Camera,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    doc: &model::Document,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    occlusion: Option<&construction::PickOcclusion>,
) -> Option<gpu_viewport::ViewportHoverHighlight> {
    if suppress_hover {
        return None;
    }
    let pp = pointer_screen?;
    match tool {
        Tool::Sketch => pick_sketch_face(pp, project, doc, cam.eye())
            .map(gpu_viewport::ViewportHoverHighlight::SketchFace),
        // Loft tool (#202): glow the closed profile (circle or line loop) under the cursor —
        // the same cross section a click would add — so it has hover feedback like every other
        // pick tool. The whole profile loop lights up, not just the one edge hit.
        Tool::Loft if sketch_session.is_none() => {
            let gp = cam.ground_point(pp, viewport, vp);
            let target = resolve_pick_target(pp, project, gp, doc, occlusion)?;
            let element = match target.kind {
                construction::PickTargetKind::Circle(ci) => Some(SceneElement::Circle(ci)),
                construction::PickTargetKind::Line(li) => Some(SceneElement::Line(li)),
                _ => None,
            };
            let section = element.and_then(|el| extrude::loft_section_from_element(doc, el))?;
            let (world_loop, _) = extrude::face_profile_world(doc, &section.face)?;
            Some(gpu_viewport::ViewportHoverHighlight::ClosedLoop { world_loop })
        }
        // The Text tool joins the draw tools (#383): outside a sketch it clicks a face to
        // begin sketching there, so it hover-highlights faces the same way.
        Tool::Rectangle | Tool::Line | Tool::Circle | Tool::Text
            if sketch_session.is_none() =>
        {
            pick_sketch_face(pp, project, doc, cam.eye())
                .map(gpu_viewport::ViewportHoverHighlight::SketchFace)
        }
        Tool::ConstructionPlane if !creating_plane => {
            let gp = cam.ground_point(pp, viewport, vp);
            resolve_pick_target(pp, project, gp, doc, occlusion)
                .map(|t| gpu_viewport::ViewportHoverHighlight::PickTarget(t.kind))
        }
        // Dimension tool (#190): glow the dimensionable segment under the cursor — the same
        // thing a click would dimension — so hover has feedback like every other pick tool.
        Tool::Dimension
            if !editing_committed_dim && !over_committed_dim_label && !dim_label_drag =>
        {
            let session = sketch_session?;
            let gp = cam.ground_point(pp, viewport, vp);
            let target = resolve_pick_target(pp, project, gp, doc, occlusion)?;
            crate::constraints::distance_target_from_pick(doc, session.sketch, &target.kind)
                .map(|_| gpu_viewport::ViewportHoverHighlight::PickTarget(target.kind))
        }
        Tool::Select | Tool::Constraint
            if !editing_committed_dim && !over_committed_dim_label && !dim_label_drag =>
        {
            let gp = cam.ground_point(pp, viewport, vp);
            // 3D body sub-elements (#144): a vertex, edge, or face of any body highlights under
            // the cursor, in that priority order (a corner beats an edge beats the face it's on).
            // A vertex hidden behind another body is not a candidate (#155).
            if let Some((kind, _)) =
                construction::nearest_body_vertex(pp, project, doc).filter(|(kind, _)| {
                    // Hidden/shadow bodies aren't pickable (#258); a vertex behind another body
                    // isn't either (#155).
                    occlusion.is_none_or(|occ| occ.pickable(doc, kind))
                        && match kind {
                            construction::PickTargetKind::BodyVertex { position, .. } => {
                                occlusion.is_none_or(|occ| !occ.occluded(*position))
                            }
                            _ => true,
                        }
                })
            {
                return Some(gpu_viewport::ViewportHoverHighlight::PickTarget(kind));
            }
            let t = resolve_pick_target(pp, project, gp, doc, occlusion);
            // A sketch vertex under the cursor wins over the origin; otherwise the origin, when
            // hovered within its pick radius, beats edges — matching click selection (#240).
            if let Some(t) = &t {
                if matches!(t.kind, construction::PickTargetKind::Point(_)) {
                    return Some(gpu_viewport::ViewportHoverHighlight::PickTarget(t.kind.clone()));
                }
            }
            if let Some(session) = sketch_session {
                if let Some(frame) = sketch_geometry_frame(doc, session.sketch) {
                    if project(frame.origin).is_some_and(|op| {
                        (op - pp).length() <= construction::POINT_PICK_RADIUS_PX
                    }) {
                        return Some(gpu_viewport::ViewportHoverHighlight::Element(
                            SceneElement::Origin,
                        ));
                    }
                }
                // The sketch's fixed reference lines (#394) — the origin axes (#189) and the
                // sketched-on face's own edges (#26/#27) — hover-highlight like they
                // click-select (#241). Plain sketch lines fall through to the generic pick
                // below; `nearest_sketch_line_in_sketch` already ranks them together, so an
                // axis only wins here when it genuinely is the closest.
                if let Some((target, _)) =
                    nearest_sketch_line_in_sketch(pp, project, doc, session.sketch)
                {
                    if matches!(
                        target,
                        model::ConstraintLine::OriginAxis(_)
                            | model::ConstraintLine::FaceEdge { .. }
                    ) {
                        return Some(gpu_viewport::ViewportHoverHighlight::Element(
                            vertex_drag::scene_element_for_line(target),
                        ));
                    }
                }
            }
            if let Some(t) = t {
                if scene_element_from_pick(&t.kind).is_some()
                    || matches!(t.kind, construction::PickTargetKind::BodyEdge { .. })
                {
                    return Some(gpu_viewport::ViewportHoverHighlight::PickTarget(t.kind));
                }
            }
            crate::face::pick_body_face(pp, project, doc, cam.eye())
                .filter(|kind| occlusion.is_none_or(|occ| occ.pickable(doc, kind)))
                .map(gpu_viewport::ViewportHoverHighlight::PickTarget)
        }
        // Body-set tools (#227): while one is active its picker accepts whole bodies, so the
        // body under the cursor styles as selectable — hover-highlight it via the same
        // whole-body resolution the click path uses (edge/vertex/face → owning body).
        Tool::Combine | Tool::Move | Tool::Repeat | Tool::Slice => {
            let gp = cam.ground_point(pp, viewport, vp);
            let body = resolve_pick_target(pp, project, gp, doc, occlusion)
                .as_ref()
                .and_then(|t| body_index_from_pick(&t.kind))
                .or_else(|| {
                    crate::face::pick_body_face(pp, project, doc, cam.eye())
                        .filter(|kind| occlusion.is_none_or(|occ| occ.pickable(doc, kind)))
                        .as_ref()
                        .and_then(body_index_from_pick)
                });
            body.map(|bi| {
                gpu_viewport::ViewportHoverHighlight::Element(SceneElement::Body(bi))
            })
        }
        _ => None,
    }
}

fn plane_gizmo_hover(
    cp: &CreatingConstructionPlane,
    pointer_screen: Option<egui::Pos2>,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Option<AxisGizmoHit> {
    let pp = pointer_screen?;
    match &cp.reference {
        PlaneReference::Face { origin, normal, .. } => {
            if offset_gizmo_hit(pp, project, *origin, *normal, cp.offset_live) {
                Some(AxisGizmoHit::Offset)
            } else {
                None
            }
        }
        PlaneReference::Axis {
            origin,
            direction,
            ..
        } => axis_gizmo_hit(
            pp,
            project,
            *origin,
            *direction,
            cp.offset_live,
            cp.axis_angle_deg,
        ),
    }
}

fn build_viewport_scene_input<'a>(
    doc: &'a model::Document,
    cam: &'a camera::Camera,
    viewport: egui::Rect,
    sketch_session: Option<SketchSession>,
    element_visibility: &'a hierarchy::ElementVisibility,
    selection: &'a crate::selection::SceneSelection,
    document_health: &'a document_health::DocumentHealth,
    creating_rect: Option<&CreatingRect>,
    creating_line: Option<&CreatingLine>,
    creating_circle: Option<&CreatingCircle>,
    creating_plane: Option<&CreatingConstructionPlane>,
    creating_extrusion: Option<&CreatingExtrusion>,
    creating_edge_treatment: Option<&CreatingEdgeTreatment>,
    creating_revolve: Option<&actions::CreatingRevolve>,
    creating_loft: Option<&actions::CreatingLoft>,
    creating_repeat: Option<&actions::CreatingRepeat>,
    pending_extrude_target: Option<model::ExtrudeTarget>,
    plane_gizmo: Option<gpu_viewport::ViewportPlaneGizmo>,
    extrude_gizmo: Option<gpu_viewport::ViewportExtrudeGizmo>,
    vertex_treatment_gizmo: Option<gpu_viewport::ViewportExtrudeGizmo>,
    move_gizmos: Vec<gpu_viewport::ViewportExtrudeGizmo>,
    move_rotation_gizmo: Option<gpu_viewport::MoveRotationGizmo>,
    revolve_arc_gizmo: Option<gpu_viewport::RevolveArcGizmo>,
    vertex_treatment_preview: Option<Vec<Vec3>>,
    hover_highlight: Option<gpu_viewport::ViewportHoverHighlight>,
    dimension_labels: &'a [gpu_viewport::ViewportDimLabel],
    dim_label_view: Option<PlanarLabelView>,
    constraint_graphics: Option<&'a [constraint_viewport::ConstraintViewportGraphic]>,
    cut_highlight_bodies: Vec<usize>,
    faded_bodies: Vec<usize>,
    sketch_repeat_ghost: Vec<(Vec3, Vec3)>,
    edit_preview_meshes: std::collections::HashMap<usize, extrude::SolidMesh>,
) -> gpu_viewport::ViewportSceneInput<'a> {
    let preview_rect = creating_rect.and_then(|cr| {
        let session = sketch_session?;
        let frame = sketch_geometry_frame(doc, session.sketch)?;
        let end = cr.end_point(&frame, doc);
        let (ou, ov) = world_to_local(&frame, cr.origin);
        let (eu, ev) = world_to_local(&frame, end);
        let x = ou.min(eu);
        let y = ov.min(ev);
        let xr = ou.max(eu);
        let yt = ov.max(ev);
        let corners = [
            crate::face::local_to_world(&frame, x, y),
            crate::face::local_to_world(&frame, xr, y),
            crate::face::local_to_world(&frame, xr, yt),
            crate::face::local_to_world(&frame, x, yt),
        ];
        Some(gpu_viewport::PreviewRect {
            corners,
            construction: cr.construction,
        })
    });
    let preview_line = creating_line.and_then(|cl| {
        let session = sketch_session?;
        let frame = sketch_geometry_frame(doc, session.sketch)?;
        let end = cl.end_point(&frame, doc);
        let (u0, v0) = world_to_local(&frame, cl.origin);
        let (u1, v1) = world_to_local(&frame, end);
        let mut preview = Line::from_local_endpoints(session.sketch, u0, v0, u1, v1);
        preview.construction = cl.construction;
        // #73: live-preview this in-progress segment's own curve — smoothed against the
        // previous chained segment (tangent-constraint on) or an independent corner handle
        // (off). The very first segment of a fresh chain has no previous segment to derive a
        // tangent from, so it stays straight until a third point makes the joint meaningful.
        if let Some(prev_idx) = cl.chained_from {
            if let Some(prev_far) = doc.lines.get(prev_idx).map(|l| (l.x0, l.y0)) {
                let (_, line_bezier) = chained_curve_handles(
                    prev_far,
                    cl.chained_from_bezier,
                    (u0, v0),
                    (u1, v1),
                    cl.curve_mode,
                    cl.tangent_constraint,
                );
                preview.bezier = line_bezier;
            }
        }
        Some(preview)
    });
    let preview_circle = creating_circle.and_then(|cc| {
        let session = sketch_session?;
        let frame = sketch_geometry_frame(doc, session.sketch)?;
        let (cu, cv) = world_to_local(&frame, cc.origin);
        let r = cc.radius(&frame, doc);
        let angle = cc.diameter_dim_angle(&frame);
        let mut preview = Circle::from_local_center_radius(
            session.sketch, cu, cv, r, angle,
        );
        preview.construction = cc.construction;
        Some(preview)
    });
    let vp = cam.view_proj(viewport);
    let plane_preview = creating_plane.map(|cp| {
        let plane = cp.preview_plane();
        let dependents = cp
            .edit_index
            .and_then(|index| preview_plane_edit_dependents(doc, index, &plane));
        let dim_outline = plane_dim_layouts(
            &|w: Vec3| cam.project(w, viewport, &vp),
            &plane,
            &cp.reference,
            cp.offset_live,
            cp.axis_angle_deg,
        )
        .is_some();
        gpu_viewport::ViewportPlanePreview {
            plane,
            dependents,
            dim_outline,
        }
    });
    let active_sketch_face = sketch_session.and_then(|session| doc.sketch_face(session.sketch));
    let active_sketch_face = active_sketch_face.filter(|face| !matches!(face, FaceId::ConstructionPlane(_)));

    // The in-progress 3D edge chamfer/fillet (#77) also drives a ghost-preview solid, reusing
    // the exact same `preview_extrusion`/`editing_extrusion` mechanism as the extrude tool: a
    // full clone of the target extrusion with the live treatment spliced in (never touching
    // `doc` until commit), rendered translucent while the committed body is hidden. The two
    // cases are mutually exclusive (one needs a sketch open, the other needs it closed), so
    // there's no ambiguity about which one is "active".
    let editing_extrusion = creating_extrusion
        .and_then(|ce| ce.edit_index)
        .or_else(|| creating_edge_treatment.and_then(|cet| Some(cet.primary()?.0)));

    // Live ghost of the in-progress revolve (#revolve): a temp Revolution meshed with the
    // fallback lathe every frame (cheap, and identical to what a lean commit would build).
    let preview_solid = creating_revolve.and_then(|cr| {
        let axis = cr.axis?;
        let sketch = cr.sketch?;
        if cr.faces.is_empty() {
            return None;
        }
        let probe = model::Revolution {
            sketch,
            faces: cr.faces.clone(),
            axis,
            angle_deg: cr.evaluated_angle_deg(doc),
            symmetric: cr.symmetric,
            mode: model::RevolveMode::NewBody,
            name: None,
            deleted: false,
        };
        extrude::revolve_mesh(doc, &probe)
    })
    .or_else(|| {
        // Live ghost of the in-progress loft (#203): mesh the picked sections the same way a
        // commit would (ordered along the principal direction), so the blended solid previews
        // and updates as sections are added or removed. Needs at least two sections.
        let cl = creating_loft?;
        if cl.sections.len() < 2 {
            return None;
        }
        let loft = model::Loft {
            sections: extrude::order_loft_sections(doc, cl.sections.clone()),
            name: None,
            deleted: false,
        };
        extrude::loft_mesh(doc, &loft)
    });

    let preview_extrusion = creating_extrusion
        .and_then(|ce| {
            (!ce.faces.is_empty()).then(|| model::Extrusion {
                sketch: ce.sketch,
                faces: ce.faces.clone(),
                distance: ce.evaluated_distance(doc),
                // While dragging the gizmo, the target is only known live (not yet committed
                // onto `ce`) — fall back to it so the ghost preview actually shows the slanted
                // shape it will land in, instead of a generic blind extrude (#63).
                target: ce.target.clone().or(pending_extrude_target),
                expression: String::new(),
                name: None,
                deleted: false,
                edge_treatments: Vec::new(),
            })
        })
        .or_else(|| {
            let cet = creating_edge_treatment?;
            let amount = cet.evaluated_amount(doc);
            if amount <= 0.0 {
                return None;
            }
            // Ghost-preview every in-progress edge on the primary (gizmo-anchoring)
            // extrusion (#166). Edges of the set living on *other* extrusions still commit,
            // but only the primary gets a ghost — the single-slot preview mechanism shows
            // one extrusion at a time.
            let (primary, _) = cet.primary()?;
            let treatments: Vec<model::EdgeTreatment> = cet
                .edges
                .iter()
                .filter(|(ei, _)| *ei == primary)
                .map(|(_, edge)| model::EdgeTreatment { edge: *edge, kind: cet.kind, amount })
                .collect();
            crate::extrude::extrusion_with_edge_treatments(doc, primary, treatments)
        });

    // #142: a cut extrusion previews the finished cut result over the target body, not an
    // additive block. Only the extrude tool sets a `Cut` body mode (the edge-treatment path
    // never does), so this reads straight off the in-progress extrusion.
    let preview_cut_body = creating_extrusion.and_then(|ce| match ce.body_mode {
        crate::actions::ExtrudeBodyMode::Cut(bi) => Some(bi),
        _ => None,
    });

    // #223: while the Repeat tool is collecting bodies and its count/spacing change, ghost the
    // would-be instances — each picked body's mesh translated to every instance offset along the
    // axis — so the pattern previews live before commit. Reuses `repeat_offsets`, the same
    // evaluation a commit runs, so the ghosts land exactly where the copies will.
    let repeat_ghosts: Vec<extrude::SolidMesh> = creating_repeat
        .filter(|c| !c.targets.is_empty())
        .and_then(|c| {
            let probe = model::RepeatOperation {
                targets: c.targets.clone(),
                plane_targets: c.plane_targets.clone(),
                extrusion_targets: c.extrusion_targets.clone(),
                sketch_targets: c.sketch_targets.clone(),
                sketch_plane_outputs: Vec::new(),
                sketch_outputs: Vec::new(),
                axis: c.axis,
                mode: c.mode,
                count: c.count.clone(),
                spacing: c.spacing.clone(),
                length: c.length.clone(),
                length_target: None,
                outputs: Vec::new(),
                plane_outputs: Vec::new(),
                name: None,
                deleted: false,
            };
            let (_, dir) = extrude::axis_world(doc, c.axis)?;
            let offsets = extrude::repeat_offsets(doc, &probe)?;
            let mut ghosts = Vec::new();
            for &bi in &c.targets {
                if let Some(base) = extrude::body_solid_mesh(doc, bi) {
                    for &off in &offsets {
                        let t = dir * off;
                        ghosts.push(extrude::SolidMesh {
                            triangles: base
                                .triangles
                                .iter()
                                .map(|[a, b, c]| [*a + t, *b + t, *c + t])
                                .collect(),
                        });
                    }
                }
            }
            (!ghosts.is_empty()).then_some(ghosts)
        })
        .unwrap_or_default();

    gpu_viewport::ViewportSceneInput {
        doc,
        cam,
        viewport,
        palette: gpu_viewport::ViewportPalette {
            background: col::BG,
            grid: col::GRID,
            grid_axis: col::GRID_AXIS,
            x_axis: col::X_AXIS,
            y_axis: col::Y_AXIS,
            z_axis: col::Z_AXIS,
            rect_line: col::RECT_LINE,
            rect_line_constrained: col::RECT_LINE_CONSTRAINED,
            preview: col::PREVIEW,
            construction: col::CONSTRUCTION,
            projection: col::PROJECTION,
            dim_edge_highlight: col::DIM_EDGE_HIGHLIGHT,
            construction_plane_fill: construction::PLANE_FILL_RGBA,
            construction_plane_opacity: gpu_viewport::DEFAULT_CONSTRUCTION_PLANE_OPACITY,
        },
        sketch_session,
        selection,
        cut_highlight_bodies,
        faded_bodies,
        sketch_repeat_ghost,
        edit_preview_meshes,
        element_visibility,
        preview_rect,
        preview_line,
        preview_circle,
        preview_extrusion,
        preview_solid,
        repeat_ghosts,
        editing_extrusion,
        preview_cut_body,
        plane_preview,
        active_sketch_face,
        dimension_labels,
        dim_label_view,
        plane_gizmo,
        extrude_gizmo,
        vertex_treatment_gizmo,
        move_gizmos,
        move_rotation_gizmo,
        revolve_arc_gizmo,
        vertex_treatment_preview: vertex_treatment_preview
            .map(|points| gpu_viewport::VertexTreatmentPreviewGeom { points }),
        hover_highlight,
        hover_color: construction::PICK_HOVER_RGBA,
        document_health,
        constraint_graphics,
        constraint_connector_color: Some(col::DIM_EDGE_HIGHLIGHT),
    }
}
/// Expression fields grow with content up to this many characters.
const DIM_INPUT_MAX_CHARS: usize = 20;

fn dim_input_text_width(text: &str) -> f32 {
    let chars = text.chars().count().clamp(1, DIM_INPUT_MAX_CHARS);
    (chars as f32 * DIM_INPUT_CHAR_WIDTH).max(DIM_INPUT_MIN_TEXT_WIDTH)
}

fn dim_input_total_width(text: &str) -> f32 {
    dim_input_text_width(text) + DIM_INPUT_FRAME_H_PAD
}

fn dim_input_size_for_text(text: &str) -> egui::Vec2 {
    egui::vec2(dim_input_total_width(text), DIM_INPUT_HEIGHT)
}

fn dim_input_max_size() -> egui::Vec2 {
    dim_input_size_for_text(&"m".repeat(DIM_INPUT_MAX_CHARS))
}
const DIM_LABEL_GAP: f32 = 8.0;
const DIM_LABEL_PAD: f32 = 2.0;
const DIM_REPULSION_ITERS: usize = 16;

/// Preferred offsets from edge anchors (width: bottom mid, height: left mid, line: segment mid).
const WIDTH_LABEL_OFFSET: egui::Vec2 = egui::Vec2::new(-20.0, 14.0);
const HEIGHT_LABEL_OFFSET: egui::Vec2 = egui::Vec2::new(-48.0, -4.0);
/// Perpendicular gap from the line to the nearest edge of the dimension input.
const LINE_LABEL_DISTANCE: f32 = 18.0;

/// Screen-space layout for a floating dimension input.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DimInputLayout {
    pos: egui::Pos2,
    rect: egui::Rect,
}

fn dim_input_rect_at(top_left: egui::Pos2, size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(top_left, size)
}

fn layout_at(pos: egui::Pos2, size: egui::Vec2) -> DimInputLayout {
    DimInputLayout {
        pos,
        rect: dim_input_rect_at(pos, size),
    }
}

/// Smallest axis-aligned push to separate `moving` from `obstacle` (with padding).
fn separation_vector(moving: egui::Rect, obstacle: egui::Rect, padding: f32) -> egui::Vec2 {
    let obs = obstacle.expand(padding);
    if !moving.intersects(obs) {
        return egui::Vec2::ZERO;
    }
    let pen_left = moving.max.x - obs.min.x;
    let pen_right = obs.max.x - moving.min.x;
    let pen_top = moving.max.y - obs.min.y;
    let pen_bottom = obs.max.y - moving.min.y;
    // When boxes only touch (penetration 0), still nudge apart so we don't stall.
    const MIN_PUSH: f32 = 1.0;
    if pen_left.min(pen_right) < pen_top.min(pen_bottom) {
        if pen_left <= pen_right {
            egui::vec2(-pen_left.max(MIN_PUSH), 0.0)
        } else {
            egui::vec2(pen_right.max(MIN_PUSH), 0.0)
        }
    } else if pen_top <= pen_bottom {
        egui::vec2(0.0, -pen_top.max(MIN_PUSH))
    } else {
        egui::vec2(0.0, pen_bottom.max(MIN_PUSH))
    }
}

fn resolve_rectangle_dim_positions(
    bottom_mid: egui::Pos2,
    left_mid: egui::Pos2,
) -> (egui::Pos2, egui::Pos2) {
    let mut width_pos = bottom_mid + WIDTH_LABEL_OFFSET;
    let mut height_pos = left_mid + HEIGHT_LABEL_OFFSET;
    for _ in 0..DIM_REPULSION_ITERS {
        let w_rect = dim_input_rect_at(width_pos, dim_input_max_size());
        let h_rect = dim_input_rect_at(height_pos, dim_input_max_size());
        let w_push = separation_vector(w_rect, h_rect, DIM_LABEL_PAD);
        let h_push = separation_vector(h_rect, w_rect, DIM_LABEL_PAD);
        if w_push.length_sq() + h_push.length_sq() < 0.25 {
            break;
        }
        width_pos += w_push;
        height_pos += h_push;
    }
    (width_pos, height_pos)
}

fn rectangle_labels_clear(width: egui::Rect, height: egui::Rect) -> bool {
    !width.intersects(height.expand(DIM_LABEL_PAD))
}

fn rectangle_dim_layouts(
    bottom_mid: egui::Pos2,
    left_mid: egui::Pos2,
    width_text: &str,
    height_text: &str,
) -> (DimInputLayout, DimInputLayout) {
    let (width_pos, height_pos) = resolve_rectangle_dim_positions(bottom_mid, left_mid);
    let width = layout_at(width_pos, dim_input_size_for_text(width_text));
    let height = layout_at(height_pos, dim_input_size_for_text(height_text));
    debug_assert!(rectangle_labels_clear(width.rect, height.rect));
    (width, height)
}

fn segment_intersects_rect(pa: egui::Pos2, pb: egui::Pos2, rect: egui::Rect) -> bool {
    if rect.contains(pa) || rect.contains(pb) {
        return true;
    }
    let edges = [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ];
    for (c, d) in edges {
        if segments_intersect(pa, pb, c, d) {
            return true;
        }
    }
    false
}

fn segments_intersect(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, d: egui::Pos2) -> bool {
    fn cross(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    let ab = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ab == 0.0 && ab_d == 0.0 {
        return false;
    }
    ab * ab_d <= 0.0 && cd * cd_b <= 0.0
}

/// Unit vector perpendicular to the line, on the preferred label side (upper-left in screen space).
fn line_perpendicular_unit(pa: egui::Pos2, pb: egui::Pos2) -> egui::Vec2 {
    let delta = pb - pa;
    if delta.length_sq() < 1e-4 {
        return egui::vec2(-1.0, -1.0).normalized();
    }
    let dir = delta.normalized();
    let perp_a = egui::vec2(-dir.y, dir.x);
    let perp_b = egui::vec2(dir.y, -dir.x);
    let prefer = egui::vec2(-1.0, -1.0).normalized();
    if perp_a.dot(prefer) >= perp_b.dot(prefer) {
        perp_a
    } else {
        perp_b
    }
}

fn aabb_half_extent_along(dir: egui::Vec2, size: egui::Vec2) -> f32 {
    if dir.length_sq() < 1e-8 {
        return 0.0;
    }
    let n = dir.normalized();
    size.x * 0.5 * n.x.abs() + size.y * 0.5 * n.y.abs()
}

fn line_dim_top_left(
    pa: egui::Pos2,
    pb: egui::Pos2,
    gap_from_line: f32,
    size: egui::Vec2,
) -> egui::Pos2 {
    let mid = pa.lerp(pb, 0.5);
    let perp = line_perpendicular_unit(pa, pb);
    let center_dist = gap_from_line + aabb_half_extent_along(-perp, size);
    let center = mid + perp * center_dist;
    center - size * 0.5
}

fn dist_point_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    if ab.length_sq() < 1e-8 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

#[cfg(test)]
fn dist_rect_to_segment(rect: egui::Rect, pa: egui::Pos2, pb: egui::Pos2) -> f32 {
    if segment_intersects_rect(pa, pb, rect) {
        return 0.0;
    }
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    corners
        .into_iter()
        .map(|c| dist_point_to_segment(c, pa, pb))
        .fold(f32::MAX, f32::min)
}

fn line_dim_layout(pa: egui::Pos2, pb: egui::Pos2, text: &str) -> DimInputLayout {
    let size = dim_input_size_for_text(text);
    let mut gap = LINE_LABEL_DISTANCE;
    for _ in 0..DIM_REPULSION_ITERS {
        let pos = line_dim_top_left(pa, pb, gap, size);
        let rect = dim_input_rect_at(pos, size).expand(DIM_LABEL_GAP);
        if !segment_intersects_rect(pa, pb, rect) {
            return layout_at(pos, size);
        }
        gap += 2.0;
    }
    layout_at(line_dim_top_left(pa, pb, gap, size), size)
}

fn pointer_over_dim_inputs(pointer: egui::Pos2, layouts: &[DimInputLayout]) -> bool {
    layouts.iter().any(|layout| layout.rect.contains(pointer))
}

fn format_live_dimension(v: f32) -> String {
    if v.abs() < 0.1 {
        "0".to_string()
    } else {
        format!("{:.1}", v)
    }
}

/// Second click on the viewport (not a dimension input) commits the in-progress sketch.
fn should_commit_sketch_on_click(
    was_creating: bool,
    primary_pressed: bool,
    over_input: bool,
) -> bool {
    was_creating && primary_pressed && !over_input
}

/// Whether the dimension field should keep its entire value selected for overwrite typing.
fn should_select_all_rect_value(
    gained_focus: bool,
    has_focus: bool,
    is_focus_target: bool,
    pending_focus: bool,
    user_edited: bool,
    changed_this_frame: bool,
) -> bool {
    if changed_this_frame {
        return false;
    }
    gained_focus
        || (is_focus_target && pending_focus && has_focus)
        || (is_focus_target && has_focus && !user_edited)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SketchDimFieldResult {
    changed: bool,
    enter_commit: bool,
    lost_focus: bool,
    inline_parameter_added: Option<crate::parameters::InlineParameterCommit>,
    inline_parameter_error: Option<String>,
}

fn sketch_dimension_enter_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn consume_sketch_dimension_enter(ui: &mut egui::Ui) {
    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
}

/// Commit when Enter was pressed on a focused dim field, or when Enter is pressed with no dim focused.
fn should_commit_sketch_on_enter(
    field_enter_commit: bool,
    dim_field_focused: bool,
    enter_pressed: bool,
) -> bool {
    field_enter_commit || (enter_pressed && !dim_field_focused)
}

fn angle_expression_field_errors(text: &str, doc: &model::Document) -> Vec<String> {
    let t = text.trim();
    if t.is_empty() {
        return vec!["Expression cannot be empty".to_string()];
    }
    if crate::value::eval_angle_rad_in_doc(t, doc).is_none() {
        return vec![format!("Invalid angle expression '{t}'")];
    }
    Vec::new()
}

/// Show a sketch dimension field; selects all text when it gains focus so typing replaces the value.
fn show_sketch_dimension_field(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    doc: &mut model::Document,
    sketch: Option<model::SketchId>,
    is_focus_target: bool,
    pending_focus: &mut bool,
    user_edited: bool,
    angle: bool,
) -> SketchDimFieldResult {
    let has_focus = ctx.memory(|m| m.focused()) == Some(id);
    if has_focus {
        expression_autocomplete_handle_keys(ui, ctx, id, text, doc, &[]);
    }
    let field_errors = if angle {
        angle_expression_field_errors(text, doc)
    } else {
        length_expression_field_errors(text, doc, None)
    };
    let has_errors = !field_errors.is_empty();
    let show_computed_row = if angle {
        crate::value::shows_computed_angle_in_doc(text, doc)
    } else {
        shows_computed_length_in_doc(text, doc)
    };
    let widget = if has_focus {
        &ui.style().visuals.widgets.active
    } else {
        &ui.style().visuals.widgets.inactive
    };
    let frame = egui::Frame::default()
        .fill(if has_errors {
            INVALID_BG
        } else if has_focus {
            col::DIM_INPUT_BG_FOCUS
        } else {
            col::DIM_INPUT_BG
        })
        .stroke(egui::Stroke::new(
            widget.bg_stroke.width,
            if has_errors {
                INVALID_BORDER
            } else if has_focus {
                col::DIM_INPUT_BORDER_FOCUS
            } else {
                col::DIM_INPUT_BORDER
            },
        ))
        .inner_margin(egui::Margin::symmetric(5, 3))
        .corner_radius(3);

    let computed = if has_errors {
        None
    } else if angle {
        let unit = match sketch {
            Some(s) => crate::model::effective_angle_unit(doc, s),
            None => doc.default_angle_unit,
        };
        crate::value::computed_angle_in_doc(text, doc)
            .filter(|_| show_computed_row)
            .map(|v| crate::value::format_angle_display_in(v, unit))
    } else {
        let unit = match sketch {
            Some(s) => crate::model::effective_length_unit(doc, s),
            None => doc.default_length_unit,
        };
        computed_length_in_doc(text, doc)
            .filter(|_| show_computed_row)
            .map(|v| crate::value::format_length_display_in(v, unit))
    };
    let text_width = dim_input_text_width(text);

    let frame_output = frame.show(ui, |ui| {
        ui.set_width(text_width);
        ui.vertical_centered(|ui| {
            if let Some(v) = computed {
                ui.label(
                    egui::RichText::new(v)
                        .font(egui::FontId::monospace(11.0))
                        .color(col::DIM_INPUT_TEXT.gamma_multiply(0.65)),
                );
            } else if show_computed_row {
                ui.add_space(14.0);
            }
            ui.style_mut().spacing.text_edit_width = text_width;
            ui.visuals_mut().selection.bg_fill = col::DIM_INPUT_SELECTION;
            egui::TextEdit::singleline(text)
                .id(id)
                .frame(false)
                .desired_width(text_width)
                .font(egui::FontId::monospace(13.0))
                .text_color(if has_errors {
                    INVALID_TEXT
                } else if has_focus {
                    col::DIM_INPUT_TEXT_FOCUS
                } else {
                    col::DIM_INPUT_TEXT
                })
                .margin(egui::vec2(0.0, 0.0))
                .show(ui)
        })
        .inner
    });
    let output = frame_output.inner;
    if output.response.has_focus() {
        let cursor = output
            .state
            .cursor
            .char_range()
            .map(|range| range.primary.index)
            .unwrap_or_else(|| text.chars().count());
        if expression_autocomplete_show_dropdown(
            ui,
            ctx,
            &output.response,
            id,
            text,
            doc,
            &[],
            cursor,
        ) {
            output.state.clone().store(ctx, id);
        }
    }
    show_expression_error_tooltips_above(ui, &frame_output.response, &field_errors);
    let resp = &output.response;
    if is_focus_target && *pending_focus {
        resp.request_focus();
    }
    if should_select_all_rect_value(
        resp.gained_focus(),
        resp.has_focus(),
        is_focus_target,
        *pending_focus,
        user_edited,
        resp.changed(),
    ) {
        let len = text.chars().count();
        let mut state = output.state;
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::default(),
            egui::text::CCursor::new(len),
        )));
        state.store(ctx, id);
    }
    if is_focus_target && resp.has_focus() {
        *pending_focus = false;
    }
    let enter_commit = sketch_dimension_enter_pressed(ui) && resp.has_focus();
    if enter_commit {
        consume_sketch_dimension_enter(ui);
    }
    let lost_focus = resp.lost_focus();
    let mut inline_parameter_added = None;
    let mut inline_parameter_error = None;
    if enter_commit || lost_focus {
        match crate::parameters::try_commit_inline_parameter_definition(doc, text) {
            Ok(Some(outcome)) => inline_parameter_added = Some(outcome),
            Ok(None) => {}
            Err(error) => inline_parameter_error = Some(error),
        }
    }
    SketchDimFieldResult {
        changed: resp.changed(),
        enter_commit,
        lost_focus,
        inline_parameter_added,
        inline_parameter_error,
    }
}

fn apply_dimension_field_feedback(state: &mut AppState, result: &SketchDimFieldResult) {
    if let Some(outcome) = &result.inline_parameter_added {
        state.refresh_document_health();
        state.status = outcome.status_message();
    } else if let Some(error) = &result.inline_parameter_error {
        state.status = error.clone();
    }
}

fn sketch_plane_point(
    cam: &camera::Camera,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    doc: &model::Document,
    session: SketchSession,
    screen: egui::Pos2,
) -> Option<Vec3> {
    let face = doc.sketch_face(session.sketch)?;
    let frame = sketch_frame(doc, face)?;
    cam.ray_plane_hit(screen, viewport, vp, frame.origin, frame.normal)
}

/// A sensible default font family for a newly placed text (#282): a common sans/serif if
/// installed, else the first available system font. `None` only if no fonts are installed.
fn default_text_font() -> Option<String> {
    for fam in ["Helvetica", "Arial", "Segoe UI", "DejaVu Sans", "Liberation Sans"] {
        if crate::text::font_bytes(fam, false, false).is_some() {
            return Some(fam.to_string());
        }
    }
    crate::text::system_font_families().into_iter().next()
}

/// Toggle `value` in/out of `set` (add if absent, remove if present). Small helper for the
/// two-role in-sketch pickers (#238).
fn toggle(set: &mut Vec<usize>, value: usize) {
    if let Some(pos) = set.iter().position(|v| *v == value) {
        set.remove(pos);
    } else {
        set.push(value);
    }
}

/// The closed sketch face (its boundary-loop line indices) whose interior contains `world`, or
/// `None` if the point isn't inside any face (#238). Prefers the **smallest-area** containing loop
/// so clicking a region inside a hole picks the inner face, not the enclosing one.
fn face_loop_at_world(
    doc: &model::Document,
    sketch: model::SketchId,
    world: Vec3,
) -> Option<Vec<usize>> {
    let frame = crate::face::sketch_geometry_frame(doc, sketch)?;
    let (u, v) = world_to_local(&frame, world);
    let mut best: Option<(f32, Vec<usize>)> = None;
    for lines in crate::polygon::closed_line_loops(doc, sketch) {
        let Some(verts) = crate::polygon::loop_vertices_uv(doc, sketch, &lines) else {
            continue;
        };
        if !crate::polygon::point_in_polygon_2d((u, v), &verts) {
            continue;
        }
        // Shoelace area; smaller wins so an inner face is preferred over the region enclosing it.
        let mut area = 0.0f32;
        for i in 0..verts.len() {
            let j = (i + 1) % verts.len();
            area += verts[i].0 * verts[j].1 - verts[j].0 * verts[i].1;
        }
        let area = area.abs() * 0.5;
        if best.as_ref().map_or(true, |(a, _)| area < *a) {
            best = Some((area, lines));
        }
    }
    best.map(|(_, lines)| lines)
}

fn rectangle_dim_layout_from_corners(
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: [Vec3; 4],
    width_text: &str,
    height_text: &str,
) -> Option<(DimInputLayout, DimInputLayout)> {
    let bottom_mid = project(corners[0].lerp(corners[1], 0.5))?;
    let left_mid = project(corners[0].lerp(corners[3], 0.5))?;
    Some(rectangle_dim_layouts(
        bottom_mid,
        left_mid,
        width_text,
        height_text,
    ))
}

fn rect_highlight_edge(corners: [Vec3; 4], edge: RectDimEdge) -> (Vec3, Vec3) {
    match edge {
        RectDimEdge::Width => (corners[0], corners[1]),
        RectDimEdge::Height => (corners[0], corners[3]),
    }
}

fn push_circle_diameter_dim_layout(
    layouts: &mut Vec<CommittedDimLayout>,
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    label_view: &PlanarLabelView,
    frame: &face::SketchFrame,
    circle: &Circle,
    target: DimLabelTarget,
    a: Vec3,
    b: Vec3,
    stored_label_offset: Option<f32>,
    label: String,
) {
    let color = col::DIM_ANNOTATION;
    let (ua, va) = world_to_local(frame, a);
    let (ub, vb) = world_to_local(frame, b);
    let outward_uv = outward_perpendicular_uv(ua, va, ub, vb, circle.cx, circle.cy);
    let outward_world = uv_dir_to_world(frame.u_axis, frame.v_axis, outward_uv.0, outward_uv.1);
    if outward_world.length_squared() < 1e-8 {
        return;
    }
    let galley = painter.layout_no_wrap(
        label.clone(),
        egui::FontId::proportional(LABEL_FONT_SIZE),
        color,
    );
    let galley_size = galley.size();
    let diameter_px = project(a)
        .zip(project(b))
        .map(|(pa, pb)| (pb - pa).length())
        .unwrap_or(0.0);
    let label_outward_px = circle_diameter_label_outward_px(
        diameter_px,
        galley_size.x,
        galley_size.y,
        stored_label_offset,
    );
    let world_geom = circle_diameter_dimension_world_geom(
        a,
        b,
        outward_world,
        label_outward_px,
        galley_size.y,
        &project,
    );
    let Some(geom) = project_linear_dimension_geom(&world_geom, &project) else {
        return;
    };
    let label_rect = planar_dimension_label_layout(
        painter,
        &world_geom,
        label_view,
        &label,
        color,
        &project,
    );
    layouts.push(CommittedDimLayout {
        target,
        geom,
        world_geom,
        arc_geom: None,
        angle_display: None,
        angle_radius_world: 0.0,
        label,
        label_rect,
        outward: geom.outward,
        offset: label_outward_px,
    });
}

fn push_arc_dim_layout(
    layouts: &mut Vec<CommittedDimLayout>,
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    frame: &face::SketchFrame,
    doc: &model::Document,
    target: DimLabelTarget,
    line_a: model::ConstraintLine,
    line_b: model::ConstraintLine,
    rotation_sign: model::ConstraintSign,
    dim_offset: Option<f32>,
    label: String,
) {
    let Some(display) = angle_constraint_display(doc, line_a, line_b, rotation_sign) else {
        return;
    };
    let center = display.center;
    let dir_a = display.dir_a;
    let dir_b = display.dir_b;
    let plane_normal = frame.normal;
    let pixel_offset = effective_arc_dim_offset(dim_offset);
    let radius_world = pixels_to_world_distance(&project, center, dir_a, pixel_offset);
    let label_outset_world = pixels_to_world_distance(&project, center, dir_a, LABEL_OUTSET);
    let Some(world_geom) = arc_dimension_world_geom(
        center,
        dir_a,
        dir_b,
        plane_normal,
        radius_world,
        label_outset_world,
    ) else {
        return;
    };
    let Some(arc_geom) = project_arc_dimension_geom(&world_geom, &project) else {
        return;
    };
    let color = col::DIM_ANNOTATION;
    let label_rect = {
        let galley = painter.layout_no_wrap(
            label.clone(),
            egui::FontId::proportional(LABEL_FONT_SIZE),
            color,
        );
        egui::Rect::from_center_size(arc_geom.label_center, galley.size())
            .expand(dimensions::LABEL_HIT_PAD)
    };
    let outward = dimensions::arc_label_outward_screen(&arc_geom);
    layouts.push(CommittedDimLayout {
        target,
        geom: dimensions::LinearDimensionGeom {
            ext_a_near: arc_geom.start,
            ext_a_far: arc_geom.start,
            ext_b_near: arc_geom.end,
            ext_b_far: arc_geom.end,
            dim_a: arc_geom.start,
            dim_b: arc_geom.end,
            label_center: arc_geom.label_center,
            along: (arc_geom.end - arc_geom.start).normalized(),
            outward,
        },
        world_geom: dimensions::LinearDimensionWorldGeom {
            ext_a_near: center,
            ext_a_far: center,
            ext_b_near: center,
            ext_b_far: center,
            dim_a: center,
            dim_b: center,
            label_center: world_geom.label_center,
            along_world: dir_a,
            outward_world: plane_normal,
        },
        arc_geom: Some(arc_geom),
        angle_display: Some(display),
        angle_radius_world: radius_world,
        label,
        label_rect,
        outward,
        offset: pixel_offset,
    });
}

fn committed_dim_layout(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    label_view: &PlanarLabelView,
    frame: &face::SketchFrame,
    target: DimLabelTarget,
    a: Vec3,
    b: Vec3,
    outward_uv: (f32, f32),
    pixel_offset: f32,
    label: String,
) -> Option<CommittedDimLayout> {
    let color = col::DIM_ANNOTATION;
    let outward_world = uv_dir_to_world(frame.u_axis, frame.v_axis, outward_uv.0, outward_uv.1);
    if outward_world.length_squared() < 1e-8 {
        return None;
    }
    let anchor = a.lerp(b, 0.5);
    let offset_world = pixels_to_world_distance(&project, anchor, outward_world, pixel_offset);
    let overshoot_world =
        pixels_to_world_distance(&project, anchor, outward_world, EXTENSION_OVERSHOOT);
    let label_outset_world =
        pixels_to_world_distance(&project, anchor, outward_world, LABEL_OUTSET);
    let world_geom = linear_dimension_world_geom(
        a,
        b,
        outward_world,
        offset_world,
        overshoot_world,
        label_outset_world,
    );
    let geom = project_linear_dimension_geom(&world_geom, &project)?;
    let label_rect = planar_dimension_label_layout(
        painter,
        &world_geom,
        label_view,
        &label,
        color,
        &project,
    );
    Some(CommittedDimLayout {
        target,
        geom,
        world_geom,
        arc_geom: None,
        angle_display: None,
        angle_radius_world: 0.0,
        label,
        label_rect,
        outward: geom.outward,
        offset: pixel_offset,
    })
}

fn build_committed_dim_layouts(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    label_view: &PlanarLabelView,
    doc: &model::Document,
    session: SketchSession,
) -> Vec<CommittedDimLayout> {
    let Some(frame) = sketch_geometry_frame(doc, session.sketch) else {
        return Vec::new();
    };
    let mut layouts = Vec::new();
    // The sketch's local-space centroid: default label placement points *away* from it, so
    // labels land outside the drawn outline instead of piling into its interior.
    let sketch_centroid = {
        let mut sum = (0.0f32, 0.0f32);
        let mut n = 0usize;
        for (li, line) in doc.lines.iter().enumerate() {
            if line.sketch != session.sketch || !document_lifecycle::line_alive(doc, li) {
                continue;
            }
            sum.0 += line.x0 + line.x1;
            sum.1 += line.y0 + line.y1;
            n += 2;
        }
        (n > 0).then(|| (sum.0 / n as f32, sum.1 / n as f32))
    };
    for (index, constraint) in doc
        .constraints
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.deleted && c.sketch == session.sketch)
    {
        let ConstraintKind::Distance { target } = constraint.kind.clone() else {
            continue;
        };
        if matches!(target, DistanceTarget::CircleDiameter(_)) {
            continue;
        }
        let Some((a, b)) = constraint_segment_endpoints(doc, index) else {
            continue;
        };
        let (ua, va) = world_to_local(&frame, a);
        let (ub, vb) = world_to_local(&frame, b);
        let outward_uv = match sketch_centroid {
            Some((cu, cv)) => outward_perpendicular_uv(ua, va, ub, vb, cu, cv),
            None => preferred_outward_uv(ua, va, ub, vb),
        };
        let label = constraint_evaluated_length(doc, index)
            .map(|v| {
                crate::value::format_length_display_in(
                    v,
                    crate::model::effective_length_unit(doc, session.sketch),
                )
            })
            .unwrap_or_else(|| "?".to_string());
        // Stack colliding labels: a label without a user-stored offset that would land on
        // an already-placed one steps further out until it clears (or the offset cap).
        let base_offset = effective_dim_offset(constraint.dim_offset);
        let mut offset = base_offset;
        let mut candidate = committed_dim_layout(
            painter, &project, label_view, &frame, index, a, b, outward_uv, offset,
            label.clone(),
        );
        if constraint.dim_offset.is_none() {
            let collides = |c: &CommittedDimLayout, placed: &[CommittedDimLayout]| {
                c.label_rect != egui::Rect::NOTHING
                    && placed.iter().any(|l| {
                        l.label_rect != egui::Rect::NOTHING
                            && l.label_rect.intersects(c.label_rect.expand(2.0))
                    })
            };
            let mut tries = 0;
            while offset < dimensions::MAX_DIM_OFFSET
                && tries < 8
                && candidate.as_ref().is_some_and(|c| collides(c, &layouts))
            {
                offset = (offset + 26.0).min(dimensions::MAX_DIM_OFFSET);
                tries += 1;
                candidate = committed_dim_layout(
                    painter, &project, label_view, &frame, index, a, b, outward_uv, offset,
                    label.clone(),
                );
            }
        }
        if let Some(layout) = candidate {
            layouts.push(layout);
        }
    }
    for (index, constraint) in doc
        .constraints
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.deleted && c.sketch == session.sketch)
    {
        let ConstraintKind::Distance {
            target: DistanceTarget::CircleDiameter(i),
        } = constraint.kind.clone()
        else {
            continue;
        };
        let Some(circle) = doc.circles.get(i) else {
            continue;
        };
        let Some((a, b)) = constraint_segment_endpoints(doc, index) else {
            continue;
        };
        let label = constraint_evaluated_length(doc, index)
            .map(|v| {
                crate::value::format_diameter_display_in(
                    v,
                    crate::model::effective_length_unit(doc, session.sketch),
                )
            })
            .unwrap_or_else(|| "?".to_string());
        push_circle_diameter_dim_layout(
            &mut layouts,
            painter,
            &project,
            label_view,
            &frame,
            circle,
            index,
            a,
            b,
            constraint.dim_offset,
            label,
        );
    }
    for (index, constraint) in doc
        .constraints
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.deleted && c.sketch == session.sketch)
    {
        let ConstraintKind::Angle {
            line_a,
            line_b,
            rotation_sign,
        } = constraint.kind.clone()
        else {
            continue;
        };
        let label = constraint_evaluated_angle(doc, index)
            .map(|v| {
                crate::value::format_angle_display_in(
                    v,
                    crate::model::effective_angle_unit(doc, session.sketch),
                )
            })
            .unwrap_or_else(|| "?".to_string());
        push_arc_dim_layout(
            &mut layouts,
            painter,
            &project,
            &frame,
            doc,
            index,
            line_a,
            line_b,
            rotation_sign,
            constraint.dim_offset,
            label,
        );
    }
    layouts
}

fn draw_committed_dim_layouts<Project>(
    painter: &egui::Painter,
    layouts: &[CommittedDimLayout],
    label_view: &PlanarLabelView,
    project: &Project,
    health: &document_health::DocumentHealth,
    angle_gizmo_constraint: Option<DimLabelTarget>,
    hovered_angle_gizmo: Option<DimLabelTarget>,
    viewport: egui::Rect,
) where
    Project: Fn(Vec3) -> Option<egui::Pos2>,
{
    for layout in layouts {
        let color = document_health::constraint_annotation_color(
            health,
            layout.target,
            col::DIM_ANNOTATION,
        );
        if let (Some(arc_geom), Some(display)) =
            (&layout.arc_geom, layout.angle_display.as_ref())
        {
            let show_gizmo = angle_gizmo_constraint == Some(layout.target);
            let gizmo_hovered = show_gizmo && hovered_angle_gizmo == Some(layout.target);
            // Keep the angle annotation/gizmo on screen: if the lines' meeting point projects
            // outside the viewport, slide the whole annotation to the padded edge.
            let offset = project(display.center)
                .map(|c| {
                    dimensions::angle_gizmo_viewport_offset(c, viewport, ANGLE_GIZMO_VIEWPORT_PAD)
                })
                .unwrap_or(egui::Vec2::ZERO);
            let shifted_arc;
            let arc_ref = if offset == egui::Vec2::ZERO {
                arc_geom
            } else {
                shifted_arc = arc_geom.translated(offset);
                &shifted_arc
            };
            let project_shifted = |w: Vec3| project(w).map(|p| p + offset);
            draw_angle_constraint_annotation(
                painter,
                &project_shifted,
                display,
                layout.world_geom.outward_world,
                arc_ref,
                &layout.label,
                color,
                layout.angle_radius_world,
                show_gizmo,
                gizmo_hovered,
            );
        } else {
            draw_linear_dimension(
                painter,
                &layout.geom,
                &layout.label,
                color,
                Some((&layout.world_geom, label_view, project)),
            );
        }
    }
}

/// Padding (px) keeping the clamped angle gizmo clear of the viewport edge.
const ANGLE_GIZMO_VIEWPORT_PAD: f32 = 48.0;

/// Pixel offset of the extrude-height dimension line from the measured edge.
const EXTRUDE_DIM_OFFSET: f32 = 24.0;

/// Draw a dimension line along one vertical edge of an in-progress extrusion when its
/// height is a constrained (typed) value, so the constraint reads like a sketch dimension.
fn draw_extrude_height_dimension<Project>(
    painter: &egui::Painter,
    project: &Project,
    doc: &model::Document,
    ce: &actions::CreatingExtrusion,
) where
    Project: Fn(Vec3) -> Option<egui::Pos2>,
{
    if !ce.user_edited || ce.faces.is_empty() {
        return;
    }
    let distance = ce.evaluated_distance(doc);
    if distance.abs() < 1e-4 {
        return;
    }
    let Some((corners, normal)) = extrude::face_profile_world(doc, &ce.faces[0]) else {
        return;
    };
    if corners.len() < 3 {
        return;
    }
    // One vertical edge of the prism: a base corner up to its extruded top.
    let pa = corners[0];
    let pb = pa + normal * distance;
    // Offset the dimension line away from the solid, within the sketch plane.
    let center = corners
        .iter()
        .fold(Vec3::ZERO, |acc, c| acc + *c)
        / corners.len() as f32;
    let outward_world = (pa - center).normalize_or_zero();
    if outward_world.length_squared() < 1e-8 {
        return;
    }
    let anchor = pa.lerp(pb, 0.5);
    let offset_world = pixels_to_world_distance(project, anchor, outward_world, EXTRUDE_DIM_OFFSET);
    let overshoot_world =
        pixels_to_world_distance(project, anchor, outward_world, EXTENSION_OVERSHOOT);
    let label_outset_world =
        pixels_to_world_distance(project, anchor, outward_world, LABEL_OUTSET);
    let world_geom = linear_dimension_world_geom(
        pa,
        pb,
        outward_world,
        offset_world,
        overshoot_world,
        label_outset_world,
    );
    let Some(geom) = project_linear_dimension_geom(&world_geom, project) else {
        return;
    };
    let label = crate::value::format_length_display_in(
        distance.abs(),
        crate::model::effective_length_unit(doc, ce.sketch),
    );
    draw_linear_dimension::<fn(Vec3) -> Option<egui::Pos2>>(
        painter,
        &geom,
        &label,
        col::DIM_ANNOTATION,
        None,
    );
}

fn angle_gizmo_hit_target(
    layouts: &[CommittedDimLayout],
    pointer: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    angle_gizmo_constraint: Option<DimLabelTarget>,
    viewport: egui::Rect,
) -> Option<DimLabelTarget> {
    let active = angle_gizmo_constraint?;
    layouts.iter().rev().find_map(|layout| {
        if layout.target != active {
            return None;
        }
        let display = layout.angle_display.as_ref()?;
        // Match the on-screen clamping used when drawing, so the handle stays grabbable.
        let offset = project(display.center)
            .map(|c| dimensions::angle_gizmo_viewport_offset(c, viewport, ANGLE_GIZMO_VIEWPORT_PAD))
            .unwrap_or(egui::Vec2::ZERO);
        let project_shifted = |w: Vec3| project(w).map(|p| p + offset);
        let handle = angle_gizmo_handle_world(display, layout.angle_radius_world);
        angle_gizmo_handle_hit(pointer, &project_shifted, handle).then_some(layout.target)
    })
}

fn draw_angle_dim_for_lines<Project>(
    painter: &egui::Painter,
    project: &Project,
    frame: &face::SketchFrame,
    doc: &model::Document,
    line_a: model::ConstraintLine,
    line_b: model::ConstraintLine,
    rotation_sign: model::ConstraintSign,
    dim_offset: Option<f32>,
    label: &str,
    show_gizmo: bool,
    gizmo_hovered: bool,
) where
    Project: Fn(Vec3) -> Option<egui::Pos2>,
{
    let Some(display) = angle_constraint_display(doc, line_a, line_b, rotation_sign) else {
        return;
    };
    let pixel_offset = effective_arc_dim_offset(dim_offset);
    let radius_world =
        pixels_to_world_distance(&project, display.center, display.dir_a, pixel_offset);
    let label_outset_world =
        pixels_to_world_distance(&project, display.center, display.dir_a, LABEL_OUTSET);
    let Some(world_geom) = arc_dimension_world_geom(
        display.center,
        display.dir_a,
        display.dir_b,
        frame.normal,
        radius_world,
        label_outset_world,
    ) else {
        return;
    };
    let Some(arc_geom) = project_arc_dimension_geom(&world_geom, &project) else {
        return;
    };
    draw_angle_constraint_annotation(
        painter,
        project,
        &display,
        frame.normal,
        &arc_geom,
        label,
        col::DIM_ANNOTATION,
        radius_world,
        show_gizmo,
        gizmo_hovered,
    );
}

fn pointer_over_committed_dim_label(
    layouts: &[CommittedDimLayout],
    pointer: egui::Pos2,
) -> bool {
    layouts.iter().any(|l| l.label_rect.contains(pointer))
}

fn dim_input_layout_centered_on(label_rect: egui::Rect, text: &str) -> DimInputLayout {
    let size = dim_input_size_for_text(text);
    let pos = label_rect.center() - size * 0.5;
    layout_at(pos, size)
}

fn handle_committed_dim_label_double_click(
    ui: &egui::Ui,
    layouts: &[CommittedDimLayout],
    state: &mut AppState,
) -> bool {
    if !state.can_edit_sketch_dimensions() || state.editing_committed_dim.is_some() {
        return false;
    }
    if !ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
        return false;
    }
    let Some(pos) = ui.input(|i| i.pointer.hover_pos()) else {
        return false;
    };
    let Some(hit) = layouts.iter().rev().find(|h| h.label_rect.contains(pos)) else {
        return false;
    };
    state.apply(Action::BeginEditCommittedDim { target: hit.target });
    true
}

/// The extrude-able face (rectangle/circle) under the cursor, if any. When the picked shape
/// overlaps exactly one other raw shape in its sketch (#16/#62), resolves the click to the
/// right atomic boolean region (their intersection, or one minus the other) instead of the
/// whole picked shape — see `extrude::overlapping_partner`/`resolve_boolean_click`. Any other
/// case (no overlap, ambiguous 3+-way overlap, or the click landing outside both loops) falls
/// back to today's whole-shape picking.
fn pick_extrude_face(
    pp: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    eye: Vec3,
    cam: &camera::Camera,
    viewport: egui::Rect,
    vp: &glam::Mat4,
) -> Option<model::ExtrudeFace> {
    let base = match pick_sketch_face(pp, project, doc, eye)? {
        FaceId::Circle(i) => model::ExtrudeFace::Circle(i),
        FaceId::Polygon(lines) => model::ExtrudeFace::Polygon(lines),
        FaceId::ConstructionPlane(_) | FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. } => {
            return None;
        }
    };
    if let Some(resolved) = resolve_boolean_extrude_face(doc, &base, pp, cam, viewport, vp) {
        return Some(resolved);
    }
    Some(base)
}

/// The bare 3D body face (cap or side wall — never a sketch profile) under the cursor, if
/// any (#122): the fallback the Extrude tool tries when [`pick_extrude_face`] finds nothing,
/// so a solid's own face can be pushed/pulled directly, no separate sketch needed.
fn pick_extrude_body_face(
    pp: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    eye: Vec3,
) -> Option<FaceId> {
    match pick_sketch_face(pp, project, doc, eye)? {
        face_id @ (FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. }) => Some(face_id),
        _ => None,
    }
}

/// If `face`'s sketch has exactly one other shape overlapping it, and `pp` (in screen space)
/// lands within their combined footprint, the atomic boolean region the click landed in.
fn resolve_boolean_extrude_face(
    doc: &model::Document,
    face: &model::ExtrudeFace,
    pp: egui::Pos2,
    cam: &camera::Camera,
    viewport: egui::Rect,
    vp: &glam::Mat4,
) -> Option<model::ExtrudeFace> {
    let sketch = actions::extrude_face_sketch(doc, face)?;
    let other = extrude::overlapping_partner(doc, sketch, face)?;
    let frame = sketch_geometry_frame(doc, sketch)?;
    let world = cam.ray_plane_hit(pp, viewport, vp, frame.origin, frame.normal)?;
    let point = world_to_local(&frame, world);
    extrude::resolve_boolean_click(doc, sketch, face, &other, point)
}

fn extrude_face_id(face: model::ExtrudeFace) -> FaceId {
    face.face_id()
}

/// Object under the cursor to extrude up to (vertex preferred, then face/plane), with the
/// signed distance from the extrusion base to its extended plane. Excludes the faces being
/// extruded.
/// Distance, in sketch units, that the extrude gizmo handle floats above the
/// solid's top face so it sits a little above the surface rather than on it.
const EXTRUDE_GIZMO_LIFT: f32 = 4.0;
/// Length (sketch mm) of the in-sketch Move gizmo's u/v push-pull arrows (#306).
const SKETCH_MOVE_ARROW_MM: f32 = 12.0;

/// World position of the revolve arc gizmo's push/pull handle (#262): `zero_dir` rotated
/// `angle_deg` around `axis`, scaled out to `radius` from `center`.
fn revolve_arc_handle_world(
    center: Vec3,
    axis: Vec3,
    zero_dir: Vec3,
    radius: f32,
    angle_deg: f32,
) -> Vec3 {
    let rot = glam::Quat::from_axis_angle(axis, angle_deg.to_radians());
    center + (rot * zero_dir) * radius
}

/// Map a cursor to a revolve sweep angle by its angular position around the arc, measured in
/// screen space relative to the 0° direction (#262). Returns degrees in `[1, 360]`, or `None`
/// if the gizmo center/reference don't project. `sign_probe` is a point a small positive angle
/// along the sweep, used to fix which screen rotation direction counts as increasing angle.
fn revolve_arc_angle_from_cursor(
    cursor: egui::Pos2,
    center: Vec3,
    zero_world: Vec3,
    sign_probe: Vec3,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Option<f32> {
    let c = project(center)?;
    let z = project(zero_world)?;
    let probe = project(sign_probe)?;
    let base = (z - c).angle();
    // Which screen direction is "positive" sweep: the sign of the probe's offset from base.
    let probe_delta = wrap_signed_radians((probe - c).angle() - base);
    let positive_ccw = probe_delta >= 0.0;
    let raw = wrap_signed_radians((cursor - c).angle() - base);
    // Fold to a 0..2π sweep in the positive direction.
    let mut sweep = if positive_ccw { raw } else { -raw };
    if sweep < 0.0 {
        sweep += std::f32::consts::TAU;
    }
    let deg = sweep.to_degrees();
    Some(deg.clamp(1.0, 360.0))
}

/// Wrap an angle in radians to `(-π, π]`.
fn wrap_signed_radians(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a <= -PI {
        a += TAU;
    }
    a
}

/// Screen-space grab radius (px) for the revolve arc gizmo's push/pull handle (#262).
const REVOLVE_ARC_HANDLE_PICK_PX: f32 = 12.0;

/// egui id of the floating extrude-distance text field.
const REVOLVE_ANGLE_FIELD_ID: &str = "revolve_angle_field";
const EXTRUDE_DISTANCE_FIELD_ID: &str = "extrude_distance_input";

/// egui id of the floating chamfer/fillet amount text field.
const VERTEX_TREATMENT_AMOUNT_FIELD_ID: &str = "vertex_treatment_amount_input";

/// egui id of the floating 3D edge chamfer/fillet amount text field (#77).
const EDGE_TREATMENT_AMOUNT_FIELD_ID: &str = "edge_treatment_amount_input";

/// World-space origin (the vertex) and normal (inward bisector of the two adjoining lines,
/// pointing into the corner so pulling the gizmo away from the vertex increases the amount)
/// for the chamfer/fillet gizmo, given the picked vertex. `None` if the vertex no longer joins
/// exactly two plain lines.
fn vertex_treatment_anchor(
    doc: &model::Document,
    sketch: model::SketchId,
    point: ConstraintPoint,
) -> Option<(Vec3, Vec3)> {
    let frame = sketch_geometry_frame(doc, sketch)?;
    let corner = vertex_drag::treatment_corner(doc, sketch, point)?;
    let (v, a, b) = (corner.v, corner.a, corner.b);
    let dist_va = ((a.0 - v.0).powi(2) + (a.1 - v.1).powi(2)).sqrt();
    let dist_vb = ((b.0 - v.0).powi(2) + (b.1 - v.1).powi(2)).sqrt();
    if dist_va < 1e-6 || dist_vb < 1e-6 {
        return None;
    }
    let dir_a = ((a.0 - v.0) / dist_va, (a.1 - v.1) / dist_va);
    let dir_b = ((b.0 - v.0) / dist_vb, (b.1 - v.1) / dist_vb);
    let dir_a_world = frame.u_axis * dir_a.0 + frame.v_axis * dir_a.1;
    let dir_b_world = frame.u_axis * dir_b.0 + frame.v_axis * dir_b.1;
    let normal = (dir_a_world + dir_b_world).normalize_or_zero();
    if normal.length_squared() < 1e-8 {
        return None;
    }
    let origin = face::local_to_world(&frame, v.0, v.1);
    Some((origin, normal))
}

/// World-space preview polyline for the in-progress chamfer/fillet, recomputed every frame from
/// the *live* gizmo amount so dragging the handle visibly resizes the cut/round before commit
/// (#76). Traces the treated corner end to end: the first line's far endpoint, the truncated
/// point, the bridge (straight for a chamfer, sampled from the fillet's bezier — reuses
/// [`Line::sample_local`] so the preview matches the actual bridge geometry
/// [`Action::CommitVertexTreatment`] will create), the other truncated point, and the second
/// line's far endpoint. `None` while the corner can't be treated (e.g. the live amount is zero,
/// or the vertex no longer joins exactly two lines) — callers should just skip drawing.
fn vertex_treatment_preview_points(
    doc: &model::Document,
    sketch: model::SketchId,
    cvt: &CreatingVertexTreatment,
) -> Option<Vec<Vec3>> {
    let frame = sketch_geometry_frame(doc, sketch)?;
    let corner = vertex_drag::treatment_corner(doc, sketch, cvt.point.clone())?;
    let amount = cvt.evaluated_amount(doc);
    let geom = model::vertex_treatment_geometry(corner.v, corner.a, corner.b, cvt.kind, amount)?;

    let mut bridge =
        Line::from_local_endpoints(sketch, geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
    bridge.bezier = geom.bezier;

    let mut local_points = Vec::with_capacity(model::BEZIER_SEGMENTS + 3);
    local_points.push(corner.a);
    local_points.extend(bridge.sample_local(model::BEZIER_SEGMENTS));
    local_points.push(corner.b);

    Some(
        local_points
            .into_iter()
            .map(|(u, v)| face::local_to_world(&frame, u, v))
            .collect(),
    )
}

/// Where the extrude gizmo handle is drawn along the normal: the actual extrude
/// distance plus a small lift in the extrusion direction.
fn extrude_gizmo_display_offset(distance: f32) -> f32 {
    let dir = if distance < 0.0 { -1.0 } else { 1.0 };
    distance + dir * EXTRUDE_GIZMO_LIFT
}

fn pick_extrude_target(
    pp: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    base: Vec3,
    normal: Vec3,
    exclude: &[model::ExtrudeFace],
    eye: Vec3,
    editing: Option<usize>,
) -> Option<(model::ExtrudeTarget, f32)> {
    use model::ExtrudeTarget;
    const VERTEX_RADIUS_PX: f32 = 12.0;

    // Nearest vertex.
    let mut best: Option<(f32, ExtrudeTarget)> = None;
    for vertex in snapping::all_sketch_vertices(doc) {
        if let Some(world) = extrude::constraint_point_world(doc, vertex.clone()) {
            if let Some(sp) = project(world) {
                let d = (sp - pp).length();
                if d <= VERTEX_RADIUS_PX && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, ExtrudeTarget::Vertex(vertex)));
                }
            }
        }
    }

    let target = if let Some((_, t)) = best {
        t
    } else {
        match pick_sketch_face(pp, project, doc, eye)? {
            FaceId::Circle(i) if !exclude.contains(&model::ExtrudeFace::Circle(i)) => {
                ExtrudeTarget::Face(model::ExtrudeFace::Circle(i))
            }
            FaceId::ConstructionPlane(i) => ExtrudeTarget::Plane(i),
            // Another (or, unless it's the extrusion being edited, the same) body's cap/side
            // wall is a valid snap target (#126) — excluded only when it belongs to the
            // extrusion currently being pulled, which would be a meaningless self-reference.
            face_id @ (FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. })
                if editing != Some(extrusion) =>
            {
                ExtrudeTarget::BodyFace(face_id)
            }
            _ => return None,
        }
    };
    let dist = extrude::target_distance(doc, base, normal, &target)?;
    Some((target, dist))
}

/// Snap radius in screen pixels, converted to sketch units per the current view.
const SNAP_RADIUS_PX: f32 = 12.0;

/// The snap radius in sketch-local units near `world` on the sketch plane.
fn snap_radius_uv(
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    frame: &face::SketchFrame,
    world: Vec3,
) -> f32 {
    pixels_to_world_distance(project, world, frame.u_axis, SNAP_RADIUS_PX)
}

/// World position and target of the active snap (dragged vertex or line end), for the marker.
fn active_snap(
    state: &AppState,
    sketch: SketchId,
    frame: &face::SketchFrame,
) -> Option<(Vec3, snapping::SnapTarget)> {
    if let Some((point, target)) = state.active_snap.clone() {
        let (u, v) = crate::geometric_constraints::point_uv(&state.doc, sketch, point).ok()?;
        return Some((face::local_to_world(frame, u, v), target));
    }
    if let Some(target) = state.line_end_snap.clone() {
        if let Some(cl) = &state.creating_line {
            return Some((cl.end_point(frame, &state.doc), target));
        }
    }
    if let Some(target) = state.rect_opposite_snap.clone() {
        if let Some(cr) = &state.creating_rect {
            return Some((cr.end_point(frame, &state.doc), target));
        }
    }
    None
}

/// The constraint icon representing a snap target.
fn snap_icon(target: snapping::SnapTarget) -> icons::IconId {
    match target {
        snapping::SnapTarget::Midpoint(_) => icons::IconId::Midpoint,
        snapping::SnapTarget::Vertex(_)
        | snapping::SnapTarget::Origin
        | snapping::SnapTarget::OnLine(_)
        | snapping::SnapTarget::OnLineExtension(_) => icons::IconId::Coincident,
        snapping::SnapTarget::NormalAtMidpoint(_) => icons::IconId::Perpendicular,
    }
}

/// Snap a world-space sketch-plane point to nearby geometry, returning the (possibly snapped)
/// world point and the snap target it latched onto.
fn snap_ground_point(
    state: &AppState,
    session: SketchSession,
    frame: &face::SketchFrame,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    world: Vec3,
    exclude: &[ConstraintPoint],
) -> (Vec3, Option<snapping::SnapTarget>) {
    if !state.snapping_enabled {
        return (world, None);
    }
    let (u, v) = world_to_local(frame, world);
    let radius = snap_radius_uv(project, frame, world);
    if let Some(snap) = snapping::find_snap(&state.doc, session.sketch, (u, v), radius, exclude) {
        return (
            face::local_to_world(frame, snap.uv.0, snap.uv.1),
            Some(snap.target),
        );
    }
    // No direct snap: fall back to the extension guides of the last-hovered vertex (#21),
    // letting the point latch onto the infinite extension of those edges.
    if !state.extension_anchors.is_empty() {
        if let Some(snap) = snapping::find_extension_snap(
            &state.doc,
            session.sketch,
            &state.extension_anchors,
            (u, v),
            radius,
            exclude,
        ) {
            return (
                face::local_to_world(frame, snap.uv.0, snap.uv.1),
                Some(snap.target),
            );
        }
    }
    // Still nothing: fall back to the normal-through-midpoint guide of the last-touched
    // midpoint (#41), letting the point latch onto that infinite perpendicular line.
    if state.normal_inference_anchor.is_some() {
        if let Some(snap) = snapping::find_normal_at_midpoint_snap(
            &state.doc,
            session.sketch,
            state.normal_inference_anchor.clone(),
            (u, v),
            radius,
            exclude,
        ) {
            return (
                face::local_to_world(frame, snap.uv.0, snap.uv.1),
                Some(snap.target),
            );
        }
    }
    (world, None)
}

/// Update the active inference-snap guides from the latest snap result: hovering a real vertex
/// makes its incident edges the extension anchors (#21); hovering a line's midpoint makes that
/// line the normal-at-midpoint anchor (#41). Other snaps leave both guides in place so the user
/// can pull away from the touched vertex/midpoint and still snap to its guide line.
fn update_extension_anchors(state: &mut AppState, snap_target: Option<snapping::SnapTarget>) {
    match snap_target {
        Some(snapping::SnapTarget::Vertex(point)) => {
            state.extension_anchors = snapping::vertex_extension_anchors(point);
        }
        Some(snapping::SnapTarget::Midpoint(line)) => {
            state.normal_inference_anchor = Some(line);
        }
        _ => {}
    }
}

fn handle_vertex_drag(
    ui: &egui::Ui,
    drag: &mut Option<VertexDrag>,
    state: &mut AppState,
    session: SketchSession,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    cam: &camera::Camera,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pointer_screen: Option<egui::Pos2>,
) -> bool {
    if state.creating_rect.is_some()
        || state.creating_line.is_some()
        || state.creating_circle.is_some()
        || state.editing_committed_dim.is_some()
    {
        *drag = None;
        return false;
    }

    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());

    if let Some(active) = drag.as_ref() {
        if primary_released {
            // Leaving a snapped vertex in place pins it with the implied constraint.
            if let Some((point, target)) = state.active_snap.take() {
                let _ = state.apply(Action::ApplySnapConstraint { point, target });
            }
            *drag = None;
            return false;
        }
        if primary_down {
            if let Some(pp) = pointer_screen {
                if let Some(world) =
                    sketch_plane_point(cam, viewport, vp, &state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch).unwrap();
                    let (mut u, mut v) = world_to_local(&frame, world);
                    state.active_snap = None;
                    if state.snapping_enabled {
                        let radius = snap_radius_uv(project, &frame, world);
                        let exclude = vertex_drag::coincident_group(
                            &state.doc,
                            session.sketch,
                            active.point.clone(),
                        );
                        if let Some(snap) = snapping::find_snap(
                            &state.doc,
                            session.sketch,
                            (u, v),
                            radius,
                            &exclude,
                        ) {
                            u = snap.uv.0;
                            v = snap.uv.1;
                            state.active_snap = Some((active.point.clone(), snap.target));
                        }
                    }
                    let _ = state.apply(Action::DragVertex {
                        point: active.point.clone(),
                        u,
                        v,
                    });
                }
            }
            return true;
        }
        *drag = None;
    }

    if primary_pressed {
        if let Some(pp) = pointer_screen {
            if let Some((point, _)) =
                nearest_sketch_point_in_sketch(pp, project, &state.doc, session.sketch)
            {
                let element = vertex_drag::scene_element_for_point(point.clone());
                if document_health::require_element_editable(&state.document_health, element)
                    .is_err()
                {
                    return false;
                }
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                let selectable = SceneElement::Point(point.clone());
                // Select on the first press; only start dragging a vertex that is already
                // selected, so a click selects it without moving it (#239). A second press-drag
                // then moves it.
                if state.scene_selection.is_selected(selectable.clone()) && !additive {
                    *drag = Some(VertexDrag { point });
                } else {
                    state.apply(Action::ClickSceneElement { element: selectable, additive });
                }
                return true;
            }
            // The origin is a fixed, selectable point (#189) — no drag. Only when no real
            // vertex is nearer (that pick returned above), so a coincident vertex still wins.
            if let Some(frame) = sketch_geometry_frame(&state.doc, session.sketch) {
                let near_origin = project(frame.origin)
                    .is_some_and(|op| (op - pp).length() <= construction::POINT_PICK_RADIUS_PX);
                if near_origin {
                    state.apply(Action::ClickSceneElement {
                        element: SceneElement::Origin,
                        additive: ui.input(|i| additive_click_modifiers(&i.modifiers)),
                    });
                    return true;
                }
            }
        }
    }

    false
}

fn handle_line_drag(
    ui: &egui::Ui,
    state: &mut AppState,
    session: SketchSession,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    cam: &camera::Camera,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pointer_screen: Option<egui::Pos2>,
) -> bool {
    if state.creating_rect.is_some()
        || state.creating_line.is_some()
        || state.creating_circle.is_some()
        || state.editing_committed_dim.is_some()
    {
        if state.line_drag_session.is_some() {
            let _ = state.apply(Action::EndLineDrag);
        }
        return false;
    }

    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());

    if state.line_drag_session.is_some() {
        if primary_released {
            let _ = state.apply(Action::EndLineDrag);
            return false;
        }
        if primary_down {
            if let Some(pp) = pointer_screen {
                if let Some(world) =
                    sketch_plane_point(cam, viewport, vp, &state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch).unwrap();
                    let (u, v) = world_to_local(&frame, world);
                    let _ = state.apply(Action::DragLine { u, v });
                }
            }
            return true;
        }
        let _ = state.apply(Action::EndLineDrag);
        return false;
    }

    if primary_pressed {
        if let Some(pp) = pointer_screen {
            if nearest_sketch_point_in_sketch(pp, project, &state.doc, session.sketch).is_some() {
                return false;
            }
            if let Some((target, _)) =
                nearest_sketch_line_in_sketch(pp, project, &state.doc, session.sketch)
            {
                let element = vertex_drag::scene_element_for_line(target.clone());
                // Fixed reference lines (origin axes, face edges) can't be dragged — just select
                // them (#241), without the editability gate the drag path needs.
                if !vertex_drag::can_drag_line(&state.doc, session.sketch, target.clone()) {
                    state.apply(Action::ClickSceneElement {
                        element,
                        additive: ui.input(|i| additive_click_modifiers(&i.modifiers)),
                    });
                    return true;
                }
                if document_health::require_element_editable(&state.document_health, element.clone())
                    .is_err()
                {
                    return false;
                }
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                // Select on the first press; only drag a line that is already selected, so a
                // click selects without moving it (#239).
                if !(state.scene_selection.is_selected(element.clone()) && !additive) {
                    state.apply(Action::ClickSceneElement { element, additive });
                    return true;
                }
                if let Some(world) =
                    sketch_plane_point(cam, viewport, vp, &state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch).unwrap();
                    let (u, v) = world_to_local(&frame, world);
                    let _ = state.apply(Action::BeginLineDrag {
                        target,
                        anchor_u: u,
                        anchor_v: v,
                    });
                    let _ = state.apply(Action::DragLine { u, v });
                    return true;
                }
            }
        }
    }

    false
}

/// Drag one of a curved [`Line`]'s two tangent handles (rendered only for lines whose
/// `bezier` field is set — the drag-to-curve gesture or right-click-to-curve conversion).
/// Width drag handles of a selected wrapped text box (#409): press on an edge handle grabs
/// it; while held, the pointer's baseline-space x resizes the wrap width (the left handle
/// also shifts the origin so the right edge stays put). Same press/drag/release shape as
/// `handle_bezier_handle_drag`.
#[allow(clippy::too_many_arguments)]
fn handle_text_width_drag(
    ui: &egui::Ui,
    drag: &mut Option<TextWidthDrag>,
    state: &mut AppState,
    selected_text: Option<usize>,
    session: SketchSession,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    cam: &camera::Camera,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pointer_screen: Option<egui::Pos2>,
) -> bool {
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());

    if let Some(active) = drag.as_ref() {
        if primary_released || !primary_down {
            *drag = None;
            return false;
        }
        if let Some(pp) = pointer_screen {
            if let Some(world) = sketch_plane_point(cam, viewport, vp, &state.doc, session, pp) {
                if let Some(text) = state
                    .doc
                    .sketch_texts
                    .get(active.text)
                    .filter(|t| !t.deleted && t.wrap_width.is_some())
                {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch).unwrap();
                    let (u, v) = world_to_local(&frame, world);
                    let (bx, _) = crate::text::local_to_baseline(text, u, v);
                    let wrap = text.wrap_width.unwrap();
                    let (origin, new_wrap) = if active.left {
                        // Keep the right edge fixed: the origin slides with the handle.
                        let shift = bx.min(wrap - actions::MIN_TEXT_WRAP_MM);
                        let (ou, ov) = crate::text::baseline_to_local(text, shift, 0.0);
                        ((ou, ov), wrap - shift)
                    } else {
                        (text.origin, bx.max(actions::MIN_TEXT_WRAP_MM))
                    };
                    let _ = state.apply(Action::ResizeSketchText {
                        index: active.text,
                        origin,
                        wrap_width: new_wrap,
                    });
                }
            }
        }
        return true;
    }

    if primary_pressed {
        if let (Some(pp), Some(ti)) = (pointer_screen, selected_text) {
            let handle = state
                .doc
                .sketch_texts
                .get(ti)
                .filter(|t| !t.deleted && t.sketch == session.sketch)
                .and_then(crate::text::wrap_width_handles_local)
                .and_then(|handles| {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch)?;
                    let mut best: Option<(bool, f32)> = None;
                    for (left, (u, v)) in [(true, handles[0]), (false, handles[1])] {
                        let Some(sp) = project(face::local_to_world(&frame, u, v)) else {
                            continue;
                        };
                        let dist = (pp - sp).length();
                        if dist <= construction::POINT_PICK_RADIUS_PX
                            && best.is_none_or(|(_, d)| dist < d)
                        {
                            best = Some((left, dist));
                        }
                    }
                    best
                });
            if let Some((left, _)) = handle {
                *drag = Some(TextWidthDrag { text: ti, left });
                return true;
            }
        }
    }

    false
}

fn handle_bezier_handle_drag(
    ui: &egui::Ui,
    drag: &mut Option<BezierHandleDrag>,
    state: &mut AppState,
    session: SketchSession,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    cam: &camera::Camera,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    pointer_screen: Option<egui::Pos2>,
) -> bool {
    if state.creating_rect.is_some()
        || state.creating_line.is_some()
        || state.creating_circle.is_some()
        || state.editing_committed_dim.is_some()
    {
        *drag = None;
        return false;
    }

    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());

    if let Some(active) = drag.as_ref() {
        if primary_released {
            *drag = None;
            return false;
        }
        if primary_down {
            if let Some(pp) = pointer_screen {
                if let Some(world) =
                    sketch_plane_point(cam, viewport, vp, &state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&state.doc, session.sketch).unwrap();
                    let (u, v) = world_to_local(&frame, world);
                    let _ = state.apply(Action::SetBezierHandle {
                        line: active.line,
                        near_start: active.near_start,
                        u,
                        v,
                    });
                }
            }
            return true;
        }
        *drag = None;
        return false;
    }

    if primary_pressed {
        if let Some(pp) = pointer_screen {
            if let Some((line_index, near_start)) =
                nearest_bezier_handle_in_sketch(pp, project, &state.doc, session.sketch)
            {
                *drag = Some(BezierHandleDrag { line: line_index, near_start });
                return true;
            }
        }
    }

    false
}

fn nearest_bezier_handle_in_sketch(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    sketch: model::SketchId,
) -> Option<(usize, bool)> {
    let frame = sketch_geometry_frame(doc, sketch)?;
    let mut best: Option<(usize, bool, f32)> = None;
    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted || line.sketch != sketch {
            continue;
        }
        let Some([c0, c1]) = line.bezier else {
            continue;
        };
        for (near_start, (cu, cv)) in [(true, c0), (false, c1)] {
            let world = face::local_to_world(&frame, cu, cv);
            let Some(sp) = project(world) else {
                continue;
            };
            let dist = (screen - sp).length();
            if dist <= construction::POINT_PICK_RADIUS_PX
                && best.as_ref().is_none_or(|(_, _, d)| dist < *d)
            {
                best = Some((li, near_start, dist));
            }
        }
    }
    best.map(|(li, near_start, _)| (li, near_start))
}

/// Number of distinct plain lines meeting at `point` (via `Coincident` constraints) — a
/// right-clicked vertex only offers "Convert to bezier curve" when this is exactly 2.
fn vertex_incident_line_count(
    doc: &model::Document,
    sketch: model::SketchId,
    point: ConstraintPoint,
) -> usize {
    vertex_drag::coincident_group(doc, sketch, point)
        .into_iter()
        .filter_map(|p| match p {
            ConstraintPoint::LineEndpoint { line, .. } => Some(line),
            _ => None,
        })
        .collect::<HashSet<_>>()
        .len()
}

fn handle_angle_gizmo_drag(
    ui: &egui::Ui,
    layouts: &[CommittedDimLayout],
    drag: &mut Option<AngleGizmoDrag>,
    state: &mut AppState,
    session: SketchSession,
    viewport: egui::Rect,
    vp: &glam::Mat4,
    cam: &camera::Camera,
    angle_gizmo_constraint: DimLabelTarget,
) -> bool {
    if !state.can_edit_sketch_dimensions() || state.editing_committed_dim.is_none() {
        return false;
    }
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let Some(frame) = sketch_geometry_frame(&state.doc, session.sketch) else {
        return false;
    };

    if let Some(active) = drag.as_ref() {
        if primary_released {
            *drag = None;
            return false;
        }
        if primary_down {
            if let Some(pp) = pointer {
                if let Some(layout) =
                    layouts.iter().find(|l| l.target == active.constraint_id)
                {
                    if let Some(display) = layout.angle_display {
                        if let Some(hit) = cam.ray_plane_hit(
                            pp, viewport, vp, display.center, frame.normal,
                        ) {
                            if let Some(angle_rad) =
                                angle_rad_from_sketch_hit(&display, frame.normal, hit)
                            {
                                let _ = state.apply(Action::SetConstraintAngleValue {
                                    constraint_id: active.constraint_id,
                                    angle_rad,
                                });
                            }
                        }
                    }
                }
            }
            return true;
        }
        *drag = None;
    }

    if primary_pressed {
        if let Some(pos) = pointer {
            let project = |w: glam::Vec3| cam.project(w, viewport, vp);
            if let Some(target) =
                angle_gizmo_hit_target(layouts, pos, &project, Some(angle_gizmo_constraint), viewport)
            {
                if document_health::require_constraint_editable(
                    &state.document_health,
                    &state.doc,
                    target,
                )
                .is_err()
                {
                    return false;
                }
                *drag = Some(AngleGizmoDrag {
                    constraint_id: target,
                });
                return true;
            }
        }
    }

    false
}

fn handle_committed_dim_label_drag(
    ui: &egui::Ui,
    layouts: &[CommittedDimLayout],
    drag: &mut Option<DimLabelDrag>,
    state: &mut AppState,
) -> bool {
    if !state.can_edit_sketch_dimensions() || state.editing_committed_dim.is_some() {
        return false;
    }

    let pointer = ui.input(|i| i.pointer.hover_pos());
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let double_clicked =
        ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));

    if let Some(active) = drag.as_ref() {
        if primary_released || double_clicked {
            *drag = None;
            return !double_clicked;
        }
        if primary_down {
            if let Some(pos) = pointer {
                let moved = (pos - active.anchor_screen).length();
                if moved >= DIM_LABEL_DRAG_THRESHOLD_PX {
                    let delta = (pos - active.anchor_screen).dot(active.outward);
                    let offset = if constraint_is_circle_diameter(&state.doc, active.target) {
                        effective_circle_diameter_label_offset(Some(active.start_offset + delta))
                    } else if constraint_is_angle(&state.doc, active.target) {
                        effective_arc_dim_offset(Some(active.start_offset + delta))
                    } else {
                        effective_dim_offset(Some(active.start_offset + delta))
                    };
                    state.apply(Action::SetDimLabelOffset {
                        target: active.target,
                        offset,
                    });
                    return true;
                }
            }
            return layouts.iter().any(|layout| {
                pointer.is_some_and(|pos| layout.label_rect.contains(pos))
            });
        }
        *drag = None;
    }

    if primary_pressed && !double_clicked {
        if let Some(pos) = pointer {
            if let Some(hit) = layouts.iter().rev().find(|h| h.label_rect.contains(pos)) {
                if document_health::require_constraint_editable(
                    &state.document_health,
                    &state.doc,
                    hit.target,
                )
                .is_err()
                {
                    return false;
                }
                *drag = Some(DimLabelDrag {
                    target: hit.target,
                    outward: hit.outward,
                    start_offset: hit.offset,
                    anchor_screen: pos,
                });
                return true;
            }
        }
    }

    false
}

fn draw_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    face: FaceId,
    color: egui::Color32,
) {
    match face {
        FaceId::ConstructionPlane(i) => {
            if let Some(plane) = doc.construction_planes.get(i) {
                let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
                draw_quad_face_highlight(painter, project, corners, color);
            }
        }
        FaceId::Circle(i) => {
            if let Some(circle) = doc.circles.get(i) {
                draw_circle_face_highlight(painter, project, doc, circle, color);
            }
        }
        FaceId::Polygon(lines) => {
            if let Some((poly, _)) =
                extrude::face_profile_world(doc, &model::ExtrudeFace::Polygon(lines))
            {
                draw_polygon_face_highlight(painter, project, &poly, color);
            }
        }
        FaceId::ExtrudeCap {
            extrusion,
            profile,
            top,
        } => {
            if let Some(poly) = extrude::cap_polygon_world(doc, extrusion, &profile, top) {
                draw_polygon_face_highlight(painter, project, &poly, color);
            }
        }
        FaceId::ExtrudeSide {
            extrusion,
            profile,
            edge,
        } => {
            if let Some(quad) = extrude::side_quad_world(doc, extrusion, &profile, edge as usize) {
                draw_polygon_face_highlight(painter, project, &quad, color);
            }
        }
    }
}

/// Highlight the object an in-progress extrusion is currently snapping to (a vertex,
/// face, or plane), so the extrude-to-object target is visible while dragging the gizmo.
fn draw_extrude_target_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    target: model::ExtrudeTarget,
    color: egui::Color32,
) {
    match target {
        model::ExtrudeTarget::Vertex(point) => {
            if let Some(sp) = extrude::constraint_point_world(doc, point).and_then(project) {
                painter.circle_filled(sp, 5.0, color);
                painter.circle_stroke(sp, 8.0, egui::Stroke::new(2.0, color));
            }
        }
        model::ExtrudeTarget::Face(face) => {
            draw_face_highlight(painter, project, doc, extrude_face_id(face), color);
        }
        model::ExtrudeTarget::Plane(index) => {
            draw_face_highlight(painter, project, doc, FaceId::ConstructionPlane(index), color);
        }
        model::ExtrudeTarget::BodyFace(face_id) => {
            draw_face_highlight(painter, project, doc, face_id, color);
        }
    }
}

impl App {
    /// Tab for in-progress sketch dimensions. Consumes Tab so focus cannot escape to the toolbar
    /// while creating geometry. Enter is handled after dim TextEdits render (see draw_viewport).
    fn handle_in_progress_object_keyboard(&mut self, ui: &mut egui::Ui) {
        if self.state.command_palette.open {
            return;
        }
        if parameters::parameter_field_focused(ui.ctx(), &self.state.doc) {
            return;
        }

        let tab_pressed = ui.input(|i| i.key_pressed(egui::Key::Tab));

        if self.state.creating_rect.is_some() {
            if tab_pressed {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
                let focused = self
                    .state
                    .creating_rect
                    .as_ref()
                    .map(|cr| cr.focused)
                    .unwrap_or(0);
                self.state
                    .apply(Action::FocusRectDimension {
                        axis: next_rect_focus_axis(focused),
                    });
            }
            return;
        }

        if self.state.creating_line.is_some() {
            if tab_pressed {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
                if let Some(cl) = &mut self.state.creating_line {
                    cl.pending_focus = true;
                }
            }
            return;
        }

        if self.state.creating_plane.is_some() {
            if tab_pressed {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
                if self
                    .state
                    .creating_plane
                    .as_ref()
                    .is_some_and(|cp| cp.reference.is_axis())
                {
                    let focused = self
                        .state
                        .creating_plane
                        .as_ref()
                        .map(|cp| cp.focused)
                        .unwrap_or(PlaneDim::Offset);
                    self.state.apply(Action::FocusPlaneDim {
                        dim: next_plane_focus_dim(focused),
                    });
                } else if let Some(cp) = &mut self.state.creating_plane {
                    cp.pending_focus = true;
                }
            }
            return;
        }

        if self.state.editing_committed_dim.is_some() && tab_pressed {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        }
    }

    /// The technical-drawing pane (#180): a black-on-white sheet for the open drawing. Each
    /// view renders its body as an orthographic/isometric wireframe (feature edges), laid out
    /// in a grid; views are added and removed from the controls at the top.
    fn draw_drawing_pane(&mut self, ui: &mut egui::Ui, drawing: usize) {
        use crate::model::DrawingOrientation;
        // The editor is white-on-black to match the app's dark-mode aesthetic (#254); export
        // (see `drawing.rs`) stays the opposite — black ink on a white sheet.
        const INK: egui::Color32 = egui::Color32::from_gray(228);
        const SHEET: egui::Color32 = egui::Color32::from_gray(24);

        // Dark sheet across the whole central area.
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, SHEET);
        // Pointer position for edge hover / label drag (#294).
        let pointer_screen = ui.input(|i| i.pointer.hover_pos());

        // Pan/zoom the sheet like the 3D viewport, but never rotate (#273). Drag the empty
        // background to pan; scroll to zoom about the cursor. Card drags (#274) sit on top and
        // take priority, so this only fires on empty sheet.
        let bg = ui.interact(
            area,
            ui.make_persistent_id(("drawing_page_bg", drawing)),
            egui::Sense::click_and_drag(),
        );
        if bg.dragged() {
            self.drawing_pan += bg.drag_delta();
        }
        let scroll = if bg.hovered() { ui.input(|i| i.raw_scroll_delta.y) } else { 0.0 };
        if scroll != 0.0 {
            let f = (1.0 + scroll * 0.0015).clamp(0.5, 2.0);
            let cursor = bg.hover_pos().unwrap_or(area.center());
            let center0 = area.center() + self.drawing_pan;
            self.drawing_zoom = (self.drawing_zoom * f).clamp(0.2, 8.0);
            let new_center = cursor + (center0 - cursor) * f;
            self.drawing_pan = new_center - area.center();
        }
        if bg.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // The page outline + margin, drawn to the drawing's real page dimensions (#273): a
        // landscape/portrait rectangle at the page's aspect ratio, transformed by the pan/zoom,
        // with the margin rectangle inside it.
        let mut set_page: Option<(f32, f32, f32)> = None;
        // The page rectangle in screen space (#273), reused to position the placed views (#274).
        let mut page_rect: Option<egui::Rect> = None;
        if let Some((pw, ph, pm)) = self
            .state
            .doc
            .drawings
            .get(drawing)
            .filter(|d| !d.deleted)
            .map(|d| (d.page_width_mm, d.page_height_mm, d.margin_mm))
        {
            let avail = egui::Rect::from_min_max(
                area.min + egui::vec2(16.0, 96.0),
                area.max - egui::vec2(16.0, 16.0),
            );
            if avail.width() > 20.0 && avail.height() > 20.0 && pw > 0.0 && ph > 0.0 {
                let fit = (avail.width() / pw).min(avail.height() / ph);
                let scale = fit * self.drawing_zoom;
                let page_size = egui::vec2(pw * scale, ph * scale);
                let page =
                    egui::Rect::from_center_size(area.center() + self.drawing_pan, page_size);
                page_rect = Some(page);
                // A faint white page on the dark sheet, with the margin as a dashed inset.
                ui.painter().rect_filled(page, 2.0, egui::Color32::from_gray(40));
                ui.painter().rect_stroke(
                    page,
                    2.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(120)),
                    egui::StrokeKind::Inside,
                );
                let inset = (pm * scale).min(page_size.x / 2.0 - 1.0).min(page_size.y / 2.0 - 1.0);
                if inset > 0.0 {
                    ui.painter().rect_stroke(
                        page.shrink(inset),
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            // Right-click the sheet background (#273): view/edit the page dimensions (in inches).
            bg.context_menu(|ui| {
                ui.label(egui::RichText::new("Page (inches)").strong());
                let id = ui.make_persistent_id(("drawing_page_edit", drawing));
                let mm_per_in = 25.4;
                let mut draft = ui
                    .data(|d| d.get_temp::<[f32; 3]>(id))
                    .unwrap_or([pw / mm_per_in, ph / mm_per_in, pm / mm_per_in]);
                let row = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(v).speed(0.05).range(0.5..=60.0).suffix(" in"));
                    });
                };
                row(ui, "Width", &mut draft[0]);
                row(ui, "Height", &mut draft[1]);
                row(ui, "Margin", &mut draft[2]);
                ui.data_mut(|d| d.insert_temp(id, draft));
                ui.horizontal(|ui| {
                    if ui.button("Landscape Letter").clicked() {
                        draft = [11.0, 8.5, 0.5];
                        ui.data_mut(|d| d.insert_temp(id, draft));
                    }
                    if ui.button("Portrait Letter").clicked() {
                        draft = [8.5, 11.0, 0.5];
                        ui.data_mut(|d| d.insert_temp(id, draft));
                    }
                });
                if ui.button("Apply").clicked() {
                    set_page = Some((
                        draft[0] * mm_per_in,
                        draft[1] * mm_per_in,
                        draft[2] * mm_per_in,
                    ));
                    ui.data_mut(|d| d.remove::<[f32; 3]>(id));
                    ui.close();
                }
            });
        }

        // Whether this pane is rendering inside the popped-out drawing window (#276).
        let in_window = self.drawing_window == Some(drawing);
        #[allow(unused_mut)] // only mutated by the native-only "Open in window" button
        let mut pop_out = false;
        let mut remove_view: Option<usize> = None;
        let mut toggle_dim: Option<(usize, [i32; 3], [i32; 3])> = None;
        let mut toggle_circle_dim: Option<(usize, [i32; 3])> = None;
        let mut toggle_angle: Option<(usize, model::DrawingEdgeKey, model::DrawingEdgeKey)> = None;
        // First edge of an in-progress angle pick (Shift+click), kept across frames per drawing.
        let pending_angle_id = ui.make_persistent_id(("drawing_angle_pending", drawing));
        let mut pending_angle: Option<(usize, model::DrawingEdgeKey)> =
            ui.data(|d| d.get_temp(pending_angle_id));

        let body_label = |doc: &model::Document, bi: usize| {
            crate::names::node_label(doc, hierarchy::HierarchyNode::Body(bi))
        };

        // Escape no longer leaves the Drawing workbench (#318) — it's used for cancelling
        // in-progress tool actions. A Back button (in the toolbar, left of Select) returns to
        // the model instead.
        // The title/hint were removed (#349) and export moved to the toolbar's Export icon (#348),
        // so only the pop-out action remains on this row (until #347 moves it to the OS menu).
        #[cfg(not(target_arch = "wasm32"))]
        if !in_window {
            ui.horizontal(|ui| {
                if ui
                    .button("Open in window")
                    .on_hover_text("Open this drawing in its own window, beside the 3D view")
                    .clicked()
                {
                    pop_out = true;
                }
            });
            ui.separator();
        }

        // Views are added with the toolbar's Add-view tool (#289): pick a body or sketch in
        // the Elements pane and a projection drops onto the page, ready to drag and configure
        // in the context pane. (The old inline "Add view:" combo row is gone.)

        // Only the active Dimension tool gets a usage hint (#292): the idle prompt that used to
        // sit here was noise on every drawing.
        if self.state.tool == Tool::Dimension {
            ui.colored_label(
                egui::Color32::from_gray(120),
                "Dimension tool: click an edge for its length · Shift+click two edges for their angle",
            );
        }
        // The sheet: each view is a cell with its caption and a projected wireframe of its body.
        let views = self
            .state
            .doc
            .drawings
            .get(drawing)
            .map(|d| d.views.clone())
            .unwrap_or_default();
        // Each view is a draggable card positioned on the page at its `pos` fraction (#274).
        let mut move_view: Option<(usize, f32, f32)> = None;
        let mut set_orientation: Option<(usize, DrawingOrientation)> = None;
        // True when the Aligned-view tool picked its parent this frame (#296), so the same
        // click doesn't also commit the child.
        let mut align_parent_set_this_frame = false;
        // Each rendered view's (scale, bbox_center, area_center) transform, for the aligned
        // projection-line pass (#377) after the card loop.
        let mut view_transforms: Vec<Option<(f32, egui::Vec2, egui::Pos2)>> =
            vec![None; views.len()];
        if let Some(page) = page_rect {
            // WYSIWYG with the exports (#376): the card is exactly the export's page fraction
            // (no pixel clamp) and text sizes scale with the on-screen page, so layout
            // decisions — e.g. whether a dimension label fits along its line — match the PDF.
            let cell_w = page.width() * 0.42;
            let cell_h = page.height() * 0.42;
            let px_per_pt = {
                let page_w_mm = self
                    .state
                    .doc
                    .drawings
                    .get(drawing)
                    .map(|d| d.page_width_mm)
                    .unwrap_or(279.4)
                    .max(1e-3);
                page.width() / (page_w_mm * crate::drawing::PT_PER_MM)
            };
            // The export's 11 pt dimension/caption font, in on-screen pixels.
            let dim_font = 11.0 * px_per_pt;
            let painter = ui.painter().clone();
            for (vi, view) in views.iter().enumerate() {
                // Aligned children (#296) resolve their shared axis to the parent's.
                let (rpx, rpy) = crate::drawing::resolved_view_pos(&self.state.doc, drawing, vi);
                let center =
                    page.min + egui::vec2(rpx * page.width(), rpy * page.height());
                let cell = egui::Rect::from_center_size(center, egui::vec2(cell_w, cell_h));

                // Drag anywhere on the card to move it (#293) — not just the caption strip, so
                // grabbing the projection itself doesn't fall through and pan the page. The
                // Dimension tool's edge picking still works: its click-sense interact is created
                // after this one, so it stays on top for clicks in the drawing area.
                let drag = ui.interact(
                    cell,
                    ui.make_persistent_id(("drawing_view_drag", drawing, vi)),
                    egui::Sense::click_and_drag(),
                );
                // Only the Select tool moves views (#374): with e.g. the Dimension tool a
                // drag across a card must not relocate it (the interact still swallows the
                // drag so it doesn't fall through and pan the page).
                if drag.dragged() && self.state.tool == Tool::Select {
                    // Relative drag: keep the grab point under the cursor instead of snapping
                    // the card's centre to it. An aligned child (#296) only slides along its
                    // free axis — the shared axis stays locked to its parent.
                    let mut delta = drag.drag_delta();
                    if let Some(dir) = view.aligned_dir.filter(|_| view.aligned_parent.is_some()) {
                        if dir.shares_pos_x() {
                            delta.x = 0.0;
                        } else {
                            delta.y = 0.0;
                        }
                    }
                    if delta != egui::Vec2::ZERO {
                        let nx = (view.pos_x + delta.x / page.width()).clamp(0.0, 1.0);
                        let ny = (view.pos_y + delta.y / page.height()).clamp(0.0, 1.0);
                        move_view = Some((vi, nx, ny));
                    }
                }
                if drag.hovered() && self.state.tool == Tool::Select {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                // Clicking a card selects it (#289): the context pane opens its view editor.
                // With the Aligned-view tool (#296), a click instead chooses this as the
                // parent to align a child to (handled after the loop).
                if drag.clicked() {
                    if self.state.tool == Tool::DrawingAlign {
                        self.drawing_align_parent = Some(vi);
                        align_parent_set_this_frame = true;
                    } else {
                        let element = context::DrawingElementRef::Projection(vi);
                        if ui.input(|i| selection::additive_click_modifiers(&i.modifiers)) {
                            self.state.toggle_drawing_element(drawing, element);
                        } else {
                            self.state.select_drawing_only(drawing, element);
                        }
                    }
                }
                drag.context_menu(|ui| {
                    ui.label("View");
                    for o in DrawingOrientation::ALL {
                        if ui.selectable_label(view.orientation == *o, o.label()).clicked() {
                            set_orientation = Some((vi, *o));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Remove").clicked() {
                        remove_view = Some(vi);
                        ui.close();
                    }
                });
                // The selected card (#289) gets an accent border; a hovered card gets a lighter
                // one so it's clear the whole card is clickable (#316); others a faint outline.
                let selected_here = self
                    .state
                    .is_drawing_element_selected(drawing, context::DrawingElementRef::Projection(vi));
                let align_parent_here = self.drawing_align_parent == Some(vi)
                    && self.state.tool == Tool::DrawingAlign;
                // The Select-tool element picker hovering this projection's row highlights it (#328).
                let picker_hover_here = self.state.hovered_drawing_element
                    == Some(context::DrawingElementRef::Projection(vi));
                let stroke = if selected_here || align_parent_here || picker_hover_here {
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 230))
                } else if drag.hovered() {
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 140, 170))
                } else {
                    egui::Stroke::new(1.0, egui::Color32::from_gray(80))
                };
                painter.rect_stroke(cell.shrink(2.0), 2.0, stroke, egui::StrokeKind::Inside);
                let source_label = match view.sketch {
                    Some(si) => crate::names::node_label(
                        &self.state.doc,
                        hierarchy::HierarchyNode::Sketch(si),
                    ),
                    None => body_label(&self.state.doc, view.body),
                };
                let scale_suffix =
                    crate::drawing::resolved_view_scale(&self.state.doc, drawing, vi)
                        .map(|s| format!(" ({s})"))
                        .unwrap_or_default();
                // The caption label is toggleable, positionable within the card, and its text
                // overridable (#372); a custom template interpolates {expr} fields (#338).
                if !view.label_hidden {
                    let caption = match &view.label_text {
                        Some(t) => crate::value::interpolate_text(t, &self.state.doc),
                        None => format!(
                            "{source_label} — {}{scale_suffix}",
                            view.orientation.label()
                        ),
                    };
                    use crate::model::DrawingLabelPos as LP;
                    // Mirror the export's anchors (#376): x insets of CELL_PAD, a top row
                    // whose baseline sits 20 pt into the card and a bottom row 8 pt off its
                    // edge — all scaled to on-screen pixels. egui anchors by the text box, so
                    // the baselines are approximated with the font's ascent/descent.
                    let pad = crate::drawing::CELL_PAD * px_per_pt;
                    let top_y = cell.min.y + (20.0 - 0.8 * 11.0) * px_per_pt;
                    let bottom_y = cell.max.y - (8.0 - 0.2 * 11.0) * px_per_pt;
                    let (pos, align) = match view.label_pos {
                        LP::TopLeft => {
                            (egui::pos2(cell.min.x + pad, top_y), egui::Align2::LEFT_TOP)
                        }
                        LP::TopCenter => {
                            (egui::pos2(cell.center().x, top_y), egui::Align2::CENTER_TOP)
                        }
                        LP::TopRight => {
                            (egui::pos2(cell.max.x - pad, top_y), egui::Align2::RIGHT_TOP)
                        }
                        LP::BottomLeft => {
                            (egui::pos2(cell.min.x + pad, bottom_y), egui::Align2::LEFT_BOTTOM)
                        }
                        LP::BottomCenter => (
                            egui::pos2(cell.center().x, bottom_y),
                            egui::Align2::CENTER_BOTTOM,
                        ),
                        LP::BottomRight => (
                            egui::pos2(cell.max.x - pad, bottom_y),
                            egui::Align2::RIGHT_BOTTOM,
                        ),
                    };
                    painter.text(pos, align, caption, egui::FontId::proportional(dim_font), INK);
                }
                // Remove button in the cell's top-right corner.
                let x_rect = egui::Rect::from_min_size(
                    egui::pos2(cell.max.x - 24.0, cell.min.y + 4.0),
                    egui::vec2(20.0, 20.0),
                );
                // Remove uses the bundled ✕ SVG (IconId::Close), never a font glyph (#325).
                if ui
                    .put(
                        x_rect,
                        egui::ImageButton::new(icons::sized_texture(ui.ctx(), icons::IconId::Close))
                            .frame(true),
                    )
                    .on_hover_text("Remove view")
                    .clicked()
                {
                    remove_view = Some(vi);
                }
                // Project the body's feature edges into the cell (below the caption strip),
                // padded exactly like the export (#376) so auto-fit scales match.
                let draw_area = {
                    let pad = crate::drawing::CELL_PAD * px_per_pt;
                    egui::Rect::from_min_max(
                        cell.min + egui::vec2(pad, 26.0 * px_per_pt + pad),
                        cell.max - egui::vec2(pad, pad),
                    )
                };
                // Aligned children render with their unfolded (rotated) basis (#351).
                let (right, up) = crate::drawing::resolved_view_axes(&views, view);
                let project = |p: Vec3| egui::vec2(p.dot(right), p.dot(up));
                // Crease edges drive circle detection (#319); the dimensionable set also carries
                // silhouette edges so a smooth extrusion's length can be dimensioned (#334).
                let crease_edges = crate::drawing::drawing_view_world_edges(&self.state.doc, view);
                let world_edges =
                    crate::drawing::drawing_view_dimensionable_edges(&self.state.doc, &views, view);
                if world_edges.is_empty() {
                    continue;
                }
                let proj: Vec<(egui::Vec2, egui::Vec2)> = world_edges
                    .iter()
                    .map(|(a, b)| (project(*a), project(*b)))
                    .collect();
                let (mut min, mut max) = (egui::vec2(f32::MAX, f32::MAX), egui::vec2(f32::MIN, f32::MIN));
                for (a, b) in &proj {
                    for p in [a, b] {
                        min = min.min(*p);
                        max = max.max(*p);
                    }
                }
                let extent = (max - min).max(egui::vec2(1e-3, 1e-3));
                // A set print scale (#300) renders at exactly `factor` page-mm per model-mm
                // (converted to screen pixels through the page's on-screen size); otherwise
                // auto-fit to the card.
                let px_per_page_mm = self
                    .state
                    .doc
                    .drawings
                    .get(drawing)
                    .map(|d| page.width() / d.page_width_mm.max(1e-3))
                    .unwrap_or(1.0);
                // An aligned child inherits its parent's scale (#296/#300).
                let resolved_scale =
                    crate::drawing::resolved_view_scale(&self.state.doc, drawing, vi);
                let _ = extent;
                let scale = match resolved_scale.as_deref().and_then(model::parse_drawing_scale) {
                    Some(factor) => factor * px_per_page_mm,
                    // Aligned children share their parent's auto-fit scale so edges line up (#364).
                    None => crate::drawing::view_autofit_scale(
                        &self.state.doc,
                        &views,
                        vi,
                        draw_area.width(),
                        draw_area.height(),
                        0.9,
                    ),
                };
                // Aligned children line up to their parent along the shared edge (#364).
                let bbox_center = {
                    let c = crate::drawing::view_render_center(&self.state.doc, &views, vi);
                    egui::vec2(c.x, c.y)
                };
                // Model +up maps to screen -y; center the fitted bbox in the draw area.
                let to_screen = |p: egui::Vec2| {
                    let d = (p - bbox_center) * scale;
                    draw_area.center() + egui::vec2(d.x, -d.y)
                };
                // Remember this view's transform for the aligned projection-line pass (#377).
                view_transforms[vi] = Some((scale, bbox_center, draw_area.center()));
                let edge_key = |wa: Vec3, wb: Vec3| {
                    let (qa, qb) = (
                        hierarchy::quantize_body_point(wa),
                        hierarchy::quantize_body_point(wb),
                    );
                    if qa <= qb { (qa, qb) } else { (qb, qa) }
                };
                // Detect tessellated circles in world space (#313) and project them for this
                // view — round face-on, a foreshortened line edge-on (#319).
                let (vright, vup) = crate::drawing::resolved_view_axes(&views, view);
                let world_circles = crate::drawing::classify_world_circles(&crease_edges);
                let pcircles: Vec<crate::drawing::ProjectedCircle> = world_circles
                    .iter()
                    .map(|c| crate::drawing::project_world_circle(c, vright, vup))
                    .collect();
                let on_circle = |a: egui::Vec2, b: egui::Vec2| {
                    crate::drawing::projected_segment_on_circle(
                        glam::Vec2::new(a.x, a.y),
                        glam::Vec2::new(b.x, b.y),
                        &pcircles,
                    )
                };

                let unit = self.state.doc.default_length_unit;
                let dims = view.dimensioned_edges.clone();
                let pending_here = pending_angle.filter(|(pv, _)| *pv == vi).map(|(_, k)| k);
                let bbox_center_v = egui::vec2(bbox_center.x, bbox_center.y);
                let diag = extent.length().max(1.0);
                let default_gap = diag * 0.05;
                let arrow = diag * 0.025;
                // The on-screen dimension line segment for edge `i` if it currently carries a
                // dimension (#324), so the Dimension tool can pick a dimension by hovering its
                // line/label, not only the model edge. Mirrors the render loop's geometry.
                let dim_line_screen = |i: usize| -> Option<(egui::Pos2, egui::Pos2)> {
                    let (a, b) = proj[i];
                    if on_circle(a, b) || (b - a).length() < 1e-3 {
                        return None;
                    }
                    let (wa, wb) = world_edges[i];
                    let key = edge_key(wa, wb);
                    if !dims.contains(&key) {
                        return None;
                    }
                    let outward = {
                        let seg = b - a;
                        let mut p = egui::vec2(-seg.y, seg.x).normalized();
                        if p == egui::Vec2::ZERO {
                            p = egui::vec2(0.0, -1.0);
                        }
                        let mid = (a + b) * 0.5;
                        if p.dot(mid - bbox_center_v) < 0.0 { -p } else { p }
                    };
                    let extra = view
                        .dimension_offsets
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, o)| *o)
                        .unwrap_or(0.0);
                    let g = crate::drawing::dimension_line_geometry(
                        glam::Vec2::new(a.x, a.y),
                        glam::Vec2::new(b.x, b.y),
                        glam::Vec2::new(outward.x, outward.y),
                        default_gap + extra,
                        arrow,
                    );
                    Some((
                        to_screen(egui::vec2(g.line.0.x, g.line.0.y)),
                        to_screen(egui::vec2(g.line.1.x, g.line.1.y)),
                    ))
                };

                // On the Dimension tool, the edge nearest the cursor previews so it's clear a
                // click toggles it (#294). The dimension's own line/label counts as the edge too
                // (#324), so an already-shown dimension can be toggled off by hovering its line.
                // Detected circles are pickable the same way (#373) — their outline (round
                // face-on, the foreshortened line edge-on) toggles the Ø dimension — so their
                // tessellation segments are excluded from the edge candidates.
                let (hovered_edge, hovered_circle) = (self.state.tool == Tool::Dimension)
                    .then(|| {
                        let pp = pointer_screen?;
                        if !draw_area.contains(pp) {
                            return None;
                        }
                        let mut best_edge: Option<(f32, usize)> = None;
                        for (i, (a, b)) in proj.iter().enumerate() {
                            if on_circle(*a, *b) {
                                continue;
                            }
                            let mut d = dist_point_to_segment(pp, to_screen(*a), to_screen(*b));
                            if let Some((la, lb)) = dim_line_screen(i) {
                                d = d.min(dist_point_to_segment(pp, la, lb));
                            }
                            if best_edge.is_none_or(|(bd, _)| d < bd) {
                                best_edge = Some((d, i));
                            }
                        }
                        let mut best_circle: Option<(f32, usize)> = None;
                        for (ci, pc) in pcircles.iter().enumerate() {
                            let d = match pc {
                                crate::drawing::ProjectedCircle::Round { center, radius } => {
                                    let sc = to_screen(egui::vec2(center.x, center.y));
                                    ((pp - sc).length() - radius * scale).abs()
                                }
                                crate::drawing::ProjectedCircle::EdgeOn { a, b } => {
                                    dist_point_to_segment(
                                        pp,
                                        to_screen(egui::vec2(a.x, a.y)),
                                        to_screen(egui::vec2(b.x, b.y)),
                                    )
                                }
                            };
                            if best_circle.is_none_or(|(bd, _)| d < bd) {
                                best_circle = Some((d, ci));
                            }
                        }
                        let best_edge = best_edge.filter(|(d, _)| *d <= 8.0);
                        let best_circle = best_circle.filter(|(d, _)| *d <= 8.0);
                        Some(match (best_edge, best_circle) {
                            (Some((de, i)), Some((dc, ci))) => {
                                if dc <= de {
                                    (None, Some(ci))
                                } else {
                                    (Some(i), None)
                                }
                            }
                            (e, c) => (e.map(|(_, i)| i), c.map(|(_, ci)| ci)),
                        })
                    })
                    .flatten()
                    .unwrap_or((None, None));

                // Click near an edge to toggle its length dimension; Shift+click two edges to
                // toggle the angle between them (#180). Only the Dimension tool picks edges
                // (#277), and the pick interact is created ONLY on that tool (#316) — otherwise
                // it would sit over most of the card and steal clicks meant to select the card
                // (which broke the Select/Aligned-view tools). A label drag (below) suppresses
                // the toggle so repositioning doesn't hide the dim.
                let resp = (self.state.tool == Tool::Dimension).then(|| {
                    ui.interact(
                        draw_area,
                        ui.make_persistent_id(("drawing_view_pick", drawing, vi)),
                        egui::Sense::click(),
                    )
                });
                if resp.as_ref().is_some_and(|r| r.clicked())
                    && self.drawing_dim_label_drag.is_none()
                {
                    if let Some(i) = hovered_edge {
                        let (wa, wb) = world_edges[i];
                        let key = edge_key(wa, wb);
                        if ui.input(|inp| inp.modifiers.shift) {
                            match pending_angle {
                                Some((pv, pk)) if pv == vi && pk != key => {
                                    toggle_angle = Some((vi, pk, key));
                                    pending_angle = None;
                                }
                                _ => pending_angle = Some((vi, key)),
                            }
                        } else {
                            toggle_dim = Some((
                                vi,
                                hierarchy::quantize_body_point(wa),
                                hierarchy::quantize_body_point(wb),
                            ));
                        }
                    } else if let Some(ci) = hovered_circle {
                        // A detected circle toggles its diameter dimension (#373).
                        toggle_circle_dim = Some((
                            vi,
                            hierarchy::quantize_body_point(world_circles[ci].center),
                        ));
                    }
                }

                // Strokes come from the styled geometry (#301/#319: includes the silhouette so
                // cylinder sides show); the per-edge loop below only adds dimensions and pick
                // highlights. Sketch views keep the per-edge stroke (no mesh silhouette).
                let styled = (view.sketch.is_none())
                    .then(|| crate::drawing::styled_view_geometry(&self.state.doc, &views, view));
                if let Some(sty) = &styled {
                    for (pts, shade) in &sty.tris {
                        // The editor sheet is dark; map the print greys down so shading reads
                        // without blowing out (exports keep the light print greys).
                        let level = (shade.clamp(0.0, 1.0) * 110.0) as u8 + 30;
                        painter.add(egui::Shape::convex_polygon(
                            pts.iter()
                                .map(|p| to_screen(egui::vec2(p.x, p.y)))
                                .collect(),
                            egui::Color32::from_gray(level),
                            egui::Stroke::NONE,
                        ));
                    }
                    for (a, b) in &sty.segments {
                        if on_circle(egui::vec2(a.x, a.y), egui::vec2(b.x, b.y)) {
                            continue;
                        }
                        painter.line_segment(
                            [
                                to_screen(egui::vec2(a.x, a.y)),
                                to_screen(egui::vec2(b.x, b.y)),
                            ],
                            egui::Stroke::new(crate::drawing::MODEL_STROKE, INK),
                        );
                    }
                }
                // A rotated label drawn centred at a screen point (#314/#320).
                let draw_rot_label = |painter: &egui::Painter, text: String, at: egui::Pos2, ang: f32| {
                    let galley =
                        painter.layout_no_wrap(text, egui::FontId::proportional(dim_font), INK);
                    let rot = egui::emath::Rot2::from_angle(ang);
                    let pos = at - rot * (galley.size() * 0.5);
                    let mut shape = egui::epaint::TextShape::new(pos, galley, INK);
                    shape.angle = ang;
                    painter.add(shape);
                };
                // Detected circles (#313): a smooth circle face-on or a diameter line edge-on
                // (#319), each with one diameter dimension. Edge-on uses a full linear
                // dimension (extension lines + arrows), like a regular length (#320).
                for (ci, (wc, pc)) in world_circles.iter().zip(&pcircles).enumerate() {
                    // The circle outline always draws; its diameter dimension only when shown (#342).
                    let show_dim = view
                        .dimensioned_circles
                        .contains(&hierarchy::quantize_body_point(wc.center));
                    // On the Dimension tool the hovered circle previews (#373), like edges do.
                    let outline = if hovered_circle == Some(ci) {
                        egui::Stroke::new(2.4, egui::Color32::from_rgb(90, 150, 230))
                    } else {
                        egui::Stroke::new(crate::drawing::MODEL_STROKE, INK)
                    };
                    let label =
                        format!("Ø{}", crate::value::format_length_display_in(wc.radius * 2.0, unit));
                    match pc {
                        crate::drawing::ProjectedCircle::Round { center, radius } => {
                            let sc = to_screen(egui::vec2(center.x, center.y));
                            painter.circle_stroke(sc, radius * scale, outline);
                            if !show_dim {
                                continue;
                            }
                            // Horizontal diameter line by default (#397); the label can be
                            // dragged up/down off the line (a circle_dim_offsets override).
                            let dir = egui::vec2(1.0, 0.0);
                            let cv = egui::vec2(center.x, center.y);
                            let (sa, sb) = (to_screen(cv - dir * *radius), to_screen(cv + dir * *radius));
                            painter.line_segment([sa, sb], egui::Stroke::new(crate::drawing::DIM_STROKE, INK));
                            let circle_key = hierarchy::quantize_body_point(wc.center);
                            let extra = view
                                .circle_dim_offsets
                                .iter()
                                .find(|(k, _)| *k == circle_key)
                                .map(|(_, o)| *o)
                                .unwrap_or(0.0);
                            let label_screen = to_screen(cv + egui::vec2(0.0, extra));
                            draw_rot_label(&painter, label, label_screen, 0.0);
                            // The label drags up/down with Select or Dimension (#397),
                            // mirroring the edge dims' label drag.
                            if matches!(self.state.tool, Tool::Select | Tool::Dimension) {
                                let label_rect = egui::Rect::from_center_size(
                                    label_screen,
                                    egui::vec2(46.0, 18.0),
                                );
                                let lr = ui.interact(
                                    label_rect,
                                    ui.make_persistent_id((
                                        "drawing_circle_dim_label",
                                        drawing,
                                        vi,
                                        circle_key,
                                    )),
                                    egui::Sense::click_and_drag(),
                                );
                                if lr.hovered()
                                    || self
                                        .drawing_dim_label_drag
                                        .is_some_and(|d| d.circle && d.key.0 == circle_key)
                                {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                }
                                if lr.drag_started() {
                                    if let Some(pp) = lr.interact_pointer_pos() {
                                        // Screen up = +offset (projected +v maps to −y).
                                        self.drawing_dim_label_drag = Some(DrawingDimLabelDrag {
                                            drawing,
                                            view: vi,
                                            key: (circle_key, circle_key),
                                            circle: true,
                                            start_offset: extra,
                                            start_pointer: pp,
                                            outward_screen: egui::vec2(0.0, -1.0),
                                            mm_per_px: if scale.abs() > 1e-6 {
                                                1.0 / scale
                                            } else {
                                                0.0
                                            },
                                        });
                                    }
                                }
                            }
                        }
                        crate::drawing::ProjectedCircle::EdgeOn { a, b } => {
                            let (av, bv) = (egui::vec2(a.x, a.y), egui::vec2(b.x, b.y));
                            painter.line_segment([to_screen(av), to_screen(bv)], outline);
                            if !show_dim {
                                continue;
                            }
                            let outward = {
                                let seg = bv - av;
                                let mut p = egui::vec2(-seg.y, seg.x).normalized();
                                if p == egui::Vec2::ZERO { p = egui::vec2(0.0, -1.0); }
                                let mid = (av + bv) * 0.5;
                                if p.dot(mid - bbox_center_v) < 0.0 { -p } else { p }
                            };
                            // An offset override (#397) pushes the whole linear dim further
                            // out, like the edge dims' dimension_offsets.
                            let circle_key = hierarchy::quantize_body_point(wc.center);
                            let extra = view
                                .circle_dim_offsets
                                .iter()
                                .find(|(k, _)| *k == circle_key)
                                .map(|(_, o)| *o)
                                .unwrap_or(0.0);
                            let g = crate::drawing::dimension_line_geometry(
                                glam::Vec2::new(av.x, av.y),
                                glam::Vec2::new(bv.x, bv.y),
                                glam::Vec2::new(outward.x, outward.y),
                                default_gap + extra,
                                arrow,
                            );
                            let sp = |p: glam::Vec2| to_screen(egui::vec2(p.x, p.y));
                            for (p, q) in g.extensions {
                                painter.line_segment([sp(p), sp(q)], egui::Stroke::new(crate::drawing::DIM_STROKE, INK));
                            }
                            painter.line_segment([sp(g.line.0), sp(g.line.1)], egui::Stroke::new(crate::drawing::DIM_STROKE, INK));
                            for tri in g.arrows {
                                painter.add(egui::Shape::convex_polygon(
                                    tri.iter().map(|p| sp(*p)).collect(),
                                    INK,
                                    egui::Stroke::NONE,
                                ));
                            }
                            let (sla, slb) = (sp(g.line.0), sp(g.line.1));
                            let out_screen = (sp(g.line.0
                                + glam::Vec2::new(outward.x, outward.y))
                                - sp(g.line.0))
                            .normalized();
                            let (lp, ang) = crate::drawing::dimension_label_layout(
                                glam::Vec2::new(sla.x, sla.y),
                                glam::Vec2::new(slb.x, slb.y),
                                glam::Vec2::new(out_screen.x, out_screen.y),
                                crate::drawing::text_device_width(dim_font, &label),
                                5.0 * px_per_pt,
                            );
                            let label_screen = egui::pos2(lp.x, lp.y);
                            draw_rot_label(&painter, label, label_screen, ang);
                            // Draggable like the edge dims (#397): slides the linear dim
                            // nearer/further via the circle's offset override.
                            if matches!(self.state.tool, Tool::Select | Tool::Dimension) {
                                let label_rect = egui::Rect::from_center_size(
                                    label_screen,
                                    egui::vec2(46.0, 18.0),
                                );
                                let lr = ui.interact(
                                    label_rect,
                                    ui.make_persistent_id((
                                        "drawing_circle_dim_label",
                                        drawing,
                                        vi,
                                        circle_key,
                                    )),
                                    egui::Sense::click_and_drag(),
                                );
                                if lr.hovered()
                                    || self
                                        .drawing_dim_label_drag
                                        .is_some_and(|d| d.circle && d.key.0 == circle_key)
                                {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                }
                                if lr.drag_started() {
                                    if let Some(pp) = lr.interact_pointer_pos() {
                                        self.drawing_dim_label_drag = Some(DrawingDimLabelDrag {
                                            drawing,
                                            view: vi,
                                            key: (circle_key, circle_key),
                                            circle: true,
                                            start_offset: extra,
                                            start_pointer: pp,
                                            outward_screen: egui::vec2(out_screen.x, out_screen.y),
                                            mm_per_px: if scale.abs() > 1e-6 {
                                                1.0 / scale
                                            } else {
                                                0.0
                                            },
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                for (i, (a, b)) in proj.iter().enumerate() {
                    // Circle-tessellation segments are drawn as the smooth circle above (#313).
                    let is_circle_seg = on_circle(*a, *b);
                    let (sa, sb) = (to_screen(*a), to_screen(*b));
                    let (wa, wb) = world_edges[i];
                    let key = edge_key(wa, wb);
                    // The first edge of an in-progress angle pick glows; the hovered edge on the
                    // Dimension tool previews so it's clear a click toggles it (#294).
                    let glow = pending_here == Some(key);
                    if glow {
                        painter.line_segment(
                            [sa, sb],
                            egui::Stroke::new(2.4, egui::Color32::from_rgb(30, 90, 200)),
                        );
                    } else if hovered_edge == Some(i) {
                        painter.line_segment(
                            [sa, sb],
                            egui::Stroke::new(2.4, egui::Color32::from_rgb(90, 150, 230)),
                        );
                    } else if styled.is_none() && !is_circle_seg {
                        painter.line_segment([sa, sb], egui::Stroke::new(crate::drawing::MODEL_STROKE, INK));
                    }
                    if dims.contains(&key) && (*b - *a).length() >= 1e-3 && !is_circle_seg {
                        // Architectural dimension line (#294): extension lines, an offset
                        // dimension line with arrowheads, and the length centred on it.
                        let (av, bv) = (egui::vec2(a.x, a.y), egui::vec2(b.x, b.y));
                        let outward = {
                            let seg = bv - av;
                            let mut p = egui::vec2(-seg.y, seg.x).normalized();
                            if p == egui::Vec2::ZERO {
                                p = egui::vec2(0.0, -1.0);
                            }
                            let mid = (av + bv) * 0.5;
                            if p.dot(mid - bbox_center_v) < 0.0 { -p } else { p }
                        };
                        let extra = view
                            .dimension_offsets
                            .iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, o)| *o)
                            .unwrap_or(0.0);
                        let off = default_gap + extra;
                        let g = crate::drawing::dimension_line_geometry(
                            glam::Vec2::new(av.x, av.y),
                            glam::Vec2::new(bv.x, bv.y),
                            glam::Vec2::new(outward.x, outward.y),
                            off,
                            arrow,
                        );
                        let sp = |p: glam::Vec2| to_screen(egui::vec2(p.x, p.y));
                        for (p, q) in g.extensions {
                            painter.line_segment([sp(p), sp(q)], egui::Stroke::new(crate::drawing::DIM_STROKE, INK));
                        }
                        painter.line_segment(
                            [sp(g.line.0), sp(g.line.1)],
                            egui::Stroke::new(crate::drawing::DIM_STROKE, INK),
                        );
                        for tri in g.arrows {
                            painter.add(egui::Shape::convex_polygon(
                                tri.iter().map(|p| sp(*p)).collect(),
                                INK,
                                egui::Stroke::NONE,
                            ));
                        }
                        // The label runs along the dimension line, or sits past its end if the
                        // line is too short (#314).
                        let label_text =
                            crate::value::format_length_display_in((wa - wb).length(), unit);
                        let (sla, slb) = (sp(g.line.0), sp(g.line.1));
                        let out_screen =
                            (sp(g.line.0 + glam::Vec2::new(outward.x, outward.y)) - sp(g.line.0))
                                .normalized();
                        let (lp, ang) = crate::drawing::dimension_label_layout(
                            glam::Vec2::new(sla.x, sla.y),
                            glam::Vec2::new(slb.x, slb.y),
                            glam::Vec2::new(out_screen.x, out_screen.y),
                            crate::drawing::text_device_width(dim_font, &label_text),
                            5.0 * px_per_pt,
                        );
                        let label_screen = egui::pos2(lp.x, lp.y);
                        let galley = painter.layout_no_wrap(
                            label_text,
                            egui::FontId::proportional(dim_font),
                            INK,
                        );
                        // TextShape rotates about its top-left `pos`; offset so the label's
                        // centre lands at label_screen after rotation (#314).
                        let rot = egui::emath::Rot2::from_angle(ang);
                        let pos = label_screen - rot * (galley.size() * 0.5);
                        let mut shape = egui::epaint::TextShape::new(pos, galley, INK);
                        shape.angle = ang;
                        painter.add(shape);
                        // The label is draggable with Select or Dimension (#294): a small
                        // interact rect at the label repositions the whole dimension line.
                        if matches!(self.state.tool, Tool::Select | Tool::Dimension) {
                            let label_rect =
                                egui::Rect::from_center_size(label_screen, egui::vec2(46.0, 18.0));
                            let lr = ui.interact(
                                label_rect,
                                ui.make_persistent_id(("drawing_dim_label", drawing, vi, key)),
                                egui::Sense::click_and_drag(),
                            );
                            // Clicking a dimension with the Select tool selects it (#336), so
                            // Delete/Backspace can remove it; clears any card/note selection.
                            if lr.clicked() && self.state.tool == Tool::Select {
                                let element = context::DrawingElementRef::Dimension {
                                    view: vi,
                                    a: key.0,
                                    b: key.1,
                                };
                                if ui.input(|i| selection::additive_click_modifiers(&i.modifiers)) {
                                    self.state.toggle_drawing_element(drawing, element);
                                } else {
                                    self.state.select_drawing_only(drawing, element);
                                }
                            }
                            let is_selected_dim = self.state.is_drawing_element_selected(
                                drawing,
                                context::DrawingElementRef::Dimension { view: vi, a: key.0, b: key.1 },
                            );
                            // The element picker hovering this dimension's row highlights it (#328).
                            let picker_hover_dim = self.state.hovered_drawing_element
                                == Some(context::DrawingElementRef::Dimension {
                                    view: vi,
                                    a: key.0,
                                    b: key.1,
                                });
                            let active = lr.hovered()
                                || picker_hover_dim
                                || self.drawing_dim_label_drag.map(|d| d.key) == Some(key);
                            if active {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            // Highlight the dimension being hovered so it's obvious which one a
                            // drag will move (#326) — with the Dimension tool too (#375), where
                            // hovering the dimension's line or its edge also marks it (a click
                            // there toggles it, a label drag repositions it). A selected
                            // dimension stays highlighted so it's clear what Delete will
                            // remove (#336).
                            let dim_tool_hover =
                                self.state.tool == Tool::Dimension && hovered_edge == Some(i);
                            if active
                                || dim_tool_hover
                                || (is_selected_dim && self.state.tool == Tool::Select)
                            {
                                let accent = egui::Color32::from_rgb(90, 150, 230);
                                painter.line_segment(
                                    [sp(g.line.0), sp(g.line.1)],
                                    egui::Stroke::new(2.0, accent),
                                );
                                painter.rect_stroke(
                                    label_rect,
                                    2.0,
                                    egui::Stroke::new(1.0, accent),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if lr.drag_started() {
                                if let Some(pp) = lr.interact_pointer_pos() {
                                    let om = sp(g.line.0 + glam::Vec2::new(outward.x, outward.y))
                                        - sp(g.line.0);
                                    self.drawing_dim_label_drag = Some(DrawingDimLabelDrag {
                                        drawing,
                                        view: vi,
                                        key,
                                        circle: false,
                                        start_offset: extra,
                                        start_pointer: pp,
                                        outward_screen: om.normalized(),
                                        mm_per_px: if scale.abs() > 1e-6 { 1.0 / scale } else { 0.0 },
                                    });
                                }
                            }
                        }
                    }
                }
                // Angle dimensions between edge pairs: the degree value at (or near) the corner.
                let dequant = |q: [i32; 3]| Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) / 100.0;
                for (k1, k2) in &view.angle_dims {
                    let (a0, a1) = (dequant(k1.0), dequant(k1.1));
                    let (b0, b1) = (dequant(k2.0), dequant(k2.1));
                    let d1 = (a1 - a0).normalize_or_zero();
                    let d2 = (b1 - b0).normalize_or_zero();
                    if d1.length_squared() < 0.5 || d2.length_squared() < 0.5 {
                        continue;
                    }
                    let angle = d1.angle_between(d2).to_degrees();
                    // Anchor at a shared corner if the edges touch, else between their midpoints.
                    let shared = [k1.0, k1.1]
                        .into_iter()
                        .find(|e| *e == k2.0 || *e == k2.1)
                        .map(dequant);
                    let anchor = shared.unwrap_or_else(|| {
                        ((a0 + a1) * 0.5 + (b0 + b1) * 0.5) * 0.5
                    });
                    let sp = to_screen(project(anchor));
                    painter.text(
                        sp + egui::vec2(0.0, -12.0 * px_per_pt),
                        egui::Align2::CENTER_CENTER,
                        format!("{angle:.0}°"),
                        egui::FontId::proportional(12.0 * px_per_pt),
                        INK,
                    );
                }
            }
        }

        // Free text annotations on the page (#312): render them, let the Select tool drag
        // them, and let the Text tool place new ones (click = growing, drag = wrapped box).
        if let Some(page) = page_rect {
            let mut place: Option<(f32, f32, Option<f32>)> = None; // pos_x, pos_y, wrap_frac
            let mut move_ann: Option<(usize, f32, f32)> = None;
            let mut select_ann: Option<usize> = None;
            let annotations = self
                .state
                .doc
                .drawings
                .get(drawing)
                .map(|d| d.annotations.clone())
                .unwrap_or_default();
            for (ai, ann) in annotations.iter().enumerate() {
                if ann.deleted {
                    continue;
                }
                let font_px = (ann.size_frac * page.height()).clamp(6.0, 200.0);
                let pos = page.min + egui::vec2(ann.pos_x * page.width(), ann.pos_y * page.height());
                let wrap_px = ann
                    .wrap_frac
                    .map(|w| (w * page.width()).max(10.0))
                    .unwrap_or(f32::INFINITY);
                // Substitute {expr} variable fields against the document's parameters (#338); the
                // context-pane editor still holds the raw template.
                let rendered = crate::value::interpolate_text(&ann.text, &self.state.doc);
                let galley = ui.painter().layout(
                    rendered,
                    egui::FontId::proportional(font_px),
                    INK,
                    wrap_px,
                );
                let rect = egui::Rect::from_min_size(pos, galley.size());
                let selected = self
                    .state
                    .is_drawing_element_selected(drawing, context::DrawingElementRef::Text(ai));
                // The Select-tool element picker hovering this note's row highlights it (#328).
                let picker_hover = self.state.hovered_drawing_element
                    == Some(context::DrawingElementRef::Text(ai));
                if selected || picker_hover {
                    ui.painter().rect_stroke(
                        rect.expand(2.0),
                        1.0,
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 150, 230)),
                        egui::StrokeKind::Outside,
                    );
                }
                ui.painter().galley(pos, galley, INK);
                // Select-tool drag/select (#312).
                if self.state.tool == Tool::Select {
                    let resp = ui.interact(
                        rect,
                        ui.make_persistent_id(("drawing_annotation", drawing, ai)),
                        egui::Sense::click_and_drag(),
                    );
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    if resp.clicked() {
                        select_ann = Some(ai);
                    }
                    // Double-clicking a textbox jumps to its context-pane editor (#379):
                    // the field takes focus with the text selected, so typing replaces it.
                    if resp.double_clicked() {
                        select_ann = Some(ai);
                        self.state.context_pane.focus_annotation_field = true;
                    }
                    if resp.dragged() {
                        select_ann = Some(ai);
                        let d = resp.drag_delta();
                        let nx = (ann.pos_x + d.x / page.width()).clamp(0.0, 1.0);
                        let ny = (ann.pos_y + d.y / page.height()).clamp(0.0, 1.0);
                        move_ann = Some((ai, nx, ny));
                    }
                }
            }

            // Text tool placement: press anchors, release commits (#312/#282 click-vs-drag).
            if self.state.tool == Tool::Text {
                if let Some(pp) = pointer_screen {
                    let frac = |p: egui::Pos2| {
                        (
                            ((p.x - page.min.x) / page.width()).clamp(0.0, 1.0),
                            ((p.y - page.min.y) / page.height()).clamp(0.0, 1.0),
                        )
                    };
                    if ui.input(|i| i.pointer.primary_pressed()) && page.contains(pp) {
                        self.drawing_text_anchor = Some(frac(pp));
                    } else if ui.input(|i| i.pointer.primary_released()) {
                        if let Some((ax, ay)) = self.drawing_text_anchor.take() {
                            let (rx, _ry) = frac(pp);
                            let wrap = ((rx - ax).abs() >= 0.03).then_some((rx - ax).abs());
                            place = Some((ax.min(rx), ay, wrap));
                        }
                    }
                }
            } else {
                self.drawing_text_anchor = None;
            }

            if let Some((ai, nx, ny)) = move_ann {
                self.state.apply(Action::MoveDrawingAnnotation {
                    drawing,
                    annotation: ai,
                    pos_x: nx,
                    pos_y: ny,
                });
            }
            if let Some(ai) = select_ann {
                let element = context::DrawingElementRef::Text(ai);
                if ui.input(|i| selection::additive_click_modifiers(&i.modifiers)) {
                    self.state.toggle_drawing_element(drawing, element);
                } else {
                    self.state.select_drawing_only(drawing, element);
                }
            }
            // Clicking blank page space with the Select tool deselects everything (#346). `bg` is
            // the page-background interact created before the cards/notes, so it only reports a
            // click when nothing on top consumed it.
            else if self.state.tool == Tool::Select && bg.clicked() {
                self.state.clear_drawing_selection();
            }
            if let Some((px, py, wrap)) = place {
                self.state.apply(Action::AddDrawingAnnotation {
                    drawing,
                    text: "Text".to_string(),
                    pos_x: px,
                    pos_y: py,
                    wrap_frac: wrap,
                });
            }
        }

        // Aligned projection lines (#377): dashed, lightweight lines connecting each toggled
        // aligned child's silhouette extremes to its base view's, each endpoint mapped through
        // its own view's transform so the lines land on the rendered geometry.
        for (vi, view) in views.iter().enumerate() {
            if !view.align_lines {
                continue;
            }
            let (Some(lines), Some(parent)) = (
                crate::drawing::aligned_projection_lines(&self.state.doc, &views, vi),
                view.aligned_parent,
            ) else {
                continue;
            };
            let (Some((ps, pb, pc)), Some((cs, cb, cc))) = (
                view_transforms.get(parent).copied().flatten(),
                view_transforms.get(vi).copied().flatten(),
            ) else {
                continue;
            };
            let map = |p: glam::Vec2, scale: f32, bbox: egui::Vec2, center: egui::Pos2| {
                let d = (egui::vec2(p.x, p.y) - bbox) * scale;
                center + egui::vec2(d.x, -d.y)
            };
            for (ppt, cpt) in lines {
                let a = map(ppt, ps, pb, pc);
                let b = map(cpt, cs, cb, cc);
                ui.painter().add(egui::Shape::dashed_line(
                    &[a, b],
                    egui::Stroke::new(0.8, egui::Color32::from_gray(150)),
                    4.0,
                    3.0,
                ));
            }
        }

        // Aligned-view tool (#296): once a parent projection is chosen, the mouse's direction
        // from it picks the child orientation (down/up/left/right); a ghost previews it lined
        // up with the parent, and a click commits it.
        if self.state.tool == Tool::DrawingAlign {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.drawing_align_parent = None;
            }
            if let (Some(p), Some(page), Some(pp)) =
                (self.drawing_align_parent, page_rect, pointer_screen)
            {
                let parent_ok = self
                    .state
                    .doc
                    .drawings
                    .get(drawing)
                    .and_then(|d| d.views.get(p))
                    .is_some();
                if !parent_ok {
                    self.drawing_align_parent = None;
                } else {
                    let cell_w = page.width() * 0.42;
                    let cell_h = page.height() * 0.42;
                    let (rpx, rpy) =
                        crate::drawing::resolved_view_pos(&self.state.doc, drawing, p);
                    let pcenter =
                        page.min + egui::vec2(rpx * page.width(), rpy * page.height());
                    let d = pp - pcenter;
                    let dir = if d.x.abs() > d.y.abs() {
                        if d.x > 0.0 { model::AlignDir::Right } else { model::AlignDir::Left }
                    } else if d.y > 0.0 {
                        model::AlignDir::Below
                    } else {
                        model::AlignDir::Above
                    };
                    let parent_orient = self.state.doc.drawings[drawing].views[p].orientation;
                    if let Some(child) = crate::drawing::aligned_child_orientation(parent_orient, dir)
                    {
                        // Free-axis position from the cursor; shared axis follows the parent.
                        let (cx, cy) = if dir.shares_pos_x() {
                            (pcenter.x, pp.y)
                        } else {
                            (pp.x, pcenter.y)
                        };
                        let ghost = egui::Rect::from_center_size(
                            egui::pos2(cx, cy),
                            egui::vec2(cell_w, cell_h),
                        );
                        ui.painter().rect_stroke(
                            ghost,
                            2.0,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 230)),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            ghost.center(),
                            egui::Align2::CENTER_CENTER,
                            child.label(),
                            egui::FontId::proportional(13.0),
                            egui::Color32::from_rgb(130, 180, 240),
                        );
                        // Commit on a fresh click that didn't just choose the parent.
                        if !align_parent_set_this_frame
                            && ui.input(|i| i.pointer.primary_pressed())
                        {
                            let pos = if dir.shares_pos_x() {
                                ((cy - page.min.y) / page.height()).clamp(0.0, 1.0)
                            } else {
                                ((cx - page.min.x) / page.width()).clamp(0.0, 1.0)
                            };
                            let added = matches!(
                                self.state.apply(Action::AddAlignedDrawingView {
                                    drawing,
                                    parent: p,
                                    dir,
                                    pos,
                                }),
                                actions::ActionResult::Ok
                            );
                            if added {
                                let vi = self.state.doc.drawings[drawing].views.len() - 1;
                                self.state
                                    .select_drawing_only(drawing, context::DrawingElementRef::Projection(vi));
                                self.drawing_align_parent = None;
                            }
                        }
                    }
                }
            }
        } else if self.drawing_align_parent.is_some() {
            self.drawing_align_parent = None;
        }

        // Follow / end an in-flight dimension-label drag (#294): the label rides the pointer's
        // perpendicular offset from its edge, written as a dimension_offsets override.
        if let Some(d) = self.drawing_dim_label_drag {
            if ui.input(|i| i.pointer.primary_down()) {
                if let Some(pp) = pointer_screen {
                    let delta_px = (pp - d.start_pointer).dot(d.outward_screen);
                    let offset = d.start_offset + delta_px * d.mm_per_px;
                    if d.circle {
                        self.state.apply(Action::SetDrawingCircleDimOffset {
                            drawing: d.drawing,
                            view: d.view,
                            center: d.key.0,
                            offset: Some(offset),
                        });
                    } else {
                        self.state.apply(Action::SetDrawingDimensionOffset {
                            drawing: d.drawing,
                            view: d.view,
                            a: d.key.0,
                            b: d.key.1,
                            offset: Some(offset),
                        });
                    }
                }
            } else {
                self.drawing_dim_label_drag = None;
            }
        }

        // Dropping a body/sketch dragged from the Elements pane (#290) places a projection
        // at the drop point, exactly like the Add-view tool.
        if let Some(page) = page_rect {
            if bg.dnd_hover_payload::<hierarchy::DrawingDragPayload>().is_some() {
                ui.painter().rect_stroke(
                    page,
                    2.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 230)),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(payload) = bg.dnd_release_payload::<hierarchy::DrawingDragPayload>() {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()).or(bg.hover_pos()) {
                    let nx = ((pos.x - page.min.x) / page.width()).clamp(0.0, 1.0);
                    let ny = ((pos.y - page.min.y) / page.height()).clamp(0.0, 1.0);
                    let orientation = model::DrawingOrientation::default();
                    let added = match payload.0.clone() {
                        SceneElement::Body(body) => matches!(
                            self.state.apply(Action::AddDrawingView { drawing, body, orientation }),
                            actions::ActionResult::Ok
                        ),
                        SceneElement::Sketch(sketch) => matches!(
                            self.state
                                .apply(Action::AddDrawingSketchView { drawing, sketch, orientation }),
                            actions::ActionResult::Ok
                        ),
                        _ => false,
                    };
                    if added {
                        let view = self
                            .state
                            .doc
                            .drawings
                            .get(drawing)
                            .map(|d| d.views.len() - 1)
                            .unwrap_or(0);
                        self.state.apply(Action::MoveDrawingView {
                            drawing,
                            view,
                            pos_x: nx,
                            pos_y: ny,
                        });
                        self.state
                            .select_drawing_only(drawing, context::DrawingElementRef::Projection(view));
                    }
                }
            }
        }
        if let Some(view) = remove_view {
            self.state.apply(Action::RemoveDrawingView { drawing, view });
        }
        if let Some((view, pos_x, pos_y)) = move_view {
            self.state.apply(Action::MoveDrawingView { drawing, view, pos_x, pos_y });
        }
        if let Some((view, orientation)) = set_orientation {
            self.state
                .apply(Action::SetDrawingViewOrientation { drawing, view, orientation });
        }
        if let Some((view, a, b)) = toggle_dim {
            self.state
                .apply(Action::ToggleDrawingDimension { drawing, view, a, b });
        }
        if let Some((view, center)) = toggle_circle_dim {
            self.state
                .apply(Action::ToggleDrawingCircleDimension { drawing, view, center });
        }
        if let Some((view, edge1, edge2)) = toggle_angle {
            self.state
                .apply(Action::ToggleDrawingAngle { drawing, view, edge1, edge2 });
        }
        // Persist the in-progress angle pick (armed first edge) across frames.
        match pending_angle {
            Some(p) => ui.data_mut(|d| d.insert_temp(pending_angle_id, p)),
            None => ui.data_mut(|d| d.remove::<(usize, model::DrawingEdgeKey)>(pending_angle_id)),
        }

        if let Some((width_mm, height_mm, margin_mm)) = set_page {
            self.state.apply(Action::SetDrawingPage {
                drawing,
                width_mm: Some(width_mm),
                height_mm: Some(height_mm),
                margin_mm: Some(margin_mm),
            });
        }
        if pop_out {
            // Move the drawing into its own window and hand the central area back to the 3D view.
            self.drawing_window = Some(drawing);
            self.state.apply(Action::EditDrawing { drawing: None });
        }
    }

    /// Native: pick a path and write the drawing PDF. Web: generate the bytes and download.
    #[cfg(not(target_arch = "wasm32"))]
    fn export_drawing_pdf(&mut self, drawing: usize) {
        let name = crate::names::node_label(&self.state.doc, hierarchy::HierarchyNode::Drawing(drawing));
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF drawing", &["pdf"])
            .set_file_name(format!("{name}.pdf"))
            .save_file()
        {
            self.state.apply(Action::ExportDrawingPdf {
                drawing,
                path: path.to_string_lossy().to_string(),
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_drawing_svg(&mut self, drawing: usize) {
        let name = crate::names::node_label(&self.state.doc, hierarchy::HierarchyNode::Drawing(drawing));
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SVG drawing", &["svg"])
            .set_file_name(format!("{name}.svg"))
            .save_file()
        {
            self.state.apply(Action::ExportDrawingSvg {
                drawing,
                path: path.to_string_lossy().to_string(),
            });
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_drawing_svg(&mut self, drawing: usize) {
        let name = crate::names::node_label(&self.state.doc, hierarchy::HierarchyNode::Drawing(drawing));
        match crate::drawing::drawing_to_svg(&self.state.doc, drawing) {
            Some(svg) => self.web_save_bytes(
                "SVG drawing",
                &["svg"],
                format!("{name}.svg"),
                svg.into_bytes(),
                format!("Exported {name}"),
            ),
            None => self.state.status = format!("Export failed: no drawing {drawing}"),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_drawing_pdf(&mut self, drawing: usize) {
        let name = crate::names::node_label(&self.state.doc, hierarchy::HierarchyNode::Drawing(drawing));
        match crate::drawing::drawing_to_pdf(&self.state.doc, drawing) {
            Some(bytes) => self.web_save_bytes(
                "PDF drawing",
                &["pdf"],
                format!("{name}.pdf"),
                bytes,
                format!("Exported {name}"),
            ),
            None => self.state.status = format!("Export failed: no drawing {drawing}"),
        }
    }

    fn draw_viewport(
        &mut self,
        ui: &mut egui::Ui,
        render_state: Option<&eframe::egui_wgpu::RenderState>,
    ) {
        self.handle_in_progress_object_keyboard(ui);

        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let viewport = response.rect;
        self.last_viewport = Some(viewport);
        self.state.viewport_aspect = (viewport.width() / viewport.height().max(1.0)).max(0.01);
        self.state.apply_pending_sketch_reframe(viewport);
        let mut inline_parameter_field_results = Vec::<SketchDimFieldResult>::new();

        // Apply scripted right-drag as direct camera motion.
        self.synthetic.apply_pending_drag(viewport, |delta, modifiers, h| {
            if modifiers.shift {
                self.state.cam.pan(delta, h);
                if let Some(log) = &self.state.command_log {
                    log.borrow_mut().note_pan(delta);
                }
            } else {
                self.state.cam.orbit(delta);
                if let Some(log) = &self.state.command_log {
                    log.borrow_mut().note_orbit(delta);
                }
            }
        });

        // In FPS mode (#91) the mouse is the player's head and the wheel switches
        // tools, so orbit/pan/zoom stand down (`tick_fps_mode` owns the camera).
        let fps_active = self.state.fps.is_some();
        if response.dragged_by(egui::PointerButton::Secondary) && !fps_active {
            if ui.input(|i| i.modifiers.shift) {
                let delta = response.drag_delta();
                self.state.cam.pan(delta, viewport.height());
                if let Some(log) = &self.state.command_log {
                    log.borrow_mut().note_pan(delta);
                }
            } else {
                let delta = response.drag_delta();
                self.state.cam.orbit(delta);
                if let Some(log) = &self.state.command_log {
                    log.borrow_mut().note_orbit(delta);
                }
            }
        }
        // Middle-mouse drag always pans (#195). Shift+right-drag pans too, but Firefox forces
        // its native context menu on Shift+right-click regardless of preventDefault, eating the
        // gesture on the web — so middle-drag is the browser-safe way to pan there.
        if response.dragged_by(egui::PointerButton::Middle) && !fps_active {
            let delta = response.drag_delta();
            self.state.cam.pan(delta, viewport.height());
            if let Some(log) = &self.state.command_log {
                log.borrow_mut().note_pan(delta);
            }
        }
        if response.hovered() && !fps_active {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                let focal = response.hover_pos().unwrap_or(viewport.center());
                self.state.cam.zoom(scroll, focal, viewport);
                if let Some(log) = &self.state.command_log {
                    log.borrow_mut().note_zoom(scroll);
                }
            }
        }
        if fps_active {
            draw_fps_crosshair(&painter, viewport);
        }

        let cam = self.state.cam.clone();
        let vp = cam.view_proj(viewport);
        let cam_project = cam.clone();
        let project = move |w: Vec3| cam_project.project(w, viewport, &vp);

        // Occlusion context for picking (#155): tools that pick scene geometry must not
        // select things hidden behind a body. Built once per frame, only for those tools
        // (it meshes every visible body). The body-set tools (Combine, Move, Repeat, Slice,
        // Revolve) need it too, or a click can pass through a front body to one behind it (#265).
        let pick_occlusion = if matches!(
            self.state.tool,
            Tool::Select
                | Tool::Constraint
                | Tool::ConstructionPlane
                | Tool::Dimension
                | Tool::Combine
                | Tool::Move
                | Tool::Repeat
                | Tool::Slice
                | Tool::Revolve
        ) {
            Some(construction::PickOcclusion::new(
                &self.state.doc,
                &self.state.element_visibility,
                cam.eye(),
            ))
        } else {
            None
        };
        let pick_occlusion = pick_occlusion.as_ref();

        let sketch_session = self.state.sketch_session;
        let planar_label_view = sketch_session.and_then(|session| {
            sketch_geometry_frame(&self.state.doc, session.sketch)
                .map(|frame| PlanarLabelView::from_camera_and_plane(&cam, frame.normal))
        });
        let committed_dim_layouts = sketch_session.zip(planar_label_view).map(|(session, view)| {
            build_committed_dim_layouts(&painter, &project, &view, &self.state.doc, session)
        });
        let viewport_owns_pointer = self.vertex_drag.is_some()
            || self.state.line_drag_session.is_some()
            || self.dim_label_drag.is_some()
            || self.angle_gizmo_drag.is_some()
            || response.dragged_by(egui::PointerButton::Secondary);
        let pointer_screen = viewport_pointer_pos(&response, viewport_owns_pointer);
        let layouts_slice = committed_dim_layouts.as_deref().unwrap_or(&[]);
        let angle_gizmo_constraint = angle_gizmo_constraint_for_edit(
            self.state.editing_committed_dim.as_ref(),
            &self.state.doc,
        );
        if angle_gizmo_constraint.is_none() {
            self.angle_gizmo_drag = None;
        }
        let angle_dim_constraints: HashSet<usize> = layouts_slice
            .iter()
            .filter(|layout| layout.arc_geom.is_some())
            .map(|layout| layout.target)
            .collect();
        let constraint_graphics = viewport_constraints_for_selection(
            &self.state.doc,
            &self.state.element_visibility,
            &self.state.scene_selection,
            &angle_dim_constraints,
        );
        let constraint_icon_hits =
            build_constraint_icon_hits(&project, &constraint_graphics);
        let over_constraint_icon = pointer_screen.is_some_and(|pp| {
            pointer_over_constraint_icon(&constraint_icon_hits, pp).is_some()
        });
        let over_committed_dim_label = self.state.can_edit_sketch_dimensions()
            && (pointer_screen.is_some_and(|pp| {
                pointer_over_committed_dim_label(layouts_slice, pp)
            }) || self.dim_label_drag.is_some());
        if handle_committed_dim_label_double_click(ui, layouts_slice, &mut self.state) {
            self.dim_label_drag = None;
            self.angle_gizmo_drag = None;
        }
        let mut angle_gizmo_dragging = false;
        if let (Some(session), Some(active_gizmo)) =
            (sketch_session, angle_gizmo_constraint)
        {
            angle_gizmo_dragging = handle_angle_gizmo_drag(
                ui,
                layouts_slice,
                &mut self.angle_gizmo_drag,
                &mut self.state,
                session,
                viewport,
                &vp,
                &cam,
                active_gizmo,
            );
        }
        if angle_gizmo_dragging {
            self.dim_label_drag = None;
            set_viewport_cursor(
                ui.ctx(),
                &response,
                true,
                egui::CursorIcon::Grabbing,
            );
        } else if handle_committed_dim_label_drag(
            ui,
            layouts_slice,
            &mut self.dim_label_drag,
            &mut self.state,
        ) {
            self.angle_gizmo_drag = None;
            set_viewport_cursor(
                ui.ctx(),
                &response,
                true,
                egui::CursorIcon::Grabbing,
            );
        } else if over_committed_dim_label {
            set_viewport_cursor(ui.ctx(), &response, false, egui::CursorIcon::Grab);
        } else if over_constraint_icon {
            set_viewport_cursor(ui.ctx(), &response, false, egui::CursorIcon::PointingHand);
        } else if let Some(pp) = pointer_screen {
            let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
            if angle_gizmo_hit_target(
                layouts_slice,
                pp,
                &project,
                angle_gizmo_constraint,
                viewport,
            )
            .is_some()
            {
                set_viewport_cursor(ui.ctx(), &response, false, egui::CursorIcon::Grab);
            }
        }

        let mut vertex_dragging = false;
        let mut line_dragging = false;
        let mut bezier_handle_dragging = false;
        let mut text_width_dragging = false;
        // Image calibration markers (#163/#424): while placing reference points (guided
        // flow) or with a calibrated image selected, the reference points and their span
        // draw on the image's host plane; points drag to move, click to select (Delete
        // removes), and during placement a dot under the cursor previews the click.
        let calibration_target: Option<(usize, Vec<(f32, f32)>, bool)> =
            if let Some(cal) = self.state.creating_calibration.clone() {
                Some((cal.image, cal.points, true))
            } else {
                self.single_selected_tracing_image().and_then(|i| {
                    let img = self.state.doc.tracing_images.get(i)?;
                    let cal = img.calibration.as_ref()?;
                    let (ox, oy) = img.origin;
                    let (w, h) = (img.width_mm.max(1e-6), img.height_mm.max(1e-6));
                    Some((
                        i,
                        vec![
                            (ox + cal.u0 * w, oy + cal.v0 * h),
                            (ox + cal.u1 * w, oy + cal.v1 * h),
                        ],
                        false,
                    ))
                })
            };
        if let Some((image, points, placing)) = calibration_target {
            let frame = self
                .state
                .doc
                .tracing_images
                .get(image)
                .filter(|img| !img.deleted)
                .map(|img| img.plane)
                .and_then(|pi| face::sketch_frame(&self.state.doc, model::FaceId::ConstructionPlane(pi)));
            if let Some(frame) = frame {
                let cam = self.state.cam.clone();
                let local = |pp: egui::Pos2| {
                    cam.ray_plane_hit(pp, viewport, &vp, frame.origin, frame.normal)
                        .map(|hit| {
                            let d = hit - frame.origin;
                            (d.dot(frame.u_axis), d.dot(frame.v_axis))
                        })
                };
                let world =
                    |p: (f32, f32)| frame.origin + frame.u_axis * p.0 + frame.v_axis * p.1;
                let point_under = |pp: egui::Pos2| -> Option<usize> {
                    points.iter().enumerate().find_map(|(i, &pt)| {
                        let sp = project(world(pt))?;
                        ((pp - sp).length() <= 8.0).then_some(i)
                    })
                };

                let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
                let primary_down = ui.input(|i| i.pointer.primary_down());
                let primary_released = ui.input(|i| i.pointer.primary_released());

                if primary_released {
                    self.calibration_point_drag = None;
                }
                if let Some((di, idx)) = self.calibration_point_drag {
                    // Dragging a marker point follows the cursor on the plane (#424).
                    if di == image && primary_down {
                        if let Some((x, y)) = pointer_screen.and_then(&local) {
                            let _ = self.state.apply(Action::SetCalibrationPoint {
                                image,
                                index: idx,
                                x,
                                y,
                            });
                        }
                    } else {
                        self.calibration_point_drag = None;
                    }
                } else if primary_pressed {
                    if let Some(pp) = pointer_screen {
                        if let Some(idx) = point_under(pp) {
                            // Grab an existing point: select it and start dragging.
                            self.selected_calibration_point = Some((image, idx));
                            self.calibration_point_drag = Some((image, idx));
                        } else if placing && points.len() < 2 {
                            if let Some((x, y)) = local(pp) {
                                self.state.apply(Action::AddCalibrationPoint { x, y });
                            }
                        } else {
                            self.selected_calibration_point = None;
                        }
                    }
                }

                // Draw with post-click state so a just-placed point shows immediately.
                let points = self
                    .state
                    .creating_calibration
                    .as_ref()
                    .filter(|c| c.image == image)
                    .map(|c| c.points.clone())
                    .unwrap_or(points);
                for (i, &pt) in points.iter().enumerate() {
                    if let Some(sp) = project(world(pt)) {
                        let selected = self.selected_calibration_point == Some((image, i));
                        let color = if selected { egui::Color32::WHITE } else { col::PREVIEW };
                        painter.circle_filled(sp, 4.0, color);
                        painter.circle_stroke(sp, 6.0, egui::Stroke::new(1.5, color));
                    }
                }
                match points.as_slice() {
                    [a, b] => draw_world_segment(&painter, &project, world(*a), world(*b), col::PREVIEW, 2.0),
                    [a] => {
                        if let Some(cursor) = pointer_screen.and_then(&local) {
                            draw_world_segment_dashed(
                                &painter,
                                &project,
                                world(*a),
                                world(cursor),
                                col::PREVIEW,
                                1.5,
                            );
                        }
                    }
                    _ => {}
                }
                // Placement preview (#424): a dot under the cursor where a click would land.
                if placing && points.len() < 2 && self.calibration_point_drag.is_none() {
                    if let Some(sp) = pointer_screen
                        .and_then(|pp| local(pp))
                        .and_then(|p| project(world(p)))
                    {
                        painter.circle_stroke(sp, 4.0, egui::Stroke::new(1.5, col::PREVIEW));
                        painter.circle_filled(sp, 1.5, col::PREVIEW);
                    }
                }
            }
        } else {
            self.selected_calibration_point = None;
            self.calibration_point_drag = None;
        }

        if matches!(self.state.tool, Tool::Select | Tool::Constraint)
            && self.state.creating_calibration.is_none()
            && self.state.editing_committed_dim.is_none()
            && !over_committed_dim_label
            && self.dim_label_drag.is_none()
            && !angle_gizmo_dragging
            && self.angle_gizmo_drag.is_none()
        {
            if let Some(session) = sketch_session {
                let selected_text = self.single_selected_sketch_text();
                text_width_dragging = handle_text_width_drag(
                    ui,
                    &mut self.text_width_drag,
                    &mut self.state,
                    selected_text,
                    session,
                    viewport,
                    &vp,
                    &cam,
                    &project,
                    pointer_screen,
                );
                if !text_width_dragging {
                bezier_handle_dragging = handle_bezier_handle_drag(
                    ui,
                    &mut self.bezier_handle_drag,
                    &mut self.state,
                    session,
                    viewport,
                    &vp,
                    &cam,
                    &project,
                    pointer_screen,
                );
                }
                if let Some(active) = &self.bezier_handle_drag {
                    // Persists past this frame (unlike `bezier_handle_drag`, which clears on
                    // release) so a plain click — not just a drag — selects the handle (#75).
                    self.selected_bezier_handle = Some((active.line, active.near_start));
                }
                if !bezier_handle_dragging {
                    line_dragging = handle_line_drag(
                        ui,
                        &mut self.state,
                        session,
                        viewport,
                        &vp,
                        &cam,
                        &project,
                        pointer_screen,
                    );
                }
                if !text_width_dragging
                    && !bezier_handle_dragging
                    && !line_dragging
                    && self.state.line_drag_session.is_none()
                {
                    vertex_dragging = handle_vertex_drag(
                        ui,
                        &mut self.vertex_drag,
                        &mut self.state,
                        session,
                        viewport,
                        &vp,
                        &cam,
                        &project,
                        pointer_screen,
                    );
                }
                if text_width_dragging
                    || bezier_handle_dragging
                    || vertex_dragging
                    || line_dragging
                    || self.state.line_drag_session.is_some()
                {
                    set_viewport_cursor(
                        ui.ctx(),
                        &response,
                        true,
                        egui::CursorIcon::Grabbing,
                    );
                } else if let Some(pp) = pointer_screen {
                    if nearest_sketch_line_in_sketch(
                        pp,
                        &project,
                        &self.state.doc,
                        session.sketch,
                    )
                    .is_some()
                    {
                        set_viewport_cursor(ui.ctx(), &response, false, egui::CursorIcon::Grab);
                    }
                }
            }
        }

        let suppress_hover_highlight = suppress_viewport_pick_hover(
            ui,
            &response,
            self.vertex_drag.is_some(),
            self.state.line_drag_session.is_some(),
            self.dim_label_drag.is_some(),
            self.angle_gizmo_drag.is_some(),
            self.state
                .creating_plane
                .as_ref()
                .is_some_and(|cp| cp.axis_gizmo_drag.is_some()),
            self.bezier_handle_drag.is_some(),
        );

        // Right-click a bezier handle to offer deleting it, a two-line vertex to offer
        // converting it to a smooth bezier joint, or a curved line to offer straightening it
        // back out (#54/#75).
        if response.secondary_clicked() {
            self.viewport_context_menu = sketch_session.and_then(|session| {
                let pp = response.interact_pointer_pos().or(pointer_screen)?;
                if let Some((line, _)) =
                    nearest_bezier_handle_in_sketch(pp, &project, &self.state.doc, session.sketch)
                {
                    return Some(ViewportContextMenu::DeleteBezierHandle(line));
                }
                if let Some((point, _)) =
                    nearest_sketch_point_in_sketch(pp, &project, &self.state.doc, session.sketch)
                {
                    if vertex_incident_line_count(&self.state.doc, session.sketch, point.clone()) == 2 {
                        return Some(ViewportContextMenu::ConvertVertexToBezier(point));
                    }
                }
                if let Some((crate::model::ConstraintLine::Line(li), _)) =
                    nearest_sketch_line_in_sketch(pp, &project, &self.state.doc, session.sketch)
                {
                    if self.state.doc.lines.get(li).is_some_and(Line::is_curved) {
                        return Some(ViewportContextMenu::StraightenLine(li));
                    }
                }
                None
            });
        }
        if self.viewport_context_menu.is_some() {
            response.context_menu(|ui| match self.viewport_context_menu.clone() {
                Some(ViewportContextMenu::ConvertVertexToBezier(point)) => {
                    if ui.button("Convert to bezier curve").clicked() {
                        self.state.apply(Action::ConvertVertexToBezier { point });
                        self.viewport_context_menu = None;
                        ui.close();
                    }
                }
                Some(ViewportContextMenu::StraightenLine(line)) => {
                    if ui.button("Straighten curve").clicked() {
                        self.state.apply(Action::StraightenLine { line });
                        self.viewport_context_menu = None;
                        ui.close();
                    }
                }
                Some(ViewportContextMenu::DeleteBezierHandle(line)) => {
                    if ui.button("Delete handle").clicked() {
                        self.state.apply(Action::StraightenLine { line });
                        self.selected_bezier_handle = None;
                        self.viewport_context_menu = None;
                        ui.close();
                    }
                }
                None => {}
            });
        }

        if matches!(self.state.tool, Tool::Select | Tool::Constraint)
            && self.state.editing_committed_dim.is_none()
            && !over_committed_dim_label
            && self.dim_label_drag.is_none()
            && self.angle_gizmo_drag.is_none()
            && !vertex_dragging
            && !line_dragging
            && !bezier_handle_dragging
            && !text_width_dragging
            && self.vertex_drag.is_none()
            && self.state.line_drag_session.is_none()
            && self.bezier_handle_drag.is_none()
            && self.text_width_drag.is_none()
        {
            if let Some(pp) = pointer_screen {
                let gp = cam.ground_point(pp, viewport, &vp);
                if ui.input(|i| i.pointer.primary_pressed()) {
                    // This whole block only runs when no bezier handle was just grabbed (see
                    // the gating `&& self.bezier_handle_drag.is_none()` above), so any click
                    // reaching here selects something else — clear the handle selection (#75).
                    self.selected_bezier_handle = None;
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    // Body vertices outrank edges/other targets, mirroring the hover
                    // priority in `resolve_viewport_hover_highlight` (#144/#156) — what the
                    // hover shows is what the click selects.
                    let body_vertex = construction::nearest_body_vertex(pp, &project, &self.state.doc)
                        .filter(|(kind, _)| {
                            // Hidden/shadow bodies aren't selectable (#258); occluded ones aren't
                            // either (#155).
                            pick_occlusion.is_none_or(|occ| occ.pickable(&self.state.doc, kind))
                                && match kind {
                                    construction::PickTargetKind::BodyVertex { position, .. } => {
                                        pick_occlusion.is_none_or(|occ| !occ.occluded(*position))
                                    }
                                    _ => true,
                                }
                        })
                        .and_then(|(kind, _)| scene_element_from_pick(&kind));
                    if let Some(index) =
                        pointer_over_constraint_icon(&constraint_icon_hits, pp)
                    {
                        self.state.apply(Action::ClickSceneElement {
                            element: SceneElement::Constraint(index),
                            additive,
                        });
                    } else if let Some(element) = body_vertex {
                        self.state
                            .apply(Action::ClickSceneElement { element, additive });
                    } else if let Some(target) =
                        resolve_pick_target(pp, &project, gp, &self.state.doc, pick_occlusion)
                    {
                        if let Some(element) = scene_element_from_pick(&target.kind) {
                            self.state
                                .apply(Action::ClickSceneElement { element, additive });
                        } else if !additive {
                            self.state.apply(Action::ClearSceneSelection);
                        }
                    } else if !additive {
                        self.state.apply(Action::ClearSceneSelection);
                    }
                } else if !self.gpu_viewport && !suppress_hover_highlight {
                    if let Some(target) = resolve_pick_target(pp, &project, gp, &self.state.doc, pick_occlusion) {
                        if scene_element_from_pick(&target.kind).is_some() {
                            target.draw_highlight(&painter, &project, &self.state.doc);
                        }
                    }
                }
            }
        }

        if self.state.tool == Tool::Sketch {
            if let Some(pp) = pointer_screen {
                if ui.input(|i| i.pointer.primary_pressed()) {
                    if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                        self.state.apply(Action::BeginSketch {
                            face,
                            viewport: Some(viewport),
                        });
                    }
                } else if !self.gpu_viewport && !suppress_hover_highlight {
                    if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                        draw_face_highlight(
                            &painter,
                            &project,
                            &self.state.doc,
                            face,
                            construction::PICK_HOVER_RGBA,
                        );
                    }
                }
            }
        }

        if self.state.tool == Tool::Rectangle {
            if self.state.sketch_session.is_none() {
                if let Some(pp) = pointer_screen {
                    if ui.input(|i| i.pointer.primary_pressed()) {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            self.state.apply(Action::BeginSketch {
                                face,
                                viewport: Some(viewport),
                            });
                        }
                    } else if !self.gpu_viewport && !suppress_hover_highlight {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            draw_face_highlight(
                                &painter,
                                &project,
                                &self.state.doc,
                                face,
                                construction::PICK_HOVER_RGBA,
                            );
                        }
                    }
                }
            } else if let (Some(session), Some(pp)) =
                (self.state.sketch_session, pointer_screen)
            {
                if let Some(gp) =
                    sketch_plane_point(&cam, viewport, &vp, &self.state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
                    let was_creating = self.state.creating_rect.is_some();
                    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
                    let (sgp, snap_target) =
                        snap_ground_point(&self.state, session, &frame, &project, gp, &[]);
                    update_extension_anchors(&mut self.state, snap_target.clone());

                    if !was_creating && primary_pressed && !over_committed_dim_label {
                        self.state.rect_origin_snap = snap_target.clone();
                        self.state.rect_opposite_snap = None;
                        self.state.creating_rect = Some(CreatingRect {
                            origin: sgp,
                            texts: ["".to_string(), "".to_string()],
                            focused: 0,
                            last_mouse: sgp,
                            user_edited: [false, false],
                            pending_focus: true,
                            construction: self.state.draw_construction,
                        });
                        self.state.status = "Move mouse • type to lock dim • Tab cycle • click/Enter commit • Esc cancel"
                            .to_string();
                    }

                    let mut commit_click = false;
                    if let Some(cr) = &mut self.state.creating_rect {
                        let end = cr.end_point(&frame, &self.state.doc);
                        let (ou, ov) = world_to_local(&frame, cr.origin);
                        let (eu, ev) = world_to_local(&frame, end);
                        let corners = preview_rect_world_corners(&frame, ou, ov, eu, ev);
                        let dim_layouts = rectangle_dim_layout_from_corners(
                            &project,
                            corners,
                            &cr.texts[0],
                            &cr.texts[1],
                        );
                        let over_input = dim_layouts
                            .as_ref()
                            .is_some_and(|(w, h)| w.rect.contains(pp) || h.rect.contains(pp));

                        if should_commit_sketch_on_click(
                            was_creating,
                            primary_pressed,
                            over_input || over_committed_dim_label,
                        ) {
                            commit_click = true;
                        } else if !over_input && !over_committed_dim_label {
                            cr.last_mouse = sgp;
                            let (au, av) = world_to_local(&frame, cr.origin);
                            let (bu, bv) = world_to_local(&frame, sgp);
                            if !cr.user_edited[0] {
                                cr.texts[0] = format_live_dimension((bu - au).abs());
                            }
                            if !cr.user_edited[1] {
                                cr.texts[1] = format_live_dimension((bv - av).abs());
                            }
                            // The opposite corner only tracks the cursor when both dims are free.
                            self.state.rect_opposite_snap =
                                if cr.user_edited[0] || cr.user_edited[1] {
                                    None
                                } else {
                                    snap_target
                                };
                        }
                    }
                    if commit_click {
                        self.state.apply(Action::CommitRectangle);
                    }
                }
            }
        }

        if self.state.tool == Tool::Circle {
            if self.state.sketch_session.is_none() {
                if let Some(pp) = pointer_screen {
                    if ui.input(|i| i.pointer.primary_pressed()) {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            self.state.apply(Action::BeginSketch {
                                face,
                                viewport: Some(viewport),
                            });
                        }
                    } else if !self.gpu_viewport && !suppress_hover_highlight {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            draw_face_highlight(
                                &painter,
                                &project,
                                &self.state.doc,
                                face,
                                construction::PICK_HOVER_RGBA,
                            );
                        }
                    }
                }
            } else if let (Some(session), Some(pp)) =
                (self.state.sketch_session, pointer_screen)
            {
                if let Some(gp) =
                    sketch_plane_point(&cam, viewport, &vp, &self.state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
                    let was_creating = self.state.creating_circle.is_some();
                    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

                    if !was_creating && primary_pressed && !over_committed_dim_label {
                        // Snap the center; the rim follows the cursor freely.
                        let (center, center_snap) =
                            snap_ground_point(&self.state, session, &frame, &project, gp, &[]);
                        update_extension_anchors(&mut self.state, center_snap.clone());
                        self.state.circle_center_snap = center_snap;
                        self.state.creating_circle = Some(CreatingCircle {
                            origin: center,
                            text: String::new(),
                            last_mouse: gp,
                            user_edited: false,
                            pending_focus: true,
                            construction: self.state.draw_construction,
                        });
                        self.state.status = "Move mouse • type to lock diameter • click/Enter commit • Esc cancel"
                            .to_string();
                    }

                    let mut commit_click = false;
                    if let Some(cc) = &mut self.state.creating_circle {
                        let rim = cc.rim_point(&frame, &self.state.doc);
                        let over_input = project(cc.origin).zip(project(rim)).is_some_and(
                            |(pa, pb)| {
                                pointer_over_dim_inputs(pp, &[line_dim_layout(pa, pb, &cc.text)])
                            },
                        );

                        if should_commit_sketch_on_click(
                            was_creating,
                            primary_pressed,
                            over_input || over_committed_dim_label,
                        ) {
                            commit_click = true;
                        } else if !over_input && !over_committed_dim_label {
                            cc.last_mouse = gp;
                            if !cc.user_edited {
                                let radius = cc.radius(&frame, &self.state.doc);
                                cc.text = format_live_dimension(radius * 2.0);
                            }
                        }
                    }
                    if commit_click {
                        self.state.apply(Action::CommitCircle);
                    }
                }
            }
        }

        if self.state.tool == Tool::Line {
            if self.state.sketch_session.is_none() {
                if let Some(pp) = pointer_screen {
                    if ui.input(|i| i.pointer.primary_pressed()) {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            self.state.apply(Action::BeginSketch {
                                face,
                                viewport: Some(viewport),
                            });
                        }
                    } else if !self.gpu_viewport && !suppress_hover_highlight {
                        if let Some(face) = pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye()) {
                            draw_face_highlight(
                                &painter,
                                &project,
                                &self.state.doc,
                                face,
                                construction::PICK_HOVER_RGBA,
                            );
                        }
                    }
                }
            } else if let (Some(session), Some(pp)) =
                (self.state.sketch_session, pointer_screen)
            {
                if let Some(gp) =
                    sketch_plane_point(&cam, viewport, &vp, &self.state.doc, session, pp)
                {
                    let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
                    let was_creating = self.state.creating_line.is_some();
                    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

                    // Snap the cursor to nearby geometry (vertices, midpoints, lines).
                    let (sgp, snap_target) =
                        snap_ground_point(&self.state, session, &frame, &project, gp, &[]);
                    update_extension_anchors(&mut self.state, snap_target.clone());

                    if !was_creating && primary_pressed && !over_committed_dim_label {
                        self.state.line_start_snap = snap_target.clone();
                        self.state.line_end_snap = None;
                        self.state.creating_line = Some(CreatingLine {
                            origin: sgp,
                            text: String::new(),
                            last_mouse: sgp,
                            user_edited: false,
                            pending_focus: true,
                            construction: self.state.draw_construction,
                            curve_mode: self.state.draw_curve_mode,
                            tangent_constraint: self.state.draw_tangent_constraint,
                            chained_from: None,
                            chained_from_bezier: None,
                        });
                        self.state.status = "Move mouse • type to lock length • click/Enter commit • Esc cancel"
                            .to_string();
                    }

                    let mut commit_click = false;
                    if let Some(cl) = &mut self.state.creating_line {
                        let end = cl.end_point(&frame, &self.state.doc);
                        let over_input = project(cl.origin).zip(project(end)).is_some_and(
                            |(pa, pb)| {
                                pointer_over_dim_inputs(pp, &[line_dim_layout(pa, pb, &cl.text)])
                            },
                        );

                        if should_commit_sketch_on_click(
                            was_creating,
                            primary_pressed,
                            over_input || over_committed_dim_label,
                        ) {
                            commit_click = true;
                        } else if !over_input && !over_committed_dim_label {
                            cl.last_mouse = sgp;
                            // A typed length overrides the free end, so the snap no longer applies.
                            self.state.line_end_snap = if cl.user_edited {
                                None
                            } else {
                                let (au, av) = world_to_local(&frame, cl.origin);
                                let (bu, bv) = world_to_local(&frame, sgp);
                                let du = bu - au;
                                let dv = bv - av;
                                cl.text = format_live_dimension((du * du + dv * dv).sqrt());
                                snap_target
                            };
                        }
                    }
                    // #73: while curve-mode is on and this segment chains from a previous one,
                    // live-preview the smoothed (or corner-ized) joint by temporarily bending the
                    // previous line's end handle toward the live mouse position every frame —
                    // recomputed fresh each time, so it updates as the mouse moves and is cheap
                    // to redo. `Action::CommitLine` performs the same computation permanently
                    // once the point is actually placed.
                    if let Some(cl) = &self.state.creating_line {
                        if let Some(prev_idx) = cl.chained_from {
                            if let Some(prev_far) =
                                self.state.doc.lines.get(prev_idx).map(|l| (l.x0, l.y0))
                            {
                                let (u0, v0) = world_to_local(&frame, cl.origin);
                                let end = cl.end_point(&frame, &self.state.doc);
                                let (u1, v1) = world_to_local(&frame, end);
                                let (prev_bezier, _) = chained_curve_handles(
                                    prev_far,
                                    cl.chained_from_bezier,
                                    (u0, v0),
                                    (u1, v1),
                                    cl.curve_mode,
                                    cl.tangent_constraint,
                                );
                                if let Some(prev) = self.state.doc.lines.get_mut(prev_idx) {
                                    prev.bezier = prev_bezier;
                                }
                            }
                        }
                    }
                    if commit_click {
                        self.state.apply(Action::CommitLine);
                    }
                }
            }
        }

        if self.state.tool == Tool::Extrude {
            self.handle_extrude_tool(ui, &project, pointer_screen, &cam, viewport, &vp);
            self.show_extrude_distance_input(ui, &project);
        }

        if self.state.tool == Tool::Loft {
            self.handle_loft_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
        }

        if self.state.tool == Tool::Revolve {
            self.handle_revolve_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
            self.show_revolve_angle_input(ui, &project);
        }

        if self.state.tool == Tool::Combine {
            self.handle_combine_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
        }

        if self.state.tool == Tool::Move {
            self.handle_move_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
        }

        if self.state.tool == Tool::Repeat {
            self.handle_repeat_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
        }

        if self.state.tool == Tool::Slice {
            self.handle_slice_tool(ui, &project, pointer_screen, &cam, viewport, &vp, pick_occlusion);
        }

        if self.state.tool == Tool::Text {
            // Outside a sketch, the Text tool clicks a face to begin sketching there (#383),
            // exactly like the Rectangle/Line/Circle draw tools; the text placement then
            // starts on the next press inside the new sketch.
            if self.state.sketch_session.is_none() {
                if let Some(pp) = pointer_screen {
                    if ui.input(|i| i.pointer.primary_pressed()) {
                        if let Some(face) =
                            pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye())
                        {
                            self.state.apply(Action::BeginSketch {
                                face,
                                viewport: Some(viewport),
                            });
                        }
                    } else if !self.gpu_viewport && !suppress_hover_highlight {
                        if let Some(face) =
                            pick_sketch_face(pp, &project, &self.state.doc, self.state.cam.eye())
                        {
                            draw_face_highlight(
                                &painter,
                                &project,
                                &self.state.doc,
                                face,
                                construction::PICK_HOVER_RGBA,
                            );
                        }
                    }
                }
            } else {
                self.handle_text_tool(ui, &painter, &project, pointer_screen, &cam, viewport, &vp);
            }
        }

        if matches!(self.state.tool, Tool::Chamfer | Tool::Fillet) {
            self.handle_vertex_treatment_tool(ui, &project, pointer_screen);
            self.show_vertex_treatment_amount_input(ui, &project);
            self.handle_edge_treatment_tool(ui, &project, pointer_screen);
            self.show_edge_treatment_amount_input(ui, &project);
        }

        if self.state.tool == Tool::Dimension {
            if let (Some(session), Some(pp)) =
                (self.state.sketch_session, pointer_screen)
            {
                if let Some(gp) =
                    sketch_plane_point(&cam, viewport, &vp, &self.state.doc, session, pp)
                {
                    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
                    if self.state.editing_committed_dim.is_none()
                        && primary_pressed
                        && !over_committed_dim_label
                    {
                        if let Some(target) =
                            resolve_pick_target(pp, &project, Some(gp), &self.state.doc, pick_occlusion)
                        {
                            if let Some(distance_target) = distance_target_from_pick(
                                &self.state.doc,
                                session.sketch,
                                &target.kind,
                            ) {
                                self.state.apply(Action::BeginDimensionEdit {
                                    target: model::DimensionTarget::Distance(distance_target),
                                });
                            }
                        }
                    } else if self.state.editing_committed_dim.is_none()
                        && !suppress_hover_highlight
                        // In GPU mode the dimensionable segment glows through the shared
                        // `resolve_viewport_hover_highlight` path (#190); this painter overlay
                        // is only the fallback for the non-GPU renderer.
                        && !self.gpu_viewport
                    {
                        if let Some(target) =
                            resolve_pick_target(pp, &project, Some(gp), &self.state.doc, pick_occlusion)
                        {
                            if distance_target_from_pick(
                                &self.state.doc,
                                session.sketch,
                                &target.kind,
                            )
                            .is_some()
                            {
                                target.draw_highlight(&painter, &project, &self.state.doc);
                            }
                        }
                    }
                }
            }
        }

        if let Some(placing) = self.state.placing_angle_dimension.clone() {
            if let Some(session) = self.state.sketch_session {
                if let Some(frame) = sketch_geometry_frame(&self.state.doc, session.sketch) {
                    if let Some(pp) = pointer_screen {
                        if let Some(hover_world) =
                            cam.ray_plane_hit(pp, viewport, &vp, frame.origin, frame.normal)
                        {
                            if let Some(sign) = angle_dimension_hover_sign(
                                &self.state.doc,
                                placing.line_a.clone(),
                                placing.line_b.clone(),
                                hover_world,
                            ) {
                                if let Some(p) = self.state.placing_angle_dimension.as_mut() {
                                    p.rotation_sign = sign;
                                }
                            }
                        }
                        // The arc radius tracks the cursor's distance from the vertex, so the
                        // preview grows/shrinks and clearly reads as an angle wedge (#188).
                        if let Some(display) = crate::constraints::angle_constraint_display(
                            &self.state.doc,
                            placing.line_a.clone(),
                            placing.line_b.clone(),
                            placing.rotation_sign,
                        ) {
                            if let Some(center_px) = project(display.center) {
                                let px = (pp - center_px).length();
                                if let Some(p) = self.state.placing_angle_dimension.as_mut() {
                                    p.arc_offset = Some(px);
                                }
                            }
                        }
                    }
                    // Re-read: the hover update above may have just flipped the sign / resized.
                    let placing = self.state.placing_angle_dimension.clone().unwrap_or(placing);
                    let label = default_angle_expression(
                        &self.state.doc,
                        session.sketch,
                        placing.line_a.clone(),
                        placing.line_b.clone(),
                        placing.rotation_sign,
                    );
                    draw_angle_dim_for_lines(
                        &painter,
                        &project,
                        &frame,
                        &self.state.doc,
                        placing.line_a.clone(),
                        placing.line_b.clone(),
                        placing.rotation_sign,
                        placing.arc_offset,
                        &label,
                        false,
                        false,
                    );
                    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
                    if primary_pressed && !over_committed_dim_label {
                        let arc_offset = placing.arc_offset;
                        self.state.placing_angle_dimension = None;
                        self.state.apply(Action::BeginDimensionEdit {
                            target: model::DimensionTarget::Angle {
                                line_a: placing.line_a,
                                line_b: placing.line_b,
                                rotation_sign: placing.rotation_sign,
                            },
                        });
                        // Carry the previewed radius onto the edit so commit persists it.
                        if let Some(edit) = self.state.editing_committed_dim.as_mut() {
                            edit.dim_offset = arc_offset;
                        }
                    }
                }
            }
        }

        if self.state.tool == Tool::ConstructionPlane {
            let ground = |p: egui::Pos2| cam.ground_point(p, viewport, &vp);

            if let Some(pp) = pointer_screen {
                let gp = ground(pp);
                let was_creating = self.state.creating_plane.is_some();
                let primary_pressed = ui.input(|i| i.pointer.primary_pressed());

                if !was_creating && primary_pressed {
                    if let Some(target) =
                        resolve_pick_target(pp, &project, gp, &self.state.doc, pick_occlusion)
                    {
                        let parent = parent_from_pick_target(&self.state.doc, target.kind);
                        self.state.apply(Action::BeginConstructionPlane {
                            reference: target.reference,
                            parent,
                        });
                    }
                }

                let mut commit_click = false;
                if let Some(cp) = &mut self.state.creating_plane {
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    let primary_down = ui.input(|i| i.pointer.primary_down());
                    let primary_released = ui.input(|i| i.pointer.primary_released());

                    if primary_pressed {
                        match &cp.reference {
                            PlaneReference::Axis {
                                origin,
                                direction,
                                ..
                            } => {
                                if let Some(hit) = axis_gizmo_hit(
                                    pp,
                                    &project,
                                    *origin,
                                    *direction,
                                    cp.offset_live,
                                    cp.axis_angle_deg,
                                ) {
                                    cp.axis_gizmo_drag = Some(AxisGizmoDrag {
                                        hit,
                                        start_offset: cp.offset_live,
                                        start_angle_deg: cp.axis_angle_deg,
                                        start_screen: pp,
                                    });
                                    cp.user_edited_offset = false;
                                    cp.user_edited_angle = false;
                                }
                            }
                            PlaneReference::Face { origin, normal, .. } => {
                                if offset_gizmo_hit(
                                    pp,
                                    &project,
                                    *origin,
                                    *normal,
                                    cp.offset_live,
                                ) {
                                    cp.axis_gizmo_drag = Some(AxisGizmoDrag {
                                        hit: AxisGizmoHit::Offset,
                                        start_offset: cp.offset_live,
                                        start_angle_deg: 0.0,
                                        start_screen: pp,
                                    });
                                    cp.user_edited_offset = false;
                                }
                            }
                        }
                    }

                    let gizmo_drag = cp.axis_gizmo_drag;
                    if let Some(drag) = gizmo_drag {
                        if primary_down {
                            match drag.hit {
                                AxisGizmoHit::Offset => {
                                    let (origin, normal) = match &cp.reference {
                                        PlaneReference::Face { origin, normal, .. } => {
                                            (*origin, normal.normalize_or_zero())
                                        }
                                        PlaneReference::Axis {
                                            origin,
                                            direction,
                                            ..
                                        } => (
                                            *origin,
                                            axis_normal(*direction, drag.start_angle_deg),
                                        ),
                                    };
                                    cp.offset_live = offset_from_normal_drag(
                                        origin,
                                        normal,
                                        &project,
                                        drag.start_offset,
                                        drag.start_screen,
                                        pp,
                                    );
                                }
                                AxisGizmoHit::Angle => {
                                    if let PlaneReference::Axis {
                                        origin,
                                        direction,
                                        ..
                                    } = &cp.reference
                                    {
                                        if let Some(hit) = cam.ray_plane_hit(
                                            pp, viewport, &vp, *origin, *direction,
                                        ) {
                                            cp.axis_angle_deg = angle_from_axis_plane_hit(
                                                *origin, *direction, hit,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if primary_released {
                        cp.axis_gizmo_drag = None;
                    }

                    if scroll != 0.0
                        && !cp.user_edited_offset
                        && cp.axis_gizmo_drag.is_none()
                    {
                        cp.offset_live += scroll * 0.05;
                    }

                    if !cp.user_edited_offset {
                        let (off, ang) = cp.live_dims();
                        cp.offset_text = format_live_dimension(off);
                        if cp.reference.is_axis() && !cp.user_edited_angle {
                            cp.angle_text = format!("{:.0}", ang);
                        }
                    }

                    let preview = cp.preview_plane();
                    let dim_layouts = plane_dim_layouts(
                        &project,
                        &preview,
                        &cp.reference,
                        cp.offset_live,
                        cp.axis_angle_deg,
                    );
                    let over_input = dim_layouts.as_ref().is_some_and(|(offset, angle)| {
                        let mut layouts = vec![*offset];
                        if let Some(angle) = angle {
                            layouts.push(*angle);
                        }
                        pointer_over_dim_inputs(pp, &layouts)
                    });
                    let over_gizmo = match &cp.reference {
                        PlaneReference::Face { origin, normal, .. } => offset_gizmo_hit(
                            pp,
                            &project,
                            *origin,
                            *normal,
                            cp.offset_live,
                        ),
                        PlaneReference::Axis {
                            origin,
                            direction,
                            ..
                        } => axis_gizmo_hit(
                            pp,
                            &project,
                            *origin,
                            *direction,
                            cp.offset_live,
                            cp.axis_angle_deg,
                        )
                        .is_some(),
                    };

                    if should_commit_sketch_on_click(
                        was_creating,
                        primary_pressed,
                        over_input || over_gizmo || cp.axis_gizmo_drag.is_some(),
                    ) {
                        commit_click = true;
                    }
                }
                if commit_click {
                    self.state.apply(Action::CommitConstructionPlane);
                }
            }
        }

        let doc = &self.state.doc;
        let editing_constraint = self.state.editing_committed_dim.as_ref().and_then(|edit| {
            match &edit.target {
                DimEditTarget::Constraint(id) => Some(*id),
                DimEditTarget::New(_) => None,
            }
        });
        let gpu_dim_labels = if self.gpu_viewport {
            committed_dim_layouts
                .as_ref()
                .zip(planar_label_view)
                .map(|(layouts, view)| {
                    build_gpu_dimension_labels(
                        ui.ctx(),
                        layouts,
                        &view,
                        &cam,
                        viewport,
                        &vp,
                        &project,
                        editing_constraint,
                        &self.state.document_health,
                    )
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let plane_gizmo = self.state.creating_plane.as_ref().map(|cp| {
            gpu_viewport::ViewportPlaneGizmo {
                reference: cp.reference.clone(),
                offset: cp.offset_live,
                angle_deg: cp.axis_angle_deg,
                color: col::PREVIEW,
                hover: plane_gizmo_hover(cp, pointer_screen, &project),
            }
        });
        let mut hover_highlight = resolve_viewport_hover_highlight(
            suppress_hover_highlight,
            self.state.tool,
            sketch_session,
            self.state.creating_plane.is_some(),
            self.state.editing_committed_dim.is_some(),
            over_committed_dim_label,
            self.dim_label_drag.is_some(),
            pointer_screen,
            &cam,
            viewport,
            &vp,
            doc,
            &project,
            pick_occlusion,
        );
        // Elements-pane hover wins (#161): the mouse is over the pane, so no viewport pick
        // is active anyway; show the hovered row's element instead.
        if let Some(element) = self.pane_hovered_element.take() {
            hover_highlight = Some(gpu_viewport::ViewportHoverHighlight::Element(element));
        }
        // Extrude tool: highlight the face under the cursor and render the normal gizmo (same
        // arrow as the construction-plane offset gizmo) through the GPU scene.
        let mut extrude_gizmo = None;
        if self.state.tool == Tool::Extrude {
            if self.extrude_gizmo_drag.is_none() {
                hover_highlight = pointer_screen
                    .and_then(|pp| {
                        // A sketch text under the cursor (#285/#307): hover the whole text so
                        // it's clear a click picks all its glyph faces at once.
                        let faces = self.text_glyph_faces_at(pp, &cam, viewport, &vp)?;
                        let model::ExtrudeFace::TextGlyph { text, .. } = faces.first()? else {
                            return None;
                        };
                        Some(gpu_viewport::ViewportHoverHighlight::Element(
                            SceneElement::SketchText(*text),
                        ))
                    })
                    .or_else(|| {
                        pointer_screen
                            .and_then(|pp| {
                                pick_extrude_face(pp, &project, doc, cam.eye(), &cam, viewport, &vp)
                            })
                            .and_then(|f| {
                                // A `Boolean` region has no `FaceId` of its own (see
                                // `ExtrudeFace::face_id()`'s doc comment) — highlight its exact
                                // resolved loop instead of falling back to a whole-shape
                                // outline, so the user can see the intersection/difference
                                // area distinctly.
                                if let model::ExtrudeFace::Boolean { .. } = &f {
                                    let (profile, _) = extrude::face_profile_world(doc, &f)?;
                                    Some(gpu_viewport::ViewportHoverHighlight::ClosedLoop {
                                        world_loop: profile,
                                    })
                                } else {
                                    Some(gpu_viewport::ViewportHoverHighlight::SketchFace(
                                        extrude_face_id(f),
                                    ))
                                }
                            })
                    })
                    .or_else(|| {
                        // A bare body face (#122): no sketch profile, but still highlighted so
                        // it's clear clicking will push/pull it directly.
                        let pp = pointer_screen?;
                        let face_id = pick_extrude_body_face(pp, &project, doc, cam.eye())?;
                        Some(gpu_viewport::ViewportHoverHighlight::SketchFace(face_id))
                    });
            }
            if let Some(ce) = self.state.creating_extrusion.as_ref() {
                if let Some((origin, normal)) = extrude::faces_anchor(doc, &ce.faces) {
                    let handle_offset =
                        extrude_gizmo_display_offset(ce.evaluated_distance(doc));
                    let hovered = self.extrude_gizmo_drag.is_some()
                        || pointer_screen.is_some_and(|pp| {
                            construction::offset_gizmo_hit(pp, &project, origin, normal, handle_offset)
                        });
                    extrude_gizmo = Some(gpu_viewport::ViewportExtrudeGizmo {
                        origin,
                        normal,
                        offset: handle_offset,
                        color: col::PREVIEW,
                        hovered,
                    });
                }
            }
        }
        // Revolve tool (#303): hover the profile face under the cursor, same affordance as
        // the Extrude tool — clicking it picks the profile. Anything else keeps the generic
        // hover (lines still light up for the axis pick).
        if self.state.tool == Tool::Revolve
            && self.revolve_gizmo_drag.is_none()
            && self.state.sketch_session.is_none()
        {
            hover_highlight = pointer_screen
                .and_then(|pp| {
                    pick_extrude_face(pp, &project, doc, cam.eye(), &cam, viewport, &vp)
                })
                .and_then(|f| {
                    if let model::ExtrudeFace::Boolean { .. } = &f {
                        let (profile, _) = extrude::face_profile_world(doc, &f)?;
                        Some(gpu_viewport::ViewportHoverHighlight::ClosedLoop {
                            world_loop: profile,
                        })
                    } else {
                        Some(gpu_viewport::ViewportHoverHighlight::SketchFace(
                            extrude_face_id(f),
                        ))
                    }
                })
                .or(hover_highlight);
        }
        // Chamfer/fillet tool (#144): before an edge is picked, highlight the treatable edge
        // under the cursor so it's clear which one clicking will chamfer/fillet. Only in the 3D
        // case (no sketch open) and while no treatment is in progress (else the gizmo shows).
        if matches!(self.state.tool, Tool::Chamfer | Tool::Fillet)
            && self.state.sketch_session.is_none()
            && self.state.creating_edge_treatment.is_none()
            && !suppress_hover_highlight
        {
            if let Some(pp) = pointer_screen {
                if let Some((extrusion, _, a, b, _)) =
                    construction::nearest_treatable_edge(pp, &project, doc)
                {
                    let body = doc
                        .bodies
                        .iter()
                        .position(|body| body.source.extrusion_indices().contains(&extrusion))
                        .unwrap_or(extrusion);
                    hover_highlight = Some(gpu_viewport::ViewportHoverHighlight::PickTarget(
                        construction::PickTargetKind::BodyEdge { body, a, b },
                    ));
                }
            }
        }
        // Chamfer/fillet tool: render the same push/pull gizmo the extrude tool uses, anchored
        // at the picked vertex and pointing along the inward bisector of its two lines. Shares
        // one gizmo slot between the 2D (sketch vertex) and 3D (extrusion edge, #77) cases,
        // since exactly one of the two can be active at a time (one needs a sketch open, the
        // other needs it closed).
        let mut vertex_treatment_gizmo = None;
        let mut vertex_treatment_preview = None;
        // Revolve arc gizmo (#262): an arc swept to the current angle with a push/pull disc
        // handle at its far end, dragged around the arc to set the sweep angle.
        let revolve_arc_gizmo = (self.state.tool == Tool::Revolve)
            .then(|| self.revolve_arc_geom())
            .flatten()
            .map(|(center, axis, zero_dir, radius)| {
                let angle = self
                    .state
                    .creating_revolve
                    .as_ref()
                    .map(|c| c.evaluated_angle_deg(&self.state.doc))
                    .unwrap_or(360.0);
                let handle = revolve_arc_handle_world(center, axis, zero_dir, radius, angle);
                let hovered = self.revolve_gizmo_drag.is_some()
                    || pointer_screen.is_some_and(|pp| {
                        project(handle)
                            .is_some_and(|hp| (hp - pp).length() <= REVOLVE_ARC_HANDLE_PICK_PX)
                    });
                gpu_viewport::RevolveArcGizmo {
                    center,
                    axis,
                    zero_dir,
                    radius,
                    angle_deg: angle,
                    color: col::PREVIEW,
                    hovered,
                }
            });
        if matches!(self.state.tool, Tool::Chamfer | Tool::Fillet) {
            if let (Some(session), Some(cvt)) =
                (self.state.sketch_session, self.state.creating_vertex_treatment.as_ref())
            {
                if let Some((origin, normal)) =
                    vertex_treatment_anchor(doc, session.sketch, cvt.point.clone())
                {
                    let handle_offset =
                        construction::gizmo_display_offset(cvt.evaluated_amount(doc));
                    let hovered = self.vertex_treatment_gizmo_drag.is_some()
                        || pointer_screen.is_some_and(|pp| {
                            construction::offset_gizmo_hit(pp, &project, origin, normal, handle_offset)
                        });
                    vertex_treatment_gizmo = Some(gpu_viewport::ViewportExtrudeGizmo {
                        origin,
                        normal,
                        offset: handle_offset,
                        color: col::PREVIEW,
                        hovered,
                    });
                }
                // Live geometry preview of the treated corner (#76): recomputed every frame
                // from the live gizmo amount, both while first placing the gizmo and while
                // dragging it.
                vertex_treatment_preview =
                    vertex_treatment_preview_points(doc, session.sketch, cvt);
            } else if let Some(cet) = self.state.creating_edge_treatment.as_ref() {
                if let Some((origin, normal)) = cet.primary().and_then(|(extrusion, edge)| {
                    crate::extrude::extrusion_edge_anchor(doc, extrusion, edge)
                }) {
                    let handle_offset =
                        construction::gizmo_display_offset(cet.evaluated_amount(doc));
                    let hovered = self.edge_treatment_gizmo_drag.is_some()
                        || pointer_screen.is_some_and(|pp| {
                            construction::offset_gizmo_hit(pp, &project, origin, normal, handle_offset)
                        });
                    vertex_treatment_gizmo = Some(gpu_viewport::ViewportExtrudeGizmo {
                        origin,
                        normal,
                        offset: handle_offset,
                        color: col::PREVIEW,
                        hovered,
                    });
                }
            }
        }
        // Every tool that gathers a set through an element picker (#213) shows that picked set
        // with the selection highlight in the viewport — "all currently selected elements on a
        // focused element picker should be styled as selected" — without disturbing the
        // persistent selection. Fold each active tool's picked SceneElements into a throwaway
        // selection used only for the scene. (Chamfer/Fillet edges are highlighted separately
        // via `creating_edge_treatment` passed to the scene builder.)
        let mut folded: Vec<SceneElement> = Vec::new();
        // Bodies picked into a destructive (cut) picker are highlighted red instead of the
        // blue selection style (#213), so they aren't folded into `render_selection`.
        let mut cut_highlight_bodies: Vec<usize> = Vec::new();
        match self.state.tool {
            Tool::Loft => {
                if let Some(cl) = self.state.creating_loft.as_ref() {
                    for section in &cl.sections {
                        folded.extend(extrude::loft_section_scene_elements(section));
                    }
                }
            }
            Tool::Combine => {
                if let Some(cb) = self.state.creating_boolean.as_ref() {
                    folded.extend(cb.a.iter().map(|&bi| SceneElement::Body(bi)));
                    // In a Cut, the B side is carved away — highlight it red; otherwise blue.
                    if cb.kind == model::BooleanOpKind::Cut {
                        cut_highlight_bodies.extend(cb.b.iter().copied());
                    } else {
                        folded.extend(cb.b.iter().map(|&bi| SceneElement::Body(bi)));
                    }
                }
            }
            Tool::Move => {
                if let Some(cm) = self.state.creating_move.as_ref() {
                    folded.extend(cm.targets.iter().map(|&bi| SceneElement::Body(bi)));
                }
            }
            Tool::Repeat => {
                if let Some(cr) = self.state.creating_repeat.as_ref() {
                    folded.extend(cr.targets.iter().map(|&bi| SceneElement::Body(bi)));
                }
            }
            Tool::Slice => {
                if let Some(cs) = self.state.creating_slice.as_ref() {
                    folded.extend(cs.targets.iter().map(|&bi| SceneElement::Body(bi)));
                    // Construction-plane cutters map to a scene element; face cutters don't.
                    folded.extend(cs.cutters.iter().filter_map(|f| match f {
                        model::FaceId::ConstructionPlane(i) => {
                            Some(SceneElement::ConstructionPlane(*i))
                        }
                        _ => None,
                    }));
                }
            }
            Tool::Revolve => {
                if let Some(cr) = self.state.creating_revolve.as_ref() {
                    // Picked profile faces highlight like selected geometry (#303).
                    for face in &cr.faces {
                        folded.extend(extrude::extrude_face_scene_elements(face));
                    }
                    // Revolve's cut bodies are consumed destructively → red.
                    cut_highlight_bodies.extend(cr.cut_bodies.iter().copied());
                }
            }
            _ => {}
        }
        let render_selection = if folded.is_empty() {
            std::borrow::Cow::Borrowed(&self.state.scene_selection)
        } else {
            let mut sel = self.state.scene_selection.clone();
            for element in folded {
                sel.insert(element);
            }
            std::borrow::Cow::Owned(sel)
        };
        // Fade the descendants of the operation being edited (#260): the bodies downstream of the
        // edited op's outputs, so its live changes are visually de-emphasized.
        let faded_bodies = {
            let seeds = self.edited_operation_output_bodies();
            if seeds.is_empty() {
                Vec::new()
            } else {
                extrude::descendant_bodies(&self.state.doc, &seeds)
                    .into_iter()
                    .collect()
            }
        };
        // Move tool (#215): a translation arrow per world axis at the picked targets' centroid.
        let move_gizmos = if self.state.tool == Tool::Move {
            self.move_gizmo_arrows()
                .map(|(anchor, axes)| {
                    axes.iter()
                        .map(|&(axis, _, dir, translation)| {
                            let hovered = self.move_gizmo_drag.map(|d| d.axis) == Some(axis)
                                || (self.move_gizmo_drag.is_none()
                                    && pointer_screen.is_some_and(|pp| {
                                        construction::offset_gizmo_hit(
                                            pp,
                                            &project,
                                            anchor,
                                            dir,
                                            extrude_gizmo_display_offset(translation),
                                        )
                                    }));
                            gpu_viewport::ViewportExtrudeGizmo {
                                origin: anchor,
                                normal: dir,
                                offset: extrude_gizmo_display_offset(translation),
                                color: [col::X_AXIS, col::Y_AXIS, col::Z_AXIS][axis],
                                hovered,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Move rotation-ring gizmo (#216): a circle around the picked axis at the centroid.
        // A selected sketch text gets the same ring around its origin instead (#286).
        let move_rotation_gizmo = (self.state.tool == Tool::Move)
            .then(|| self.move_rotation_geom())
            .flatten()
            .map(|(center, axis, radius)| {
                let hovered = self.move_rotation_drag.is_some()
                    || pointer_screen.is_some_and(|pp| {
                        rotation_ring_hit(pp, &project, center, axis, radius)
                    });
                gpu_viewport::MoveRotationGizmo {
                    center,
                    axis,
                    radius,
                    color: col::PREVIEW,
                    hovered,
                }
            })
            .or_else(|| {
                (self.state.tool == Tool::Move)
                    .then(|| self.text_rotation_geom())
                    .flatten()
                    .map(|(_, center, axis, radius)| {
                        let hovered = self.text_rotation_drag.is_some()
                            || pointer_screen.is_some_and(|pp| {
                                rotation_ring_hit(pp, &project, center, axis, radius)
                            });
                        gpu_viewport::MoveRotationGizmo {
                            center,
                            axis,
                            radius,
                            color: col::PREVIEW,
                            hovered,
                        }
                    })
            });
        // Live ghost of the in-progress in-sketch repeat's duplicates (#232): dashed copies of
        // the picked lines/circles at every computed offset, so the result previews before commit.
        let sketch_repeat_ghost = self.sketch_repeat_ghost_segments();
        // Live-updated descendant geometry for the operation being edited (#260): recomputed from
        // a scratch doc so faded downstream bodies follow the gizmo drag in preview styling.
        let edit_preview_meshes = self.edit_preview_descendant_meshes();
        let scene_input = build_viewport_scene_input(
            doc,
            &cam,
            viewport,
            sketch_session,
            &self.state.element_visibility,
            &render_selection,
            &self.state.document_health,
            self.state.creating_rect.as_ref(),
            self.state.creating_line.as_ref(),
            self.state.creating_circle.as_ref(),
            self.state.creating_plane.as_ref(),
            self.state.creating_extrusion.as_ref(),
            self.state.creating_edge_treatment.as_ref(),
            self.state.creating_revolve.as_ref(),
            self.state.creating_loft.as_ref(),
            self.state.creating_repeat.as_ref(),
            self.pending_extrude_target.clone(),
            plane_gizmo,
            extrude_gizmo,
            vertex_treatment_gizmo,
            move_gizmos,
            move_rotation_gizmo,
            revolve_arc_gizmo,
            vertex_treatment_preview,
            hover_highlight,
            &gpu_dim_labels,
            planar_label_view,
            Some(&constraint_graphics),
            cut_highlight_bodies,
            faded_bodies,
            sketch_repeat_ghost,
            edit_preview_meshes,
        );
        let scene = gpu_viewport::ViewportScene::build(&scene_input);
        let gpu_drawn =
            self.gpu_viewport && gpu_viewport::paint(render_state, &painter, viewport, scene);

        if !gpu_drawn {
            painter.rect_filled(viewport, 0.0, col::BG);
            draw_ground(
                &painter,
                &project,
                viewport,
                sketch_session.is_some(),
            );

            let visibility = &self.state.element_visibility;
            let health = &self.state.document_health;
            for (li, line) in doc.lines.iter().enumerate() {
                if !line_alive(doc, li)
                    || !visibility.effective_visible(doc, SceneElement::Line(li))
                    || self.state.scene_selection.is_selected(SceneElement::Line(li))
                {
                    continue;
                }
                let dim = sketch_session.is_some_and(|s| line.sketch != s.sketch);
                let base = if line.projection.is_some() {
                    sketch_color(col::PROJECTION, dim)
                } else if line.construction {
                    sketch_color(col::CONSTRUCTION, dim)
                } else if crate::sketch_solver::sketch_fully_constrained_lines(&self.state.doc, line.sketch)
                    .is_ok_and(|set| set.contains(&li))
                {
                    sketch_color(col::RECT_LINE_CONSTRAINED, dim)
                } else {
                    sketch_color(col::RECT_LINE, dim)
                };
                let color = health_tint_color(base, health.element_status(SceneElement::Line(li)));
                if line.construction {
                    draw_construction_line_segment(&painter, &project, doc, line, color, 2.0);
                } else {
                    draw_line_segment(&painter, &project, doc, line, color, 2.0);
                }
            }
            for (ci, circle) in doc.circles.iter().enumerate() {
                if !circle_alive(doc, ci)
                    || !visibility.effective_visible(doc, SceneElement::Circle(ci))
                {
                    continue;
                }
                let dim = sketch_session
                    .is_some_and(|s| !sketch_circle_is_active(doc, s, ci, circle.sketch));
                let element_health = health.element_status(SceneElement::Circle(ci));
                draw_circle_edges(&painter, &project, doc, circle, dim, element_health);
            }
            for (i, plane) in doc.construction_planes.iter().enumerate() {
                if plane.deleted
                    || !visibility.effective_visible(doc, SceneElement::ConstructionPlane(i))
                {
                    continue;
                }
                let session_face =
                    sketch_session.and_then(|s| doc.sketch_face(s.sketch));
                let active = session_face == Some(FaceId::ConstructionPlane(i));
                let color = if active {
                    col::DIM_EDGE_HIGHLIGHT
                } else {
                    sketch_color(col::CONSTRUCTION, sketch_session.is_some())
                };
                draw_construction_plane(&painter, &project, plane, color, true);
            }
            draw_scene_selection_highlights(
                &painter,
                &project,
                doc,
                health,
                &self.state.scene_selection,
            );
            if let Some(session) = sketch_session {
                if let Some(face) = doc.sketch_face(session.sketch) {
                    if !matches!(face, FaceId::ConstructionPlane(_)) {
                        draw_face_highlight(
                            &painter,
                            &project,
                            doc,
                            face,
                            col::DIM_EDGE_HIGHLIGHT,
                        );
                    }
                }
            }
        }

        if !constraint_graphics.is_empty() {
            if !gpu_drawn {
                draw_constraint_connectors(
                    &painter,
                    &project,
                    &self.state.document_health,
                    &self.state.scene_selection,
                    &constraint_graphics,
                    col::DIM_EDGE_HIGHLIGHT,
                );
            }
            draw_constraint_icons(
                &painter,
                ui.ctx(),
                &project,
                &self.state.document_health,
                &self.state.scene_selection,
                &constraint_graphics,
                pointer_screen.and_then(|pp| {
                    pointer_over_constraint_icon(&constraint_icon_hits, pp)
                }),
                col::DIM_ANNOTATION,
                col::DIM_EDGE_HIGHLIGHT,
            );
        }

        if self.state.tool == Tool::Extrude {
            if let Some(ce) = self.state.creating_extrusion.as_ref() {
                draw_extrude_height_dimension(&painter, &project, doc, ce);
            }
            // Highlight the object the extrusion is currently snapping to.
            if let Some(target) = self.pending_extrude_target.clone() {
                draw_extrude_target_highlight(
                    &painter,
                    &project,
                    doc,
                    target,
                    col::DIM_EDGE_HIGHLIGHT,
                );
            }
        }

        // In-sketch Move gizmo (#306): a centre disc for free drags plus u/v push-pull arrows.
        if self.state.tool == Tool::Move && sketch_session.is_some() {
            if let Some((center_uv, frame)) = self.sketch_move_gizmo() {
                let active_axis = self.sketch_move_drag.map(|d| d.axis);
                let center_w = local_to_world(&frame, center_uv.0, center_uv.1);
                if let Some(cp) = project(center_w) {
                    let u_tip = local_to_world(&frame, center_uv.0 + SKETCH_MOVE_ARROW_MM, center_uv.1);
                    let v_tip = local_to_world(&frame, center_uv.0, center_uv.1 + SKETCH_MOVE_ARROW_MM);
                    // u arrow (horizontal) and v arrow (vertical), each highlighted while its
                    // axis drag is active or a free drag is running.
                    for (tip_w, this_axis, color) in [
                        (u_tip, Some(0usize), col::X_AXIS),
                        (v_tip, Some(1usize), col::Y_AXIS),
                    ] {
                        if let Some(tp) = project(tip_w) {
                            let hot = matches!(active_axis, Some(a) if a == this_axis || a.is_none());
                            let w = if hot { 3.0 } else { 2.0 };
                            painter.line_segment([cp, tp], egui::Stroke::new(w, color));
                            painter.circle_filled(tp, if hot { 5.0 } else { 4.0 }, color);
                        }
                    }
                    // Centre free-drag handle.
                    let center_hot = matches!(active_axis, Some(None));
                    painter.circle_filled(
                        cp,
                        if center_hot { 6.0 } else { 5.0 },
                        col::PREVIEW,
                    );
                }
            }
        }

        if let Some(active_session) = sketch_session {
            let active_sketch = active_session.sketch;
            let mut commit_committed_dim = false;
            if let (Some(layouts), Some(view)) = (&committed_dim_layouts, planar_label_view) {
                let hovered_angle_gizmo = pointer_screen
                    .and_then(|pp| {
                        angle_gizmo_hit_target(
                            layouts,
                            pp,
                            &project,
                            angle_gizmo_constraint,
                            viewport,
                        )
                    })
                    .or(self.angle_gizmo_drag.map(|d| d.constraint_id));
                if !gpu_drawn {
                    draw_committed_dim_layouts(
                        &painter,
                        layouts,
                        &view,
                        &project,
                        &self.state.document_health,
                        angle_gizmo_constraint,
                        hovered_angle_gizmo,
                        viewport,
                    );
                } else {
                    let arc_layouts: Vec<_> = layouts
                        .iter()
                        .filter(|layout| layout.arc_geom.is_some())
                        .cloned()
                        .collect();
                    if !arc_layouts.is_empty() {
                        draw_committed_dim_layouts(
                            &painter,
                            &arc_layouts,
                            &view,
                            &project,
                            &self.state.document_health,
                            angle_gizmo_constraint,
                            hovered_angle_gizmo,
                            viewport,
                        );
                    }
                }
                if let Some(edit) = &mut self.state.editing_committed_dim {
                    let is_angle = edit.target.is_angle(&self.state.doc);
                    let constraint_id = match &edit.target {
                        DimEditTarget::Constraint(id) => Some(*id),
                        DimEditTarget::New(_) => None,
                    };
                    let input_layout = if let Some(id) = constraint_id {
                        layouts
                            .iter()
                            .find(|l| l.target == id)
                            .map(|layout| {
                                dim_input_layout_centered_on(layout.label_rect, &edit.text)
                            })
                    } else if let Some(target) = edit.target.distance_target(&self.state.doc) {
                        distance_target_segment_endpoints(&self.state.doc, active_sketch, target)
                            .and_then(|(a, b)| {
                                project(a).zip(project(b)).map(|(pa, pb)| {
                                    line_dim_layout(pa, pb, &edit.text)
                                })
                            },
                        )
                    } else if let Some(model::DimensionTarget::Angle {
                        line_a,
                        line_b,
                        rotation_sign,
                    }) = edit.target.dimension_target(&self.state.doc)
                    {
                        // Place the input inside the angle (on the bisector), not on the vertex
                        // where it would overlap both lines.
                        sketch_session
                            .and_then(|s| sketch_geometry_frame(&self.state.doc, s.sketch))
                            .zip(angle_constraint_display(
                                &self.state.doc,
                                line_a,
                                line_b,
                                rotation_sign,
                            ))
                            .and_then(|(frame, display)| {
                                let radius_world = pixels_to_world_distance(
                                    &project,
                                    display.center,
                                    display.dir_a,
                                    effective_arc_dim_offset(None),
                                );
                                // Clear the gizmo ring/handle so it isn't hidden behind
                                // the editable input box (#40).
                                let label_outset_world = pixels_to_world_distance(
                                    &project,
                                    display.center,
                                    display.dir_a,
                                    ANGLE_DIM_INPUT_GIZMO_CLEARANCE_PX,
                                );
                                arc_dimension_world_geom(
                                    display.center,
                                    display.dir_a,
                                    display.dir_b,
                                    frame.normal,
                                    radius_world,
                                    label_outset_world,
                                )
                                .and_then(|wg| project(wg.label_center))
                                .map(|pc| {
                                    dim_input_layout_centered_on(
                                        egui::Rect::from_center_size(
                                            pc,
                                            dim_input_size_for_text(&edit.text),
                                        ),
                                        &edit.text,
                                    )
                                })
                            })
                    } else {
                        None
                    };
                    if let Some(input_layout) = input_layout {
                        let ctx = ui.ctx();
                        let id = egui::Id::new(("committed_dim", format!("{:?}", edit.target)));
                        let mut commit_dim = false;
                        let mut dim_field_result = SketchDimFieldResult::default();
                        let doc = &mut self.state.doc;
                        egui::Area::new(egui::Id::new((
                            "committed_dim_area",
                            format!("{:?}", edit.target),
                        )))
                        .fixed_pos(input_layout.pos)
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            dim_field_result = show_sketch_dimension_field(
                                ui,
                                ctx,
                                id,
                                &mut edit.text,
                                doc,
                                Some(active_sketch),
                                true,
                                &mut edit.pending_focus,
                                true,
                                is_angle,
                            );
                            commit_dim = dim_field_result.enter_commit;
                        });
                        inline_parameter_field_results.push(dim_field_result);
                        let dim_focused = ctx.memory(|m| m.focused()) == Some(id);
                        if edit.pending_focus {
                            ctx.memory_mut(|m| m.request_focus(id));
                        }
                        commit_committed_dim = should_commit_sketch_on_enter(
                            commit_dim,
                            dim_focused,
                            sketch_dimension_enter_pressed(ui),
                        );
                        if commit_committed_dim && !commit_dim {
                            consume_sketch_dimension_enter(ui);
                        }
                    }
                    if let Some(target) = edit.target.distance_target(&self.state.doc) {
                        if let Some((a, b)) =
                            distance_target_segment_endpoints(&self.state.doc, active_sketch, target)
                        {
                            draw_world_segment(
                                &painter,
                                &project,
                                a,
                                b,
                                col::DIM_EDGE_HIGHLIGHT,
                                3.5,
                            );
                        }
                    }
                    if is_angle && matches!(&edit.target, DimEditTarget::New(_)) {
                        if let Some(frame) = sketch_session
                            .and_then(|s| sketch_geometry_frame(&self.state.doc, s.sketch))
                        {
                            if let Some(model::DimensionTarget::Angle {
                                line_a,
                                line_b,
                                rotation_sign,
                            }) = edit.target.dimension_target(&self.state.doc)
                            {
                                draw_angle_dim_for_lines(
                                    &painter,
                                    &project,
                                    &frame,
                                    &self.state.doc,
                                    line_a,
                                    line_b,
                                    rotation_sign,
                                    None,
                                    &edit.text,
                                    true,
                                    false,
                                );
                            }
                        }
                    }
                }
            }
            if commit_committed_dim {
                self.state.apply(Action::CommitCommittedDim);
            }
        } else {
            self.dim_label_drag = None;
            self.state.editing_committed_dim = None;
        }
        if let (Some(cr), Some(session)) =
            (&self.state.creating_rect, self.state.sketch_session)
        {
            if let Some(frame) = sketch_geometry_frame(&self.state.doc, session.sketch) {
                if !gpu_drawn {
                    let end = cr.end_point(&frame, &self.state.doc);
                    let (ou, ov) = world_to_local(&frame, cr.origin);
                    let (eu, ev) = world_to_local(&frame, end);
                    let corners = preview_rect_world_corners(&frame, ou, ov, eu, ev);
                    if cr.construction {
                        draw_rect_edges(
                            &painter,
                            &project,
                            corners,
                            true,
                            false,
                            HealthStatus::Healthy,
                        );
                    } else {
                        draw_rect(&painter, &project, corners, col::PREVIEW, false);
                    }
                }
                let anchor_color = if cr.construction {
                    col::CONSTRUCTION
                } else {
                    col::PREVIEW
                };
                if let Some(sp) = project(cr.origin) {
                    painter.circle_filled(sp, 3.5, anchor_color);
                }
            }
        }
        if let (Some(cl), Some(session)) =
            (&self.state.creating_line, self.state.sketch_session)
        {
            if let Some(frame) = sketch_geometry_frame(&self.state.doc, session.sketch) {
                if !gpu_drawn {
                    let end = cl.end_point(&frame, &self.state.doc);
                    let (u0, v0) = world_to_local(&frame, cl.origin);
                    let (u1, v1) = world_to_local(&frame, end);
                    let preview =
                        Line::from_local_endpoints(session.sketch, u0, v0, u1, v1);
                    if cl.construction {
                        draw_construction_line_segment(
                            &painter,
                            &project,
                            &self.state.doc,
                            &preview,
                            col::CONSTRUCTION,
                            2.0,
                        );
                    } else if let (Some(pa), Some(pb)) = (project(cl.origin), project(end)) {
                        painter.line_segment([pa, pb], egui::Stroke::new(2.0, col::PREVIEW));
                    }
                }
                let anchor_color = if cl.construction {
                    col::CONSTRUCTION
                } else {
                    col::PREVIEW
                };
                if let Some(sp) = project(cl.origin) {
                    painter.circle_filled(sp, 3.5, anchor_color);
                }
            }
        }
        if let (Some(cc), Some(session)) =
            (&self.state.creating_circle, self.state.sketch_session)
        {
            if let Some(frame) = sketch_geometry_frame(&self.state.doc, session.sketch) {
                if !gpu_drawn {
                    let (cu, cv) = world_to_local(&frame, cc.origin);
                    let r = cc.radius(&frame, &self.state.doc);
                    let angle = cc.diameter_dim_angle(&frame);
                    let preview = Circle::from_local_center_radius(
                        session.sketch,
                        cu,
                        cv,
                        r,
                        angle,
                    );
                    if cc.construction {
                        draw_circle_edges(
                            &painter,
                            &project,
                            &self.state.doc,
                            &preview,
                            false,
                            HealthStatus::Healthy,
                        );
                    } else {
                        draw_circle(
                            &painter,
                            &project,
                            &self.state.doc,
                            &preview,
                            col::PREVIEW,
                            false,
                            1.5,
                        );
                    }
                }
                let anchor_color = if cc.construction {
                    col::CONSTRUCTION
                } else {
                    col::PREVIEW
                };
                if let Some(sp) = project(cc.origin) {
                    painter.circle_filled(sp, 3.5, anchor_color);
                }
            }
        }
        if let Some(cp) = &self.state.creating_plane {
            if !gpu_drawn {
                let preview = cp.preview_plane();
                draw_construction_plane(&painter, &project, &preview, col::PREVIEW, false);
                if let Some(edit_index) = cp.edit_index {
                    if let Some(dependent) =
                        preview_plane_edit_dependents(&self.state.doc, edit_index, &preview)
                    {
                        for (_, plane) in &dependent.planes {
                            draw_construction_plane(
                                &painter,
                                &project,
                                plane,
                                col::PREVIEW,
                                false,
                            );
                        }
                        for &(a, b) in &dependent.lines {
                            draw_world_segment(&painter, &project, a, b, col::PREVIEW, 2.0);
                        }
                    }
                }
            }
            if !gpu_drawn {
                let gizmo_hover = plane_gizmo_hover(cp, pointer_screen, &project);
                match &cp.reference {
                    PlaneReference::Face { origin, normal, .. } => {
                        draw_offset_gizmo(
                            &painter,
                            &project,
                            *origin,
                            *normal,
                            cp.offset_live,
                            col::PREVIEW,
                            gizmo_hover == Some(AxisGizmoHit::Offset),
                        );
                    }
                    PlaneReference::Axis {
                        origin,
                        direction,
                        ..
                    } => {
                        draw_axis_plane_gizmo(
                            &painter,
                            &project,
                            *origin,
                            *direction,
                            cp.offset_live,
                            cp.axis_angle_deg,
                            col::PREVIEW,
                            gizmo_hover,
                        );
                    }
                }
            }
        }

        if !gpu_drawn
            && self.state.tool == Tool::ConstructionPlane
            && self.state.creating_plane.is_none()
            && !suppress_hover_highlight
        {
            if let Some(pp) = response.hover_pos().or(response.interact_pointer_pos()) {
                let gp = cam.ground_point(pp, viewport, &vp);
                if let Some(target) = resolve_pick_target(pp, &project, gp, &self.state.doc, pick_occlusion) {
                    target.draw_highlight(&painter, &project, &self.state.doc);
                }
            }
        }

        if let (Some(cr), Some(session)) =
            (&mut self.state.creating_rect, self.state.sketch_session)
        {
            let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
            let end = cr.end_point(&frame, &self.state.doc);
            let (ou, ov) = world_to_local(&frame, cr.origin);
            let (eu, ev) = world_to_local(&frame, end);
            let corners = preview_rect_world_corners(&frame, ou, ov, eu, ev);
            if let Some((width_layout, height_layout)) = rectangle_dim_layout_from_corners(
                &project,
                corners,
                &cr.texts[0],
                &cr.texts[1],
            ) {
                let ctx = ui.ctx();
                let id_w = egui::Id::new("cr_width");
                let id_h = egui::Id::new("cr_height");

                let mut commit_rect = false;
                let mut width_field_result = SketchDimFieldResult::default();
                let mut height_field_result = SketchDimFieldResult::default();
                let doc = &mut self.state.doc;
                egui::Area::new(egui::Id::new("cr_width_area"))
                    .fixed_pos(width_layout.pos)
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        width_field_result = show_sketch_dimension_field(
                            ui,
                            ctx,
                            id_w,
                            &mut cr.texts[0],
                            doc,
                            Some(session.sketch),
                            cr.focused == 0,
                            &mut cr.pending_focus,
                            cr.user_edited[0],
                            false,
                        );
                        if width_field_result.changed {
                            cr.user_edited[0] = true;
                        }
                        if width_field_result.enter_commit {
                            commit_rect = true;
                        }
                    });
                inline_parameter_field_results.push(width_field_result);

                let doc = &mut self.state.doc;
                egui::Area::new(egui::Id::new("cr_height_area"))
                    .fixed_pos(height_layout.pos)
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        height_field_result = show_sketch_dimension_field(
                            ui,
                            ctx,
                            id_h,
                            &mut cr.texts[1],
                            doc,
                            Some(session.sketch),
                            cr.focused == 1,
                            &mut cr.pending_focus,
                            cr.user_edited[1],
                            false,
                        );
                        if height_field_result.changed {
                            cr.user_edited[1] = true;
                        }
                        if height_field_result.enter_commit {
                            commit_rect = true;
                        }
                    });
                inline_parameter_field_results.push(height_field_result);

                let current = ctx.memory(|m| m.focused());
                if current == Some(id_w) {
                    cr.focused = 0;
                } else if current == Some(id_h) {
                    cr.focused = 1;
                } else if cr.pending_focus {
                    let target_id = if cr.focused == 0 { id_w } else { id_h };
                    ctx.memory_mut(|m| m.request_focus(target_id));
                }

                let dim_field_focused =
                    current == Some(id_w) || current == Some(id_h);
                if should_commit_sketch_on_enter(
                    commit_rect,
                    dim_field_focused,
                    sketch_dimension_enter_pressed(ui),
                ) {
                    if !commit_rect {
                        consume_sketch_dimension_enter(ui);
                    }
                    self.state.apply(Action::CommitRectangle);
                }

                if let Some(edge) = current
                    .and_then(|id| {
                        if id == id_w {
                            rect_dim_edge_for_focus(0)
                        } else if id == id_h {
                            rect_dim_edge_for_focus(1)
                        } else {
                            None
                        }
                    })
                {
                    let (a, b) = rect_highlight_edge(corners, edge);
                    draw_world_segment(
                        &painter,
                        &project,
                        a,
                        b,
                        col::DIM_EDGE_HIGHLIGHT,
                        3.5,
                    );
                }
            }
        }

        if let (Some(cl), Some(session)) =
            (&mut self.state.creating_line, self.state.sketch_session)
        {
            let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
            let end = cl.end_point(&frame, &self.state.doc);
            if let (Some(pa), Some(pb)) = (project(cl.origin), project(end)) {
                let layout = line_dim_layout(pa, pb, &cl.text);
                let id_len = egui::Id::new("cl_length");

                let mut commit_line = false;
                let mut line_field_result = SketchDimFieldResult::default();
                {
                    let ctx = ui.ctx();
                    let doc = &mut self.state.doc;
                    egui::Area::new(egui::Id::new("cl_length_area"))
                        .fixed_pos(layout.pos)
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            line_field_result = show_sketch_dimension_field(
                                ui,
                                ctx,
                                id_len,
                                &mut cl.text,
                                doc,
                                Some(session.sketch),
                                true,
                                &mut cl.pending_focus,
                                cl.user_edited,
                                false,
                            );
                            if line_field_result.changed {
                                cl.user_edited = true;
                            }
                            commit_line = line_field_result.enter_commit;
                        });
                }
                inline_parameter_field_results.push(line_field_result);

                let length_focused = {
                    let ctx = ui.ctx();
                    let focused = ctx.memory(|m| m.focused()) == Some(id_len);
                    if !focused && cl.pending_focus {
                        ctx.memory_mut(|m| m.request_focus(id_len));
                    }
                    focused
                };
                let commit_line_now = should_commit_sketch_on_enter(
                    commit_line,
                    length_focused,
                    sketch_dimension_enter_pressed(ui),
                );
                if commit_line_now {
                    if !commit_line {
                        consume_sketch_dimension_enter(ui);
                    }
                    self.state.apply(Action::CommitLine);
                } else if length_focused {
                    draw_world_segment(
                        &painter,
                        &project,
                        cl.origin,
                        end,
                        col::DIM_EDGE_HIGHLIGHT,
                        3.5,
                    );
                }
            }
        }

        if let (Some(cc), Some(session)) =
            (&mut self.state.creating_circle, self.state.sketch_session)
        {
            let frame = sketch_geometry_frame(&self.state.doc, session.sketch).unwrap();
            let (cu, cv) = world_to_local(&frame, cc.origin);
            let preview = Circle::from_local_center_radius(
                session.sketch,
                cu,
                cv,
                cc.radius(&frame, &self.state.doc),
                cc.diameter_dim_angle(&frame),
            );
            if let Some((a, b)) = circle_world_diameter_endpoints(&self.state.doc, &preview) {
                if let (Some(pa), Some(pb)) = (project(a), project(b)) {
                    let layout = line_dim_layout(pa, pb, &cc.text);
                    let id_diam = egui::Id::new("cc_diameter");

                    let mut commit_circle = false;
                    let mut circle_field_result = SketchDimFieldResult::default();
                    {
                        let ctx = ui.ctx();
                        let doc = &mut self.state.doc;
                        egui::Area::new(egui::Id::new("cc_diameter_area"))
                            .fixed_pos(layout.pos)
                            .order(egui::Order::Foreground)
                            .show(ctx, |ui| {
                                circle_field_result = show_sketch_dimension_field(
                                    ui,
                                    ctx,
                                    id_diam,
                                    &mut cc.text,
                                    doc,
                                    Some(session.sketch),
                                    true,
                                    &mut cc.pending_focus,
                                    cc.user_edited,
                                    false,
                                );
                                if circle_field_result.changed {
                                    cc.user_edited = true;
                                }
                                commit_circle = circle_field_result.enter_commit;
                            });
                    }
                    inline_parameter_field_results.push(circle_field_result);

                    let diameter_focused = {
                        let ctx = ui.ctx();
                        let focused = ctx.memory(|m| m.focused()) == Some(id_diam);
                        if !focused && cc.pending_focus {
                            ctx.memory_mut(|m| m.request_focus(id_diam));
                        }
                        focused
                    };
                    let commit_circle_now = should_commit_sketch_on_enter(
                        commit_circle,
                        diameter_focused,
                        sketch_dimension_enter_pressed(ui),
                    );
                    if commit_circle_now {
                        if !commit_circle {
                            consume_sketch_dimension_enter(ui);
                        }
                        self.state.apply(Action::CommitCircle);
                    } else if diameter_focused {
                        draw_world_segment(
                            &painter,
                            &project,
                            a,
                            b,
                            col::DIM_EDGE_HIGHLIGHT,
                            3.5,
                        );
                    }
                }
            }
        }

        if let Some(cp) = &mut self.state.creating_plane {
            let preview = cp.preview_plane();
            if let Some((offset_layout, angle_layout)) = plane_dim_layouts(
                &project,
                &preview,
                &cp.reference,
                cp.offset_live,
                cp.axis_angle_deg,
            )
            {
                let ctx = ui.ctx();
                let id_offset = egui::Id::new("cp_offset");
                let id_angle = egui::Id::new("cp_angle");

                let mut commit_plane = false;
                let mut offset_field_result = SketchDimFieldResult::default();
                let doc = &mut self.state.doc;
                egui::Area::new(egui::Id::new("cp_offset_area"))
                    .fixed_pos(offset_layout.pos)
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        offset_field_result = show_sketch_dimension_field(
                            ui,
                            ctx,
                            id_offset,
                            &mut cp.offset_text,
                            doc,
                            None,
                            cp.focused == PlaneDim::Offset,
                            &mut cp.pending_focus,
                            cp.user_edited_offset,
                            false,
                        );
                        if offset_field_result.changed {
                            cp.user_edited_offset = true;
                        }
                        if offset_field_result.enter_commit {
                            commit_plane = true;
                        }
                    });
                inline_parameter_field_results.push(offset_field_result);

                if let Some(angle_layout) = angle_layout {
                    let doc = &mut self.state.doc;
                    let mut angle_field_result = SketchDimFieldResult::default();
                    egui::Area::new(egui::Id::new("cp_angle_area"))
                        .fixed_pos(angle_layout.pos)
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            angle_field_result = show_sketch_dimension_field(
                                ui,
                                ctx,
                                id_angle,
                                &mut cp.angle_text,
                                doc,
                                None,
                                cp.focused == PlaneDim::Angle,
                                &mut cp.pending_focus,
                                cp.user_edited_angle,
                                true,
                            );
                            if angle_field_result.changed {
                                cp.user_edited_angle = true;
                            }
                            if angle_field_result.enter_commit {
                                commit_plane = true;
                            }
                        });
                    inline_parameter_field_results.push(angle_field_result);
                }

                let current = ctx.memory(|m| m.focused());
                if current == Some(id_offset) {
                    cp.focused = PlaneDim::Offset;
                } else if current == Some(id_angle) {
                    cp.focused = PlaneDim::Angle;
                } else if cp.pending_focus {
                    let target_id = if cp.focused == PlaneDim::Offset {
                        id_offset
                    } else {
                        id_angle
                    };
                    ctx.memory_mut(|m| m.request_focus(target_id));
                }

                let dim_field_focused =
                    current == Some(id_offset) || current == Some(id_angle);
                if should_commit_sketch_on_enter(
                    commit_plane,
                    dim_field_focused,
                    sketch_dimension_enter_pressed(ui),
                ) {
                    if !commit_plane {
                        consume_sketch_dimension_enter(ui);
                    }
                    self.state.apply(Action::CommitConstructionPlane);
                }

                if !gpu_drawn {
                    draw_construction_plane(
                        &painter,
                        &project,
                        &preview,
                        col::DIM_EDGE_HIGHLIGHT,
                        false,
                    );
                }
            }
        }

        let shift_held = ui.input(|i| i.modifiers.shift);
        if camera::Camera::shows_camera_pivot(
            response.dragged_by(egui::PointerButton::Secondary),
            shift_held,
        ) {
            draw_orbit_pivot_indicator(&painter, &project, cam.target);
        }

        if matches!(self.state.tool, Tool::Select | Tool::Constraint) {
            let mut create_parameter_from_line = None;
            crate::parameters::show_computed_line_length_context_menu(
                &response,
                &self.state.doc,
                &self.state.scene_selection,
                &mut |line_index| create_parameter_from_line = Some(line_index),
            );
            if let Some(line_index) = create_parameter_from_line {
                self.state.apply(Action::CreateParameterFromLineLength {
                    line_index,
                    name: None,
                });
            }
        }

        // Snap indicator: a ring where a dragged/drawn point has latched onto geometry, or
        // where the first point of a line would land if clicked now.
        if let Some(session) = self.state.sketch_session {
            if let Some(frame) = sketch_geometry_frame(&self.state.doc, session.sketch) {
                let snap = active_snap(&self.state, session.sketch, &frame).or_else(|| {
                    // Preview where the next click would place a point (the first point of a
                    // line/rectangle, or a circle center), before any geometry exists.
                    let drawing = matches!(
                        self.state.tool,
                        Tool::Line | Tool::Rectangle | Tool::Circle
                    );
                    let mid_op = self.state.creating_line.is_some()
                        || self.state.creating_rect.is_some()
                        || self.state.creating_circle.is_some();
                    if !drawing || mid_op || self.vertex_drag.is_some() || !self.state.snapping_enabled
                    {
                        return None;
                    }
                    let pp = pointer_screen?;
                    let gp =
                        sketch_plane_point(&cam, viewport, &vp, &self.state.doc, session, pp)?;
                    let (sgp, target) =
                        snap_ground_point(&self.state, session, &frame, &project, gp, &[]);
                    target.map(|t| (sgp, t))
                });
                if let Some((world, target)) = snap {
                    if let Some(sp) = project(world) {
                        let color = egui::Color32::from_rgb(120, 215, 230);
                        // Inference guide (#21): a dashed line from the anchor edge through the
                        // snapped point, showing the extension the point is aligned with.
                        if let snapping::SnapTarget::OnLineExtension(line) = &target {
                            if let Ok(((x0, y0), (x1, y1))) = geometric_constraints::line_uv_endpoints(
                                &self.state.doc,
                                session.sketch,
                                line.clone(),
                            ) {
                                let (su, sv) = world_to_local(&frame, world);
                                let d0 = (x0 - su).hypot(y0 - sv);
                                let d1 = (x1 - su).hypot(y1 - sv);
                                let (au, av) = if d0 <= d1 { (x0, y0) } else { (x1, y1) };
                                let anchor_world = face::local_to_world(&frame, au, av);
                                if let Some(ap) = project(anchor_world) {
                                    painter.extend(egui::Shape::dashed_line(
                                        &[ap, sp],
                                        egui::Stroke::new(1.5, color),
                                        6.0,
                                        4.0,
                                    ));
                                }
                            }
                        }
                        // Normal-at-midpoint guide (#41): a dashed line from the anchor's
                        // midpoint through the snapped point, showing the perpendicular the
                        // point is aligned with.
                        if let snapping::SnapTarget::NormalAtMidpoint(line) = &target {
                            if let Ok(((x0, y0), (x1, y1))) = geometric_constraints::line_uv_endpoints(
                                &self.state.doc,
                                session.sketch,
                                line.clone(),
                            ) {
                                let mid = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
                                let mid_world = face::local_to_world(&frame, mid.0, mid.1);
                                if let Some(mp) = project(mid_world) {
                                    painter.extend(egui::Shape::dashed_line(
                                        &[mp, sp],
                                        egui::Stroke::new(1.5, color),
                                        6.0,
                                        4.0,
                                    ));
                                }
                            }
                        }
                        painter.circle_stroke(sp, 7.0, egui::Stroke::new(2.0, color));
                        // Emphasize the actual vertex being snapped to.
                        if matches!(target, snapping::SnapTarget::Vertex(_)) {
                            painter.circle_filled(sp, 3.5, color);
                        }
                        // Show the constraint a click would add (coincident, midpoint, …).
                        let icon_rect = egui::Rect::from_min_size(
                            sp + egui::vec2(9.0, -19.0),
                            egui::vec2(16.0, 16.0),
                        );
                        icons::paint_icon(&painter, ui.ctx(), snap_icon(target), icon_rect, color);
                    }
                }
            }
        }

        // Hide the view-cube HUD while a viewport screenshot is being captured this frame.
        let suppress_hud_for_screenshot = self
            .script
            .as_ref()
            .is_some_and(|runner| runner.screenshot_suppresses_hud());
        if self.state.panes.is_visible(Pane::ViewCube) && !suppress_hud_for_screenshot {
            let command_log = self
                .state
                .command_log
                .as_ref()
                .map(|log| log.borrow_mut());
            view_cube::show_hud(
                ui.ctx(),
                &mut self.state.cam,
                viewport,
                render_state,
                self.gpu_view_cube,
                command_log,
            );
        }

        let hint = match self.state.tool {
            Tool::Select => {
                if self.state.editing_committed_dim.is_some() {
                    "Edit dimension • Enter to commit • Esc to cancel"
                } else if self.state.sketch_session.is_some() {
                    "Sketch mode — drag vertices • Shift+click or ⌘/Ctrl+click multi-select • double-click a dimension to edit • Esc: exit sketch"
                } else {
                    "Click to select • Shift+click or ⌘/Ctrl+click multi-select • Right-drag: orbit  •  Wheel: zoom  •  s: sketch  •  p: plane"
                }
            }
            Tool::Sketch => {
                "s: sketch  •  Click a rectangle or construction plane face  •  Esc: cancel"
            }
            Tool::Loft => {
                if self
                    .state
                    .creating_loft
                    .as_ref()
                    .is_some_and(|cl| cl.sections.len() >= 2)
                {
                    "Loft — click more profiles to add sections • Enter: create loft • Esc: cancel"
                } else {
                    "Loft — click two or more closed profiles (circles or loops) • Enter: create loft • Esc: cancel"
                }
            }
            Tool::Revolve => {
                let cr = self.state.creating_revolve.as_ref();
                if cr.is_some_and(|c| !c.faces.is_empty() && c.axis.is_some()) {
                    "Revolve — drag the handle or type an angle • Enter: commit • Esc: cancel"
                } else if cr.is_some_and(|c| !c.faces.is_empty()) {
                    "Revolve — click a line (or global axis) to revolve around"
                } else {
                    "Revolve — click one or more coplanar profile faces"
                }
            }
            Tool::Combine => {
                let cb = self.state.creating_boolean.as_ref();
                if cb.is_some_and(|c| !c.a.is_empty()) {
                    "Combine — click bodies to add/remove • Enter: commit • Esc: cancel"
                } else {
                    "Combine — click one or more bodies to operate on"
                }
            }
            Tool::Move => {
                let cm = self.state.creating_move.as_ref();
                if self.state.sketch_session.is_some() {
                    // In-sketch selection gizmo (#306).
                    "Move — drag the centre handle to move the selection, or the arrows to move along an axis"
                } else if cm.is_some_and(|c| !c.targets.is_empty()) {
                    "Move — click bodies to add/remove • set offset/rotation in the context pane • Enter: commit"
                } else {
                    "Move — click one or more bodies to move"
                }
            }
            Tool::Repeat => {
                let cr = self.state.creating_repeat.as_ref();
                if cr.is_some_and(|c| !c.targets.is_empty()) {
                    "Repeat — set axis/mode/spacing in the context pane • Enter: commit"
                } else {
                    "Repeat — click one or more bodies to repeat"
                }
            }
            Tool::Slice => {
                let cs = self.state.creating_slice.as_ref();
                if cs.is_some_and(|c| !c.targets.is_empty()) {
                    "Slice — pick cutting planes/faces in the Cutters picker • Enter: commit • Esc: cancel"
                } else {
                    "Slice — click one or more bodies to slice"
                }
            }
            Tool::Text => {
                if self.state.editing_drawing.is_some() {
                    "Text — click on the page for a growing box, or drag to make a wrapped box"
                } else if self.state.sketch_session.is_some() {
                    "Text — click in the sketch to place text • edit it in the context pane"
                } else {
                    "Text — click a face or plane to sketch text on"
                }
            }
            Tool::DrawingAdd => {
                "Add view — click a body or sketch in the Elements pane, then drag it into place"
            }
            Tool::DrawingAlign => {
                "Aligned view — click a projection, then move the mouse and click to place a lined-up child view"
            }
            Tool::Rectangle => {
                if self.state.creating_rect.is_some() {
                    "Move mouse (free dim) • Type in focused input to constrain • Tab: switch dims • Click/Enter: create rect • Esc: cancel"
                } else if self.state.sketch_session.is_none() {
                    "r: rectangle  •  Click a face to sketch on  •  Right-drag: orbit  •  Shift-right or middle-drag: pan  •  Wheel: zoom"
                } else {
                    "r: rectangle  •  Left-click to set corner • move to size • Right-drag: orbit  • Shift-right or middle-drag: pan  •  Wheel: zoom"
                }
            }
            Tool::Line => {
                if self.state.creating_line.is_some() {
                    "Move mouse (free length) • Type in length input to constrain • Click/Enter: create line • Esc: cancel"
                } else if self.state.sketch_session.is_none() {
                    "l: line  •  Click a face to sketch on  •  Right-drag: orbit  • Shift-right or middle-drag: pan  •  Wheel: zoom"
                } else {
                    "l: line  •  Left-click to set start • move to aim • Right-drag: orbit  • Shift-right or middle-drag: pan  •  Wheel: zoom"
                }
            }
            Tool::Circle => {
                if self.state.creating_circle.is_some() {
                    "Move mouse (free diameter) • Type in diameter input to constrain • Click/Enter: create circle • Esc: cancel"
                } else if self.state.sketch_session.is_none() {
                    "o: circle  •  Click a face to sketch on  •  Right-drag: orbit  • Shift-right or middle-drag: pan  •  Wheel: zoom"
                } else {
                    "o: circle  •  Left-click to set center • move to size • Right-drag: orbit  • Shift-right or middle-drag: pan  •  Wheel: zoom"
                }
            }
            Tool::Constraint => {
                if self.state.sketch_session.is_none() {
                    "c: constraint  •  Open a sketch to add geometric constraints"
                } else {
                    "c: constraint  •  Shift+click or ⌘/Ctrl+click multi-select • 1–7 apply constraint • context pane shows options"
                }
            }
            Tool::Dimension => {
                if self.state.editing_committed_dim.is_some() {
                    "Edit dimension • Enter to commit • Esc to cancel"
                } else if self.state.sketch_session.is_none() {
                    "d: dimension  •  Open a sketch to add distance constraints"
                } else {
                    "d: dimension  •  Select geometry, press D, or click a segment • Enter commit"
                }
            }
            Tool::ConstructionPlane => {
                if self.state.creating_plane.is_some() {
                    let editing = self
                        .state
                        .creating_plane
                        .as_ref()
                        .and_then(|cp| cp.edit_index)
                        .is_some();
                    if self
                        .state
                        .creating_plane
                        .as_ref()
                        .is_some_and(|cp| cp.reference.is_axis())
                    {
                        if editing {
                            "Edit plane • drag arrow/circle or type to lock • Tab: switch dims • Click/Enter: commit • Esc: cancel"
                        } else {
                            "Drag arrow for offset • drag circle handle for angle • type to lock • Tab: switch dims • Click/Enter: commit • Esc: cancel"
                        }
                    } else if editing {
                        "Edit plane • drag arrow or type to lock offset • Click/Enter: commit • Esc: cancel"
                    } else {
                        "Drag arrow for offset • wheel or type to lock • Click/Enter: create plane • Esc: cancel"
                    }
                } else {
                    "p: plane  •  Click a face, line, shape edge, global axis, or ground • then set offset (and angle for lines)"
                }
            }
            Tool::Extrude => {
                if self.state.creating_extrusion.is_some() {
                    "e: extrude  •  Click faces to toggle • drag the arrow or type a distance • Enter: commit • Esc: cancel"
                } else {
                    "e: extrude  •  Click a coplanar face (rectangle/circle) to start an extrusion"
                }
            }
            Tool::Chamfer => {
                if self.state.creating_vertex_treatment.is_some() {
                    "k: chamfer  •  Drag the arrow or type a distance • Click/Enter: commit • Esc: cancel"
                } else if self.state.sketch_session.is_none() {
                    "k: chamfer  •  Open a sketch to chamfer a vertex"
                } else {
                    "k: chamfer  •  Click a vertex where two lines meet"
                }
            }
            Tool::Fillet => {
                if self.state.creating_vertex_treatment.is_some() {
                    "f: fillet  •  Drag the arrow or type a radius • Click/Enter: commit • Esc: cancel"
                } else if self.state.sketch_session.is_none() {
                    "f: fillet  •  Open a sketch to fillet a vertex"
                } else {
                    "f: fillet  •  Click a vertex where two lines meet"
                }
            }
        };
        painter.text(
            viewport.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(150),
        );

        for result in inline_parameter_field_results {
            apply_dimension_field_feedback(&mut self.state, &result);
        }

        // #74: an obvious border while a sketch is open, so sketch mode is never mistaken for
        // ordinary 3D navigation at a glance.
        if self.state.sketch_session.is_some() {
            painter.rect_stroke(
                viewport,
                0.0,
                egui::Stroke::new(SKETCH_MODE_BORDER_WIDTH, col::SKETCH_MODE_BORDER),
                egui::StrokeKind::Middle,
            );
        }
    }
}

/// Which normalized rectangle edge corresponds to a dimension input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RectDimEdge {
    /// Horizontal edge at min Y (width).
    Width,
    /// Vertical edge at min X (height).
    Height,
}

fn rect_dim_edge_for_focus(focused: usize) -> Option<RectDimEdge> {
    match focused {
        0 => Some(RectDimEdge::Width),
        1 => Some(RectDimEdge::Height),
        _ => None,
    }
}

fn draw_world_segment(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    a: Vec3,
    b: Vec3,
    color: egui::Color32,
    width: f32,
) {
    if let (Some(pa), Some(pb)) = (project(a), project(b)) {
        painter.line_segment([pa, pb], egui::Stroke::new(width, color));
    }
}

fn draw_world_segment_dashed(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    a: Vec3,
    b: Vec3,
    color: egui::Color32,
    width: f32,
) {
    if let (Some(pa), Some(pb)) = (project(a), project(b)) {
        painter.add(egui::Shape::dashed_line(
            &[pa, pb],
            egui::Stroke::new(width, color),
            construction::CONSTRUCTION_DASH_LENGTH_PX,
            construction::CONSTRUCTION_DASH_GAP_PX,
        ));
    }
}

fn draw_world_polyline(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    points: &[Vec3],
    color: egui::Color32,
    width: f32,
) {
    for pair in points.windows(2) {
        draw_world_segment(painter, project, pair[0], pair[1], color, width);
    }
}

fn draw_world_polyline_dashed(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    points: &[Vec3],
    color: egui::Color32,
    width: f32,
) {
    for pair in points.windows(2) {
        draw_world_segment_dashed(painter, project, pair[0], pair[1], color, width);
    }
}

const ORBIT_PIVOT_RADIUS: f32 = 4.0;
const ORBIT_PIVOT_GROUND_RADIUS: f32 = 2.0;

fn draw_orbit_pivot_indicator(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    target: Vec3,
) {
    if camera::orbit_pivot_has_ground_drop(target) {
        let foot = camera::orbit_pivot_ground_foot(target);
        draw_world_segment_dashed(
            painter,
            project,
            target,
            foot,
            col::ORBIT_PIVOT_DROP,
            1.0,
        );
        if let Some(foot_sp) = project(foot) {
            painter.circle_filled(foot_sp, ORBIT_PIVOT_GROUND_RADIUS, col::ORBIT_PIVOT);
        }
    }
    if let Some(sp) = project(target) {
        painter.circle_filled(sp, ORBIT_PIVOT_RADIUS, col::ORBIT_PIVOT);
    }
}

fn draw_construction_line_segment(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    line: &Line,
    color: egui::Color32,
    width: f32,
) {
    let Some(points) = line_world_polyline(doc, line) else {
        return;
    };
    draw_world_polyline_dashed(painter, project, &points, color, width);
}

fn circle_screen_perimeter(
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    circle: &Circle,
) -> Option<Vec<egui::Pos2>> {
    let pts = circle_world_perimeter(doc, circle, 64)?;
    pts.iter().map(|p| project(*p)).collect()
}

fn draw_circle(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    circle: &Circle,
    color: egui::Color32,
    fill: bool,
    width: f32,
) {
    let Some(screen_pts) = circle_screen_perimeter(project, doc, circle) else {
        return;
    };
    if screen_pts.len() < 2 {
        return;
    }
    if fill {
        painter.add(egui::Shape::convex_polygon(
            screen_pts.clone(),
            color.gamma_multiply(0.25),
            egui::Stroke::new(width, color),
        ));
    } else {
        painter.add(egui::Shape::closed_line(
            screen_pts,
            egui::Stroke::new(width, color),
        ));
    }
}

fn draw_construction_circle(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    circle: &Circle,
    color: egui::Color32,
    width: f32,
) {
    let Some(pts) = circle_world_perimeter(doc, circle, 64) else {
        return;
    };
    for window in pts.windows(2) {
        draw_world_segment_dashed(painter, project, window[0], window[1], color, width);
    }
}

fn draw_circle_edges(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    circle: &Circle,
    dim: bool,
    health: HealthStatus,
) {
    let solid_color = health_tint_color(sketch_color(col::RECT_LINE, dim), health);
    let construction_color = health_tint_color(sketch_color(col::CONSTRUCTION, dim), health);
    if circle.construction {
        if let Some(screen_pts) = circle_screen_perimeter(project, doc, circle) {
            painter.add(egui::Shape::convex_polygon(
                screen_pts,
                construction_color.gamma_multiply(0.18),
                egui::Stroke::NONE,
            ));
        }
        draw_construction_circle(painter, project, doc, circle, construction_color, 1.5);
    } else {
        draw_circle(painter, project, doc, circle, solid_color, true, 1.5);
    }
}

/// World-space corners of a rectangle-tool drag preview (BL, BR, TR, TL) from its two
/// local placed corners, in the sketch frame.
fn preview_rect_world_corners(
    frame: &face::SketchFrame,
    ou: f32,
    ov: f32,
    eu: f32,
    ev: f32,
) -> [Vec3; 4] {
    let x = ou.min(eu);
    let y = ov.min(ev);
    let xr = ou.max(eu);
    let yt = ov.max(ev);
    [
        face::local_to_world(frame, x, y),
        face::local_to_world(frame, xr, y),
        face::local_to_world(frame, xr, yt),
        face::local_to_world(frame, x, yt),
    ]
}

/// Draw a rectangle preview's four edges (dashed when construction) in the egui-painter path.
fn draw_rect_edges(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: [Vec3; 4],
    construction: bool,
    dim: bool,
    health: HealthStatus,
) {
    let solid_color = health_tint_color(sketch_color(col::RECT_LINE, dim), health);
    let construction_color = health_tint_color(sketch_color(col::CONSTRUCTION, dim), health);
    if construction {
        if let Some(pts) = corners
            .iter()
            .map(|&c| project(c))
            .collect::<Option<Vec<egui::Pos2>>>()
        {
            painter.add(egui::Shape::convex_polygon(
                pts,
                construction_color.gamma_multiply(0.18),
                egui::Stroke::NONE,
            ));
        }
    } else {
        draw_world_quad(painter, project, corners, solid_color, true);
    }
    for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        if construction {
            draw_world_segment_dashed(painter, project, corners[i], corners[j], construction_color, 1.5);
        } else {
            draw_world_segment(painter, project, corners[i], corners[j], solid_color, 1.5);
        }
    }
}

fn draw_scene_selection_highlights(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    health: &DocumentHealth,
    selection: &crate::selection::SceneSelection,
) {
    if selection.is_empty() {
        return;
    }
    let base_color = col::DIM_EDGE_HIGHLIGHT;
    let width = 3.0;
    for element in selection.iter() {
        let color = health_tint_color(base_color, health.element_status(element.clone()));
        let dashed = context::selection_highlight_dashed(doc, element.clone()) == Some(true);
        match element {
            SceneElement::Line(index) => {
                if !line_alive(doc, index) {
                    continue;
                }
                if let Some(line) = doc.lines.get(index) {
                    if dashed {
                        draw_construction_line_segment(painter, project, doc, line, color, width);
                    } else {
                        draw_line_segment(painter, project, doc, line, color, width);
                    }
                }
            }
            SceneElement::Circle(index) => {
                if !circle_alive(doc, index) {
                    continue;
                }
                if let Some(circle) = doc.circles.get(index) {
                    if dashed {
                        draw_construction_circle(painter, project, doc, circle, color, width);
                    } else {
                        draw_circle(painter, project, doc, circle, color, false, width);
                    }
                }
            }
            SceneElement::Constraint(index) => {
                if !constraint_alive(doc, index) {
                    continue;
                }
                if let Some((a, b)) = constraint_segment_endpoints(doc, index) {
                    draw_world_segment(painter, project, a, b, color, width);
                }
            }
            SceneElement::Point(point) => {
                if let Some(world) = point_world_position(doc, point) {
                    if let Some(screen) = project(world) {
                        painter.circle_filled(screen, 6.0, color);
                    }
                }
            }
            _ => {}
        }
    }
}

fn draw_world_quad(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: [Vec3; 4],
    color: egui::Color32,
    fill: bool,
) {
    let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
    let Some(pts) = pts else { return };
    if fill {
        painter.add(egui::Shape::convex_polygon(
            pts.clone(),
            color.gamma_multiply(0.25),
            egui::Stroke::new(1.5, color),
        ));
    } else {
        painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.5, color)));
    }
}

fn draw_line_segment(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &model::Document,
    line: &Line,
    color: egui::Color32,
    width: f32,
) {
    let Some(points) = line_world_polyline(doc, line) else {
        return;
    };
    draw_world_polyline(painter, project, &points, color, width);
}

fn dim_layout_near_screen_point(
    anchor: egui::Pos2,
    outward: egui::Vec2,
    gap_from_anchor: f32,
) -> DimInputLayout {
    let dir = if outward.length_sq() > 1e-4 {
        outward.normalized()
    } else {
        egui::vec2(-1.0, -1.0).normalized()
    };
    let size = dim_input_max_size();
    let center_dist = gap_from_anchor + aabb_half_extent_along(dir, size);
    let center = anchor + dir * center_dist;
    layout_at(center - size * 0.5, size)
}

fn dim_layout_avoiding_handle(
    anchor: egui::Pos2,
    outward: egui::Vec2,
    handle_size: f32,
) -> DimInputLayout {
    let mut gap = AXIS_GIZMO_HANDLE_HIT_RADIUS_PX + 6.0;
    let obstacle =
        egui::Rect::from_center_size(anchor, egui::vec2(handle_size, handle_size));
    for _ in 0..DIM_REPULSION_ITERS {
        let layout = dim_layout_near_screen_point(anchor, outward, gap);
        if !layout.rect.intersects(obstacle) {
            return layout;
        }
        gap += 2.0;
    }
    dim_layout_near_screen_point(anchor, outward, gap)
}

fn plane_dim_layouts(
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    _plane: &ConstructionPlane,
    reference: &PlaneReference,
    offset_live: f32,
    axis_angle_deg: f32,
) -> Option<(DimInputLayout, Option<DimInputLayout>)> {
    match reference {
        PlaneReference::Face { origin, normal, .. } => {
            let face_screen = project(*origin)?;
            let offset_screen = project(offset_handle(*origin, *normal, offset_live))?;
            let arrow = offset_screen - face_screen;
            let beside_arrow = if arrow.length_sq() > 1.0 {
                egui::vec2(-arrow.y, arrow.x).normalized()
            } else {
                egui::vec2(-1.0, 0.0)
            };
            let offset_layout =
                dim_layout_avoiding_handle(offset_screen, beside_arrow, 20.0);
            Some((offset_layout, None))
        }
        PlaneReference::Axis {
            origin,
            direction,
            ..
        } => {
            let axis_screen = project(*origin)?;
            let offset_screen = project(axis_offset_handle(
                *origin,
                *direction,
                offset_live,
                axis_angle_deg,
            ))?;
            let arrow = offset_screen - axis_screen;
            let beside_arrow = if arrow.length_sq() > 1.0 {
                egui::vec2(-arrow.y, arrow.x).normalized()
            } else {
                egui::vec2(-1.0, 0.0)
            };
            let offset_layout =
                dim_layout_avoiding_handle(offset_screen, beside_arrow, 20.0);

            let angle_screen = project(axis_angle_handle(
                *origin,
                *direction,
                axis_angle_deg,
            ))?;
            let radial = angle_screen - axis_screen;
            let angle_layout = dim_layout_avoiding_handle(angle_screen, radial, 24.0);

            Some((offset_layout, Some(angle_layout)))
        }
    }
}

fn draw_construction_plane(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    plane: &ConstructionPlane,
    color: egui::Color32,
    fill: bool,
) {
    let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
    let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
    let Some(pts) = pts else { return };
    if fill {
        let plane_color = if color == col::CONSTRUCTION {
            construction::PLANE_FILL_RGBA
        } else {
            color
        };
        painter.add(egui::Shape::convex_polygon(
            pts,
            gpu_viewport::fill_color(
                plane_color,
                gpu_viewport::DEFAULT_CONSTRUCTION_PLANE_OPACITY,
            ),
            egui::Stroke::NONE,
        ));
    } else {
        painter.add(egui::Shape::closed_line(
            pts,
            egui::Stroke::new(2.0, color),
        ));
    }
}

fn draw_rect(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: [Vec3; 4],
    color: egui::Color32,
    fill: bool,
) {
    draw_world_quad(painter, project, corners, color, fill);
}

/// Liang–Barsky clip of a screen-space segment to an axis-aligned rectangle.
fn clip_segment_to_rect(a: egui::Pos2, b: egui::Pos2, rect: egui::Rect) -> Option<(egui::Pos2, egui::Pos2)> {
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let d = b - a;
    let edges = [
        (-d.x, a.x - rect.min.x),
        (d.x, rect.max.x - a.x),
        (-d.y, a.y - rect.min.y),
        (d.y, rect.max.y - a.y),
    ];
    for (p, q) in edges {
        if p.abs() < 1e-8 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            let r = q / p;
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            let r = q / p;
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    Some((a + d * t0, a + d * t1))
}

fn draw_clipped_world_segment(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    viewport: egui::Rect,
    a: Vec3,
    b: Vec3,
    color: egui::Color32,
    width: f32,
) {
    let (Some(pa), Some(pb)) = (project(a), project(b)) else {
        return;
    };
    let Some((ca, cb)) = clip_segment_to_rect(pa, pb, viewport) else {
        return;
    };
    painter.line_segment([ca, cb], egui::Stroke::new(width, color));
}

fn sketch_color(color: egui::Color32, dim: bool) -> egui::Color32 {
    if dim {
        color.gamma_multiply(col::SKETCH_DIMMED)
    } else {
        color
    }
}

fn sketch_circle_is_active(
    doc: &model::Document,
    session: SketchSession,
    circle_index: usize,
    circle_sketch: SketchId,
) -> bool {
    if circle_sketch == session.sketch {
        return true;
    }
    if let Some(FaceId::Circle(face_index)) = doc.sketch_face(session.sketch) {
        return circle_index == face_index;
    }
    false
}

fn draw_ground(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    viewport: egui::Rect,
    dim: bool,
) {
    let e = GRID_EXTENT;
    let line = |a: Vec3, b: Vec3, color: egui::Color32, w: f32| {
        draw_clipped_world_segment(painter, project, viewport, a, b, color, w);
    };

    let mut t = -e;
    while t <= e + 0.001 {
        let base = if t.abs() < 0.001 {
            col::GRID_AXIS
        } else {
            col::GRID
        };
        let color = gpu_viewport::sketch_ground_color(base, dim);
        line(Vec3::new(-e, t, 0.0), Vec3::new(e, t, 0.0), color, 1.0);
        line(Vec3::new(t, -e, 0.0), Vec3::new(t, e, 0.0), color, 1.0);
        t += GRID_STEP;
    }

    line(
        Vec3::ZERO,
        Vec3::new(e, 0.0, 0.0),
        gpu_viewport::sketch_ground_color(col::X_AXIS, dim),
        2.0,
    );
    line(
        Vec3::ZERO,
        Vec3::new(0.0, e, 0.0),
        gpu_viewport::sketch_ground_color(col::Y_AXIS, dim),
        2.0,
    );
    line(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, e),
        gpu_viewport::sketch_ground_color(col::Z_AXIS, dim),
        2.0,
    );
}

#[cfg(test)]
mod tests {
    use super::actions::CreatingRect;
    use super::{
        build_viewport_scene_input, clip_segment_to_rect, col, initial_launch_maximize_frames,
        native_options, revolve_arc_angle_from_cursor, revolve_arc_handle_world,
        script_finished_close_action, should_commit_sketch_on_click, should_select_all_rect_value,
        side_panel_resize_active, tick_launch_maximize, uses_deferred_launch_maximize,
        vertex_treatment_preview_points, ConstraintPoint, Line, MACOS_LAUNCH_MAXIMIZE_DELAY_FRAMES,
        GRID_EXTENT, ORBIT_PIVOT_GROUND_RADIUS, ORBIT_PIVOT_RADIUS,
    };
    use crate::face::SketchFrame;
    use eframe::egui::{self, Pos2, Rect, Vec2};
    use egui::Color32;
    use glam::Vec3;

    #[test]
    fn shape_edge_stroke_color_is_shared() {
        assert_eq!(col::RECT_LINE, Color32::from_rgb(120, 170, 240));
    }

    /// #262: the revolve arc handle sits at `angle` around the axis, and a cursor at that
    /// handle's screen position reads back the same angle (dragging around the arc sets it).
    #[test]
    fn revolve_arc_angle_round_trips_through_the_cursor() {
        let center = Vec3::ZERO;
        let axis = Vec3::Z;
        let zero_dir = Vec3::X;
        let radius = 10.0;
        // Identity-ish projection: world (x, y) → screen (x, y).
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let zero_world = center + zero_dir * radius;
        let sign_probe = revolve_arc_handle_world(center, axis, zero_dir, radius, 5.0);

        for target in [30.0_f32, 90.0, 200.0, 270.0, 359.0] {
            let handle = revolve_arc_handle_world(center, axis, zero_dir, radius, target);
            let cursor = project(handle).unwrap();
            let got =
                revolve_arc_angle_from_cursor(cursor, center, zero_world, sign_probe, &project)
                    .unwrap();
            assert!(
                (got - target).abs() < 0.5,
                "angle {target} round-trips, got {got}"
            );
        }
    }

    /// #218: the body tools resolve a whole body from any of its sub-elements — an edge, a
    /// vertex, or a face — so clicking anywhere on a body selects it. (Previously they matched
    /// `scene_element_from_pick(..) == Body`, which is unreachable, so no body could be picked.)
    #[test]
    fn body_index_from_pick_reads_every_body_sub_element() {
        use crate::construction::PickTargetKind;
        use super::body_index_from_pick;
        assert_eq!(
            body_index_from_pick(&PickTargetKind::BodyVertex { body: 3, position: Vec3::ZERO }),
            Some(3)
        );
        assert_eq!(
            body_index_from_pick(&PickTargetKind::BodyEdge { body: 5, a: Vec3::ZERO, b: Vec3::X }),
            Some(5)
        );
        assert_eq!(
            body_index_from_pick(&PickTargetKind::BodyFace {
                body: 2,
                triangles: vec![],
                normal: Vec3::Z,
            }),
            Some(2)
        );
        assert_eq!(body_index_from_pick(&PickTargetKind::Line(0)), None);
        assert_eq!(body_index_from_pick(&PickTargetKind::ConstructionPlane(0)), None);
    }

    fn test_viewport_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0))
    }

    #[test]
    fn next_rect_focus_axis_toggles_width_and_height() {
        use super::{next_rect_focus_axis, RectAxis};
        assert_eq!(next_rect_focus_axis(0), RectAxis::Height);
        assert_eq!(next_rect_focus_axis(1), RectAxis::Width);
    }

    /// #125: an uncaught script error under `--exit` must close (so CI doesn't hang) and
    /// fail the process (so CI doesn't silently pass a broken script).
    #[test]
    fn script_error_with_exit_flag_closes_and_fails_the_process() {
        let action = script_finished_close_action(true, false, true);
        assert!(action.close, "must close so the process isn't left hanging");
        assert!(action.fail_process, "must fail so CI catches the broken script");
    }

    /// Without `--exit`, an error leaves the app open for interactive inspection — only the
    /// `--timeout` watchdog (a separate mechanism) should ever force it closed.
    #[test]
    fn script_error_without_exit_flag_stays_open() {
        let action = script_finished_close_action(true, false, false);
        assert!(!action.close);
        assert!(!action.fail_process);
    }

    /// `bearcad.quit()` always closes the window, whether or not `--exit` was passed — but
    /// a clean quit (no error) must never fail the process.
    #[test]
    fn explicit_quit_closes_without_failing_the_process() {
        for exit_flag in [false, true] {
            let action = script_finished_close_action(false, true, exit_flag);
            assert!(action.close);
            assert!(!action.fail_process);
        }
    }

    /// A script that finishes cleanly only closes if `--exit` was requested; either way it
    /// never fails the process.
    #[test]
    fn clean_completion_closes_only_when_exit_flag_is_set() {
        let stays_open = script_finished_close_action(false, false, false);
        assert!(!stays_open.close);
        assert!(!stays_open.fail_process);

        let closes = script_finished_close_action(false, false, true);
        assert!(closes.close);
        assert!(!closes.fail_process);
    }

    #[test]
    fn extrude_preview_uses_pending_target_before_commit() {
        // While dragging the gizmo, the snapped target lives in `pending_extrude_target`
        // (only copied onto `creating_extrusion.target` at commit time) — the ghost preview
        // must still pick it up live so it shows the real (e.g. slanted) shape while
        // dragging, not just after release (#63).
        use crate::actions::{Action, AppState, Tool};
        use crate::model::{ExtrudeFace, ExtrudeTarget};

        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
        });
        let ce = state.creating_extrusion.as_ref().unwrap();
        assert_eq!(ce.target, None, "target isn't committed onto ce yet");

        let cam = state.cam.clone();
        let element_visibility = state.element_visibility.clone();
        let selection = state.scene_selection.clone();
        let health = state.document_health.clone();
        let pending = Some(ExtrudeTarget::Plane(0));

        let scene_input = build_viewport_scene_input(
            &state.doc,
            &cam,
            test_viewport_rect(),
            None,
            &element_visibility,
            &selection,
            &health,
            None,
            None,
            None,
            None,
            state.creating_extrusion.as_ref(),
            None,
            None,
            None,
            None,
            pending.clone(),
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            std::collections::HashMap::new(),
        );
        assert_eq!(
            scene_input.preview_extrusion.as_ref().map(|e| e.target.clone()),
            Some(pending),
            "ghost preview should pick up the live pending target before commit"
        );
    }

    /// #223: while the Repeat tool holds picked bodies and an evaluable count, the scene carries a
    /// ghost mesh per would-be instance (count − 1 copies of each target).
    #[test]
    fn repeat_tool_ghosts_the_would_be_instances() {
        use crate::actions::{Action, AppState, CreatingRepeat, Tool};
        use crate::model::{ExtrudeFace, RepeatMode, RevolveAxis};

        let mut state = AppState::default();
        // Build one real body: a rectangle extruded into a block.
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
        });
        state.apply(Action::CommitExtrusion);
        assert!(state.doc.bodies.iter().any(|b| !b.deleted), "an extruded body exists");

        state.tool = Tool::Repeat;
        state.creating_repeat = Some(CreatingRepeat {
            targets: vec![0],
            axis: RevolveAxis::X,
            mode: RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            ..CreatingRepeat::default()
        });

        let cam = state.cam.clone();
        let element_visibility = state.element_visibility.clone();
        let selection = state.scene_selection.clone();
        let health = state.document_health.clone();

        let scene_input = build_viewport_scene_input(
            &state.doc,
            &cam,
            test_viewport_rect(),
            None,
            &element_visibility,
            &selection,
            &health,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            state.creating_repeat.as_ref(),
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            std::collections::HashMap::new(),
        );
        // 3 instances = original + 2 ghosts; each ghost is a non-empty translated copy.
        assert_eq!(scene_input.repeat_ghosts.len(), 2);
        assert!(scene_input.repeat_ghosts.iter().all(|g| !g.is_empty()));
    }

    /// #180: drawing-view projection axes are orthonormal and orient as expected — a Front
    /// view maps world X→right and Z→up (ignoring depth Y); the isometric basis is a proper
    /// horizontal-right / up frame.
    #[test]
    fn drawing_view_axes_project_as_expected() {
        use crate::model::DrawingOrientation;
        let (r, u) = crate::drawing::view_axes(DrawingOrientation::Front);
        assert_eq!((r, u), (Vec3::X, Vec3::Z));
        let p = Vec3::new(3.0, 99.0, 7.0);
        assert!((p.dot(r) - 3.0).abs() < 1e-6 && (p.dot(u) - 7.0).abs() < 1e-6);

        let (r, u) = crate::drawing::view_axes(DrawingOrientation::Isometric);
        assert!((r.length() - 1.0).abs() < 1e-6, "right is unit");
        assert!((u.length() - 1.0).abs() < 1e-6, "up is unit");
        assert!(r.dot(u).abs() < 1e-6, "right ⟂ up");
        assert!(r.z.abs() < 1e-6, "iso right stays horizontal");
        assert!(u.z > 0.0, "iso up points upward");
    }

    /// #203: while the Loft tool is collecting cross sections, the scene shows a live ghost of
    /// the blended solid, appearing once two sections are picked and dropping back to nothing
    /// with only one.
    #[test]
    fn loft_tool_previews_the_blended_solid_from_picked_sections() {
        use crate::actions::{AppState, CreatingLoft, Tool};
        use crate::model::{Circle, ExtrudeFace, FaceId, LoftSection};

        let mut state = AppState::default();
        // Two circles on planes 10 mm apart, so the loft blends through real space.
        let s0 = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.circles.push(Circle::from_local_center_radius(s0, 0.0, 0.0, 5.0, 0.0));
        state.doc.construction_planes.push(crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                10.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        ));
        let s1 = state.doc.add_sketch(FaceId::ConstructionPlane(1));
        state.doc.circles.push(Circle::from_local_center_radius(s1, 0.0, 0.0, 3.0, 0.0));
        state.tool = Tool::Loft;

        let cam = state.cam.clone();
        let element_visibility = state.element_visibility.clone();
        let selection = state.scene_selection.clone();
        let health = state.document_health.clone();

        let build = |state: &AppState| {
            build_viewport_scene_input(
                &state.doc,
                &cam,
                test_viewport_rect(),
                None,
                &element_visibility,
                &selection,
                &health,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                state.creating_loft.as_ref(),
                None,
                None,
                None,
                None,
                None,
                Vec::new(),
                None,
                None,
                None,
                None,
                &[],
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                std::collections::HashMap::new(),
            )
            .preview_solid
            .is_some()
        };

        // One section: nothing to blend yet, no preview.
        state.creating_loft = Some(CreatingLoft {
            sections: vec![LoftSection { sketch: s0, face: ExtrudeFace::Circle(0) }],
        });
        assert!(!build(&state), "a single section shouldn't preview a loft");

        // Two sections: the blended solid previews.
        state.creating_loft = Some(CreatingLoft {
            sections: vec![
                LoftSection { sketch: s0, face: ExtrudeFace::Circle(0) },
                LoftSection { sketch: s1, face: ExtrudeFace::Circle(1) },
            ],
        });
        assert!(build(&state), "two sections should preview the blended solid");
    }

    #[test]
    fn edge_treatment_preview_shows_the_live_amount_and_suppresses_the_real_body() {
        use crate::actions::{Action, AppState, CreatingEdgeTreatment, Tool};
        use crate::model::{ExtrudeFace, ExtrusionEdgeRef, VertexTreatmentKind};

        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
        });
        state.apply(Action::SetExtrudeDistance { distance: 5.0 });
        state.apply(Action::CommitExtrusion);
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 0);

        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        state.creating_edge_treatment = Some(CreatingEdgeTreatment {
            edges: vec![(0, edge)],
            kind: VertexTreatmentKind::Chamfer,
            amount_live: 2.0,
            text: "2".to_string(),
            user_edited: false,
            pending_focus: false,
        });

        let cam = state.cam.clone();
        let element_visibility = state.element_visibility.clone();
        let selection = state.scene_selection.clone();
        let health = state.document_health.clone();

        let scene_input = build_viewport_scene_input(
            &state.doc,
            &cam,
            test_viewport_rect(),
            None,
            &element_visibility,
            &selection,
            &health,
            None,
            None,
            None,
            None,
            None,
            state.creating_edge_treatment.as_ref(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            std::collections::HashMap::new(),
        );
        let preview = scene_input.preview_extrusion.as_ref().expect("expected a ghost preview");
        assert_eq!(preview.edge_treatments.len(), 1);
        assert_eq!(preview.edge_treatments[0].amount, 2.0);
        assert_eq!(preview.edge_treatments[0].edge, edge);
        assert_eq!(scene_input.editing_extrusion, Some(0));
        assert!(state.doc.extrusions[0].edge_treatments.is_empty());
    }

    /// A two-line right-angle corner (10mm + 10mm legs meeting at (10,0)) in a fresh sketch on
    /// the default XY plane, joined by a `Coincident` constraint — mirrors the equivalent helper
    /// in `actions.rs`'s tests (not reusable across modules since it's private there).
    fn two_coincident_lines_at_a_right_angle(
        state: &mut crate::actions::AppState,
    ) -> (crate::model::SketchId, ConstraintPoint) {
        use crate::actions::Action;
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, LineEnd, ShapeKind};

        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End };
        (sketch, point)
    }

    #[test]
    fn vertex_treatment_preview_points_traces_the_treated_corner() {
        use crate::actions::{AppState, CreatingVertexTreatment};
        use crate::model::VertexTreatmentKind;

        let mut state = AppState::default();
        let (sketch, point) = two_coincident_lines_at_a_right_angle(&mut state);

        let cvt = CreatingVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount_live: 3.0,
            text: "3".to_string(),
            user_edited: false,
            pending_focus: false,
        };

        // Straight chamfer: far endpoint -> truncated point -> truncated point -> far endpoint.
        let points =
            vertex_treatment_preview_points(&state.doc, sketch, &cvt).expect("preview points");
        assert_eq!(points.len(), 4);
        assert!(points[0].abs_diff_eq(Vec3::new(0.0, 0.0, 0.0), 1e-3));
        assert!(points[1].abs_diff_eq(Vec3::new(7.0, 0.0, 0.0), 1e-3), "{:?}", points[1]);
        assert!(points[2].abs_diff_eq(Vec3::new(10.0, 3.0, 0.0), 1e-3), "{:?}", points[2]);
        assert!(points[3].abs_diff_eq(Vec3::new(10.0, 10.0, 0.0), 1e-3));

        // Live-dragging a bigger amount visibly enlarges the cut, with no commit needed.
        let cvt_bigger = CreatingVertexTreatment { amount_live: 5.0, ..cvt.clone() };
        let bigger = vertex_treatment_preview_points(&state.doc, sketch, &cvt_bigger).unwrap();
        assert!(bigger[1].abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-3), "{:?}", bigger[1]);

        // A fillet's bridge is sampled (curved), not a 2-point straight segment.
        let cvt_fillet = CreatingVertexTreatment { kind: VertexTreatmentKind::Fillet, ..cvt };
        let fillet_points =
            vertex_treatment_preview_points(&state.doc, sketch, &cvt_fillet).unwrap();
        assert_eq!(fillet_points.len(), 2 + crate::model::BEZIER_SEGMENTS + 1);

        // No preview once the corner can't be treated (amount collapsed to zero).
        let cvt_zero = CreatingVertexTreatment { amount_live: 0.0, ..cvt_fillet };
        assert!(vertex_treatment_preview_points(&state.doc, sketch, &cvt_zero).is_none());
    }

    #[test]
    fn launch_maximize_strategy_matches_platform() {
        if uses_deferred_launch_maximize() {
            assert_eq!(native_options().viewport.maximized, None);
        } else {
            assert_eq!(native_options().viewport.maximized, Some(true));
        }
    }

    #[test]
    fn launch_maximize_waits_for_post_first_paint_on_macos() {
        if uses_deferred_launch_maximize() {
            assert_eq!(
                initial_launch_maximize_frames(),
                MACOS_LAUNCH_MAXIMIZE_DELAY_FRAMES
            );
        } else {
            assert_eq!(initial_launch_maximize_frames(), 0);
        }
    }

    #[test]
    fn tick_launch_maximize_counts_down_to_zero() {
        let ctx = egui::Context::default();
        let mut frames = 2;
        tick_launch_maximize(&mut frames, &ctx);
        assert_eq!(frames, 1);
        tick_launch_maximize(&mut frames, &ctx);
        assert_eq!(frames, 0);
        tick_launch_maximize(&mut frames, &ctx);
        assert_eq!(frames, 0);
    }

    #[test]
    fn clip_segment_clamps_infinite_spike_to_viewport() {
        let vp = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 600.0));
        let (a, b) = clip_segment_to_rect(
            Pos2::new(-12_000.0, 300.0),
            Pos2::new(12_000.0, 300.0),
            vp,
        )
        .expect("horizon spike should clip");
        assert!((a.x - vp.min.x).abs() < 0.01);
        assert!((b.x - vp.max.x).abs() < 0.01);
        assert!((a.y - 300.0).abs() < 0.01);
        assert!((b.y - 300.0).abs() < 0.01);
    }

    #[test]
    fn clip_segment_returns_none_when_fully_outside() {
        let vp = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        assert!(clip_segment_to_rect(Pos2::new(-50.0, -20.0), Pos2::new(50.0, -10.0), vp).is_none());
    }

    #[test]
    fn clip_segment_preserves_interior_segment() {
        let vp = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 100.0));
        let (a, b) = clip_segment_to_rect(Pos2::new(10.0, 20.0), Pos2::new(80.0, 70.0), vp).unwrap();
        assert_eq!(a, Pos2::new(10.0, 20.0));
        assert_eq!(b, Pos2::new(80.0, 70.0));
    }

    #[test]
    fn z_axis_color_matches_view_cube_blue() {
        assert_eq!(col::Z_AXIS, Color32::from_rgb(80, 140, 230));
    }

    #[test]
    fn orbit_pivot_ground_marker_is_smaller_than_pivot() {
        assert!(ORBIT_PIVOT_GROUND_RADIUS < ORBIT_PIVOT_RADIUS);
    }

    #[test]
    fn z_axis_extends_along_positive_z_from_origin() {
        let end = Vec3::new(0.0, 0.0, GRID_EXTENT);
        assert!(end.z > 0.0);
        assert_eq!(end.x, 0.0);
        assert_eq!(end.y, 0.0);
    }

    #[test]
    fn second_viewport_click_commits_sketch() {
        assert!(should_commit_sketch_on_click(true, true, false));
        assert!(!should_commit_sketch_on_click(false, true, false));
        assert!(!should_commit_sketch_on_click(true, true, true));
        assert!(!should_commit_sketch_on_click(true, false, false));
    }

    #[test]
    fn select_all_while_focused_and_not_user_edited() {
        assert!(should_select_all_rect_value(false, true, true, false, false, false));
    }

    #[test]
    fn select_all_on_focus_gain_or_pending_focus() {
        assert!(should_select_all_rect_value(true, true, true, false, true, false));
        assert!(should_select_all_rect_value(false, true, true, true, true, false));
    }

    #[test]
    fn no_select_all_after_user_edited_without_focus_change() {
        assert!(!should_select_all_rect_value(false, true, true, false, true, false));
    }

    #[test]
    fn typing_multi_digit_value_does_not_reselect_after_each_digit() {
        // First keystroke on a live-tracked value: don't re-select after the digit lands.
        assert!(!should_select_all_rect_value(false, true, true, false, false, true));
        // Later frames while the user continues typing.
        assert!(!should_select_all_rect_value(false, true, true, false, true, false));
        assert!(!should_select_all_rect_value(false, true, true, false, true, true));
    }

    #[test]
    fn live_mouse_tracking_still_selects_before_user_types() {
        assert!(should_select_all_rect_value(false, true, true, false, false, false));
    }

    fn rectangle_anchors(shape: egui::Rect) -> (egui::Pos2, egui::Pos2) {
        (
            egui::pos2(shape.center().x, shape.max.y),
            egui::pos2(shape.min.x, shape.center().y),
        )
    }

    #[test]
    fn rectangle_dim_labels_use_preferred_offsets_when_clear() {
        use super::{
            rectangle_dim_layouts, HEIGHT_LABEL_OFFSET, WIDTH_LABEL_OFFSET,
        };
        let shape = egui::Rect::from_min_max(egui::pos2(50.0, 50.0), egui::pos2(400.0, 400.0));
        let (bottom_mid, left_mid) = rectangle_anchors(shape);
        let (width, height) = rectangle_dim_layouts(bottom_mid, left_mid, "10", "10");
        assert_eq!(width.pos, bottom_mid + WIDTH_LABEL_OFFSET);
        assert_eq!(height.pos, left_mid + HEIGHT_LABEL_OFFSET);
    }

    #[test]
    fn rectangle_dim_labels_avoid_each_other() {
        use super::{rectangle_dim_layouts, rectangle_labels_clear};
        let shape = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(200.0, 160.0));
        let (bottom_mid, left_mid) = rectangle_anchors(shape);
        let (width, height) = rectangle_dim_layouts(bottom_mid, left_mid, "10", "10");
        assert!(rectangle_labels_clear(width.rect, height.rect));
    }

    #[test]
    fn plane_angle_dim_layout_is_near_angle_gizmo_not_offset_tip() {
        use super::{
            axis_angle_handle, axis_offset_handle, plane_dim_layouts, PlaneReference,
        };
        use crate::construction::plane_from_axis;
        let reference = PlaneReference::Axis {
            origin: Vec3::ZERO,
            direction: Vec3::X,
            label: "Line".to_string(),
        };
        let plane = plane_from_axis(20.0, 45.0, Vec3::ZERO, Vec3::X);
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let layouts = plane_dim_layouts(&project, &plane, &reference, 20.0, 45.0).unwrap();
        let angle_layout = layouts.1.expect("axis mode should have angle layout");
        let angle_center = angle_layout.pos + super::dim_input_max_size() * 0.5;
        let handle_screen = project(axis_angle_handle(Vec3::ZERO, Vec3::X, 45.0)).unwrap();
        let offset_screen =
            project(axis_offset_handle(Vec3::ZERO, Vec3::X, 20.0, 45.0)).unwrap();
        assert!(
            (angle_center - handle_screen).length()
                < (angle_center - offset_screen).length()
        );
        let handle_rect =
            egui::Rect::from_center_size(handle_screen, egui::vec2(24.0, 24.0));
        assert!(!angle_layout.rect.intersects(handle_rect));
    }

    #[test]
    fn angle_dim_input_box_clears_gizmo_handle() {
        // The editable angle-dimension input box must not sit on top of the gizmo grab
        // handle, otherwise the handle can't be grabbed (#40). Check across a spread of
        // wedge angles that the box rect stays clear of the handle's grab circle.
        use super::{dim_input_size_for_text, ANGLE_DIM_INPUT_GIZMO_CLEARANCE_PX};
        use crate::construction::AXIS_GIZMO_HANDLE_HIT_RADIUS_PX;
        use crate::dimensions::{arc_dimension_world_geom, ARC_RADIUS};
        let center = Vec3::ZERO;
        let normal = Vec3::Z;
        // Identity projection: world XY maps straight to screen px.
        let project = |w: Vec3| Pos2::new(w.x, w.y);
        for deg in [20.0_f32, 45.0, 90.0, 135.0, 160.0] {
            let theta = deg.to_radians();
            let dir_a = Vec3::X;
            let dir_b = Vec3::new(theta.cos(), theta.sin(), 0.0);
            let geom = arc_dimension_world_geom(
                center,
                dir_a,
                dir_b,
                normal,
                ARC_RADIUS,
                ANGLE_DIM_INPUT_GIZMO_CLEARANCE_PX,
            )
            .unwrap();
            let box_center = project(geom.label_center);
            let size = dim_input_size_for_text("80");
            let rect = egui::Rect::from_center_size(box_center, size);
            let handle = project(center + dir_b * ARC_RADIUS);
            // Distance from the handle to the nearest point of the box rect.
            let nearest = rect.clamp(handle);
            let gap = (nearest - handle).length();
            assert!(
                gap > AXIS_GIZMO_HANDLE_HIT_RADIUS_PX,
                "input box must clear the gizmo handle at {deg} deg (gap {gap})"
            );
        }
    }

    #[test]
    fn rectangle_dim_labels_push_apart_when_overlapping() {
        use super::{
            rectangle_dim_layouts, rectangle_labels_clear, HEIGHT_LABEL_OFFSET,
            WIDTH_LABEL_OFFSET,
        };
        // Very short preview: preferred width/height labels overlap near the bottom-left corner.
        let shape = egui::Rect::from_min_max(egui::pos2(300.0, 300.0), egui::pos2(340.0, 308.0));
        let (bottom_mid, left_mid) = rectangle_anchors(shape);
        let (width, height) = rectangle_dim_layouts(bottom_mid, left_mid, "10", "10");
        assert!(
            width.pos != bottom_mid + WIDTH_LABEL_OFFSET
                || height.pos != left_mid + HEIGHT_LABEL_OFFSET,
            "at least one label should move when they overlap"
        );
        assert!(rectangle_labels_clear(width.rect, height.rect));
    }

    fn line_dim_center(layout: super::DimInputLayout) -> egui::Pos2 {
        layout.pos + layout.rect.size() * 0.5
    }

    #[test]
    fn line_dim_label_stays_on_line_midpoint() {
        use super::{line_dim_layout, line_perpendicular_unit};
        let pa = egui::pos2(40.0, 180.0);
        let pb = egui::pos2(360.0, 220.0);
        let mid = pa.lerp(pb, 0.5);
        let dir = (pb - pa).normalized();
        let center = line_dim_center(line_dim_layout(pa, pb, "10"));
        let rel = center - mid;
        let along = rel.dot(dir);
        assert!(
            along.abs() < 1.0,
            "label center should sit on the line midpoint, along={along}"
        );
        let perp = line_perpendicular_unit(pa, pb);
        assert!(rel.dot(perp).abs() > 0.0);
    }

    #[test]
    fn line_dim_label_keeps_perpendicular_distance_when_line_tilts() {
        use super::{dist_rect_to_segment, line_dim_layout, LINE_LABEL_DISTANCE};
        let pa = egui::pos2(100.0, 200.0);
        for dy in [0.0, 40.0, 80.0, 120.0, -60.0] {
            let pb = egui::pos2(300.0, 200.0 + dy);
            let mid = pa.lerp(pb, 0.5);
            let dir = (pb - pa).normalized();
            let layout = line_dim_layout(pa, pb, "10");
            let center = line_dim_center(layout);
            let along = (center - mid).dot(dir);
            assert!(along.abs() < 1.0, "dy={dy}: along={along}");
            let gap = dist_rect_to_segment(layout.rect, pa, pb);
            assert!(
                (gap - LINE_LABEL_DISTANCE).abs() < 1.0,
                "dy={dy}: expected gap {LINE_LABEL_DISTANCE}, got {gap}"
            );
        }
    }

    #[test]
    fn line_dim_label_avoids_segment() {
        use super::{line_dim_layout, segment_intersects_rect, DIM_LABEL_GAP};
        let pa = egui::pos2(200.0, 200.0);
        let pb = egui::pos2(320.0, 260.0);
        let layout = line_dim_layout(pa, pb, "10");
        assert!(!segment_intersects_rect(
            pa,
            pb,
            layout.rect.expand(DIM_LABEL_GAP)
        ));
    }

    #[test]
    fn keyboard_shortcuts_suppressed_while_text_input_focused() {
        use super::keyboard_shortcuts_suppressed;
        let ctx = egui::Context::default();
        assert!(!keyboard_shortcuts_suppressed(&ctx));
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::new("test_text_input")));
        assert!(keyboard_shortcuts_suppressed(&ctx));
    }

    #[test]
    fn should_commit_sketch_on_enter_focused_field_or_unfocused_viewport() {
        use super::should_commit_sketch_on_enter;
        assert!(should_commit_sketch_on_enter(true, true, false));
        assert!(should_commit_sketch_on_enter(false, false, true));
        assert!(!should_commit_sketch_on_enter(false, true, true));
        assert!(!should_commit_sketch_on_enter(false, false, false));
    }

    #[test]
    fn next_plane_focus_dim_toggles_offset_and_angle() {
        use super::{next_plane_focus_dim, PlaneDim};
        assert_eq!(next_plane_focus_dim(PlaneDim::Offset), PlaneDim::Angle);
        assert_eq!(next_plane_focus_dim(PlaneDim::Angle), PlaneDim::Offset);
    }

    #[test]
    fn width_focus_maps_to_bottom_edge() {
        use super::{rect_dim_edge_for_focus, rect_highlight_edge, RectDimEdge};
        assert_eq!(rect_dim_edge_for_focus(0), Some(RectDimEdge::Width));
        let corners = [
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(5.0, 2.0, 0.0),
            Vec3::new(5.0, 8.0, 0.0),
            Vec3::new(1.0, 8.0, 0.0),
        ];
        let (a, b) = rect_highlight_edge(corners, RectDimEdge::Width);
        assert_eq!(a, Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(b, Vec3::new(5.0, 2.0, 0.0));
    }

    #[test]
    fn height_focus_maps_to_left_edge() {
        use super::{rect_dim_edge_for_focus, rect_highlight_edge, RectDimEdge};
        assert_eq!(rect_dim_edge_for_focus(1), Some(RectDimEdge::Height));
        let corners = [
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(5.0, 2.0, 0.0),
            Vec3::new(5.0, 8.0, 0.0),
            Vec3::new(1.0, 8.0, 0.0),
        ];
        let (a, b) = rect_highlight_edge(corners, RectDimEdge::Height);
        assert_eq!(a, Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(b, Vec3::new(1.0, 8.0, 0.0));
    }

    #[test]
    fn resolve_viewport_hover_highlight_suppressed_returns_none() {
        use super::resolve_viewport_hover_highlight;
        let doc = crate::model::Document::default();
        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |_: glam::Vec3| Some(egui::Pos2::ZERO);
        assert!(
            resolve_viewport_hover_highlight(
                true,
                crate::actions::Tool::Select,
                None,
                false,
                false,
                false,
                false,
                Some(egui::Pos2::ZERO),
                &cam,
                viewport,
                &vp,
                &doc,
                &project,
                None,
            )
            .is_none()
        );
    }

    /// #227: while a body-set tool is active, the whole body under the cursor hover-highlights
    /// (it's what the picker accepts), rendered as an `Element(Body)` aura.
    #[test]
    fn combine_tool_hovers_the_body_under_the_cursor() {
        use super::gpu_viewport;
        use super::resolve_viewport_hover_highlight;
        use crate::hierarchy::SceneElement;

        // A single imported-mesh body: one big triangle in the ground plane.
        let mut doc = crate::model::Document::default();
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[
                glam::Vec3::new(-20.0, -20.0, 0.0),
                glam::Vec3::new(20.0, -20.0, 0.0),
                glam::Vec3::new(0.0, 20.0, 0.0),
            ]],
            source_name: "tri".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        // Aim at the triangle's centroid.
        let centroid = glam::Vec3::new(0.0, -20.0 / 3.0, 0.0);
        let cursor = project(centroid).expect("centroid projects into the viewport");

        let hover = resolve_viewport_hover_highlight(
            false,
            crate::actions::Tool::Combine,
            None,
            false,
            false,
            false,
            false,
            Some(cursor),
            &cam,
            viewport,
            &vp,
            &doc,
            &project,
            None,
        );
        assert!(
            matches!(
                hover,
                Some(gpu_viewport::ViewportHoverHighlight::Element(SceneElement::Body(0)))
            ),
            "combine tool should hover-highlight the whole body, got {hover:?}"
        );
    }

    #[test]
    fn constraint_tool_hovers_origin_and_origin_axes() {
        use super::gpu_viewport;
        use super::resolve_viewport_hover_highlight;
        use crate::actions::SketchSession;
        use crate::hierarchy::SceneElement;
        use crate::model::{ConstraintLine, SketchAxis};

        // #394: with the Constraint tool, the sketch origin and the fixed origin axes are
        // constraint targets, so they must hover-highlight like they click-select (#241).
        let mut doc = crate::model::Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        // A line far from the origin so it never wins the pick.
        doc.lines
            .push(crate::model::Line::from_local_endpoints(sketch, 30.0, 30.0, 60.0, 30.0));

        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let resolve = |cursor: egui::Pos2| {
            resolve_viewport_hover_highlight(
                false,
                crate::actions::Tool::Constraint,
                Some(SketchSession { sketch }),
                false,
                false,
                false,
                false,
                Some(cursor),
                &cam,
                viewport,
                &vp,
                &doc,
                &project,
                None,
            )
        };

        let over_origin = project(glam::Vec3::ZERO).expect("origin projects");
        let hover = resolve(over_origin);
        assert!(
            matches!(
                hover,
                Some(gpu_viewport::ViewportHoverHighlight::Element(SceneElement::Origin))
            ),
            "hovering the origin should highlight it, got {hover:?}"
        );

        let over_x_axis = project(glam::Vec3::new(15.0, 0.0, 0.0)).expect("axis projects");
        let hover = resolve(over_x_axis);
        assert!(
            matches!(
                &hover,
                Some(gpu_viewport::ViewportHoverHighlight::Element(SceneElement::FaceEdge(
                    ConstraintLine::OriginAxis(SketchAxis::X)
                )))
            ),
            "hovering the X origin axis should highlight it, got {hover:?}"
        );
    }

    #[test]
    fn dimension_tool_hovers_a_dimensionable_line() {
        use super::gpu_viewport;
        use super::resolve_viewport_hover_highlight;
        use crate::actions::SketchSession;
        use crate::construction::PickTargetKind;

        // A sketch on the ground plane with one line near the origin.
        let mut doc = crate::model::Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines
            .push(crate::model::Line::from_local_endpoints(sketch, -20.0, 0.0, 20.0, 0.0));

        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        // Aim the cursor at the line's midpoint on screen.
        let mid = project(glam::Vec3::ZERO).expect("origin projects into the viewport");

        let hover = resolve_viewport_hover_highlight(
            false,
            crate::actions::Tool::Dimension,
            Some(SketchSession { sketch }),
            false,
            false,
            false,
            false,
            Some(mid),
            &cam,
            viewport,
            &vp,
            &doc,
            &project,
            None,
        );
        assert!(
            matches!(
                hover,
                Some(gpu_viewport::ViewportHoverHighlight::PickTarget(
                    PickTargetKind::Line(0)
                ))
            ),
            "hovering a line with the Dimension tool should highlight it, got {hover:?}"
        );
    }

    #[test]
    fn dim_input_text_width_grows_with_expression_up_to_max_chars() {
        assert!((super::dim_input_text_width("10") - 48.0).abs() < 1e-4);
        let expr = "2mm + 1ft";
        assert!(super::dim_input_text_width(expr) > 48.0);
        assert!(super::dim_input_text_width(expr) < super::dim_input_max_size().x);
        let capped = super::dim_input_text_width(&"x".repeat(30));
        let maxed = super::dim_input_text_width(&"x".repeat(20));
        assert!((capped - maxed).abs() < 1e-4);
    }

    #[test]
    fn dim_input_selection_highlight_is_faint() {
        use super::col::DIM_INPUT_SELECTION;
        assert!(
            DIM_INPUT_SELECTION.a() <= 48,
            "selection fill should be faint (alpha <= 48), got {}",
            DIM_INPUT_SELECTION.a()
        );
    }

    fn xy_frame() -> SketchFrame {
        SketchFrame {
            origin: Vec3::ZERO,
            u_axis: Vec3::X,
            v_axis: Vec3::Y,
            normal: Vec3::Z,
        }
    }

    fn make_cr(origin: (f32, f32), texts: [&str; 2], mouse: (f32, f32)) -> CreatingRect {
        CreatingRect {
            origin: Vec3::new(origin.0, origin.1, 0.0),
            texts: [texts[0].to_string(), texts[1].to_string()],
            focused: 0,
            last_mouse: Vec3::new(mouse.0, mouse.1, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
        }
    }

    #[test]
    fn end_point_free_follows_mouse() {
        let doc = crate::model::Document::default();
        let cr = make_cr((0., 0.), ["", ""], (10., 4.));
        let frame = xy_frame();
        let e = cr.end_point(&frame, &doc);
        assert!((e.x - 10.0).abs() < 1e-4);
        assert!((e.y - 4.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_one_constrained() {
        let doc = crate::model::Document::default();
        let frame = xy_frame();
        let cr = make_cr((0., 0.), ["5", ""], (12., 3.));
        let e = cr.end_point(&frame, &doc);
        assert!((e.x - 5.0).abs() < 1e-4 && (e.y - 3.0).abs() < 1e-4);

        let cr2 = make_cr((10., 20.), ["5", ""], (3., 15.));
        let e2 = cr2.end_point(&frame, &doc);
        assert!((e2.x - 5.0).abs() < 1e-4);
        assert!((e2.y - 15.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_both_constrained() {
        let doc = crate::model::Document::default();
        let frame = xy_frame();
        let cr = make_cr((0., 0.), ["3", "7"], (99., -4.));
        let e = cr.end_point(&frame, &doc);
        assert!((e.x - 3.0).abs() < 1e-4);
        assert!((e.y + 7.0).abs() < 1e-4);
    }

    #[test]
    fn end_point_invalid_text_falls_back_to_mouse() {
        let doc = crate::model::Document::default();
        let frame = xy_frame();
        let cr = make_cr((0., 0.), ["abc", "12x"], (8., 9.));
        let e = cr.end_point(&frame, &doc);
        assert!((e.x - 8.0).abs() < 1e-4);
        assert!((e.y - 9.0).abs() < 1e-4);
    }

    #[test]
    fn side_panel_resize_inactive_without_resize_drag() {
        egui::__run_test_ctx(|ctx| {
            assert!(!side_panel_resize_active(ctx));
        });
    }
}