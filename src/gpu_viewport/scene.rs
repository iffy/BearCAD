//! CPU-side scene mesh builder for the GPU viewport.

use crate::actions::SketchSession;
use crate::camera::Camera;
use crate::constraint_viewport::ConstraintViewportGraphic;
use crate::constraints::constraint_segment_endpoints;
use crate::document_health::constraint_annotation_color;
use crate::document_health::{health_tint_color, DocumentHealth};
use crate::document_lifecycle::{circle_alive, constraint_alive, line_alive};
use crate::construction::{
    axis_angle_handle, axis_normal, axis_reference_perp, gizmo_display_offset, global_axis_segment,
    plane_corners, AxisGizmoHit, AXIS_ANGLE_GIZMO_RADIUS_MM, CONSTRUCTION_DASH_GAP_PX,
    CONSTRUCTION_DASH_LENGTH_PX, CONSTRUCTION_RGBA, FACE_HOVER_FILL_MULTIPLIER, PLANE_FILL_RGBA,
    GIZMO_HANDLE_HOVER_RGBA, PLANE_DISPLAY_HALF, PickTargetKind, PlaneEditDependentPreview,
    PlaneReference,
};
use crate::context::selection_highlight_dashed;
use crate::face::{
    circle_world_perimeter, sketch_geometry_frame,
};
use crate::hierarchy::SceneElement;
use crate::model::{
    Circle, ConstructionPlane, Document, FaceId, Line,
};

/// A live drag-preview of the rectangle tool: its four world-space corners (bottom-left,
/// bottom-right, top-right, top-left) and whether it's construction geometry. Rendered as a
/// translucent quad + closed edge strokes; the committed rectangle is four plain `Line`s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewRect {
    pub corners: [Vec3; 4],
    pub construction: bool,
}
use crate::hierarchy::ElementVisibility;
use crate::dimensions::{
    dimension_arrow_wing_world, pixels_to_world_distance, LinearDimensionWorldGeom,
    PlanarLabelView, ARROW_LENGTH, ARROW_WING, LINE_WIDTH,
};
use crate::gpu_viewport::dim_labels::ViewportDimLabel;
use crate::selection::SceneSelection;
use crate::value::LengthUnit;
use eframe::egui::Color32;
use egui::Rect as UiRect;
use glam::{Mat4, Quat, Vec3};

pub const GRID_EXTENT: f32 = 200.0;
pub const GRID_STEP: f32 = 20.0;

/// Grid steps for the document's unit system (#464): the **fine** subdivision step and
/// the **heavy** step it subdivides, in mm, as consecutive rungs of a unit ladder —
/// powers of ten of a millimetre for metric documents; quarters of an inch up to an
/// inch, inches to a foot, then tens of feet for imperial ones. Each rung divides the
/// next exactly, so heavy lines always land on fine-line positions and the grid never
/// shifts as the ladder steps with zoom. `min_step_mm` is the smallest world spacing
/// worth drawing (a few px on screen); the fine step is the first rung at or above it.
pub fn grid_steps_for_unit(unit: LengthUnit, min_step_mm: f32) -> (f32, f32) {
    let min = if min_step_mm.is_finite() && min_step_mm > 0.0 {
        min_step_mm
    } else {
        10.0
    };
    match unit {
        LengthUnit::Mm | LengthUnit::Cm | LengthUnit::M => {
            // The epsilon keeps exact powers of ten (log10 = whole ± float noise) on
            // their own rung instead of jumping one up.
            let fine = 10f32.powi((min.log10() - 1e-4).ceil() as i32);
            (fine, fine * 10.0)
        }
        LengthUnit::In | LengthUnit::Ft => {
            let next = |s: f32| {
                if s < 1.0 {
                    s * 4.0
                } else if s < 12.0 {
                    12.0
                } else {
                    s * 10.0
                }
            };
            let mut fine_in = 1.0 / 1024.0;
            while fine_in * 25.4 < min && fine_in < 1e9 {
                fine_in = next(fine_in);
            }
            (fine_in * 25.4, next(fine_in) * 25.4)
        }
    }
}

/// Gamma-space blend from `a` to `b`; the fine grid lines fade toward the background
/// with this so ladder transitions never pop lines in or out.
fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
/// Brightness multiplier for geometry outside the active sketch (other sketches, planes).
pub const SKETCH_DIMMED: f32 = 0.50;
/// Ground grid and world axes stay readable while sketching.
pub const SKETCH_GROUND_DIMMED: f32 = 0.82;
pub const CIRCLE_SEGMENTS: usize = 96;

/// Fill opacity for substantial sketch faces (matches the CPU painter).
pub const SOLID_FILL_OPACITY: f32 = 0.25;
/// Fill opacity for all-construction sketch shapes (rectangles, circles).
pub const CONSTRUCTION_FILL_OPACITY: f32 = 0.18;
/// Default semi-transparent fill for construction planes.
pub const DEFAULT_CONSTRUCTION_PLANE_OPACITY: f32 = 0.30;
/// Lift plane fills slightly toward the camera so they win over the ground grid.
pub const PLANE_FILL_DEPTH_BIAS: f32 = 0.02;
/// Base depth lift for sketch shape fills toward the camera.
/// Base fill color for extruded solid bodies (shaded per triangle).
pub const SOLID_FILL: Color32 = Color32::from_rgb(150, 168, 196);
/// Ghost fill for a shadow body (a boolean operation's consumed input) while it's hovered
/// or selected in the Elements pane — the only time shadows render at all.
pub const SHADOW_BODY_FILL: Color32 = Color32::from_rgb(120, 140, 170);
/// How much body fills dim while a sketch is open (#433), so sketch geometry reads on top.
const SKETCH_MODE_BODY_DIM: f32 = 0.45;
const SHADOW_BODY_OPACITY: f32 = 0.30;
/// Fill for a **selected** body (#174): a more saturated blue than the neutral body grey,
/// so selection reads on the body itself, not just its aura outline.
pub const SOLID_FILL_SELECTED: Color32 = Color32::from_rgb(112, 152, 224);
/// Hovered-body fill (#455): a warm gold-tinted grey so hover reads on the body itself.
pub const SOLID_FILL_HOVERED: Color32 = Color32::from_rgb(196, 180, 132);
/// Fill for a body picked into a destructive (cut) element picker (#213) — the red highlight
/// override, e.g. Revolve's cut bodies or a Combine **Cut**'s B side.
pub const SOLID_FILL_CUT: Color32 = Color32::from_rgb(210, 120, 120);
/// Highlighted fill for the in-progress extrusion preview.
pub const SOLID_PREVIEW_FILL: Color32 = Color32::from_rgb(120, 215, 230);
/// Opacity of the in-progress extrusion preview body (before it is committed).
pub const SOLID_PREVIEW_OPACITY: f32 = 0.4;
/// Fill opacity for committed bodies in `ShadingMode::TransparentSolid` (#33).
pub const TRANSPARENT_SOLID_OPACITY: f32 = 0.45;
/// Opacity for a faded descendant body while its ancestor operation is being edited (#260).
const FADED_BODY_OPACITY: f32 = 0.22;
/// Edge-overlay color for `ShadingMode::Wireframe` and `ShadingMode::SolidWireframe` (#33).
/// Bright against both the dark viewport background (pure wireframe) and the mid-tone
/// `SOLID_FILL` body color (solid+wireframe).
pub const WIREFRAME_LINE_COLOR: Color32 = Color32::from_rgb(230, 235, 242);
const WIREFRAME_LINE_WIDTH_PX: f32 = 1.2;
pub const SHAPE_FILL_DEPTH_BIAS_BASE: f32 = 0.04;
/// Per-shape increment so coplanar overlaps resolve stably (higher index wins).
pub const SHAPE_FILL_DEPTH_BIAS_STEP: f32 = 0.008;
/// The per-shape index is taken modulo this before biasing, so the whole committed-fill band
/// stays bounded (`BASE .. BASE + (MOD-1)*STEP + lane`) strictly below the hover-fill and
/// stroke layers no matter how many shapes a sketch has (#143). Without it, the bias is keyed
/// on the raw entity index (`lines[0]`/circle index) and climbs unbounded — a handful of
/// shapes (each rectangle is four lines) push committed fills up past the fixed hover fill,
/// which then z-fights along coplanar overlaps on hover. The wrap only re-collides two
/// coplanar shapes whose indices differ by exactly a multiple of this modulus (rare, needs
/// many overlapping coplanar shapes); the common adjacent-index overlap stays separated.
pub const SHAPE_FILL_DEPTH_BIAS_MODULO: usize = 5;
/// In-progress previews render above committed geometry.
pub const PREVIEW_FILL_DEPTH_BIAS: f32 = 0.2;
/// Ground grid lines are nudged slightly *away* from the camera (rather than sitting exactly
/// on the reference plane) so any real, coincident geometry — most commonly an extruded body's
/// unbiased base cap, which sits at exactly the same z=0 plane as a ground-plane sketch — always
/// wins the depth test and cleanly occludes the grid, instead of z-fighting with it. Z-fighting
/// between two coplanar unbiased surfaces gets visibly worse at low grazing angles and far zoom
/// (reduced depth-buffer precision), which is why it showed up as the ground grid appearing to
/// slice through the middle of a body when orbiting below the ground and zooming out (#78).
pub const GRID_DEPTH_BIAS: f32 = -0.05;
/// Lift strokes toward the camera so lines draw over coplanar face fills and grid.
pub const STROKE_DEPTH_BIAS: f32 = 0.10;
/// Lift construction-plane hover fills above the plane surface (avoids z-fighting).
const HOVER_PLANE_DEPTH_LIFT: f32 = 0.02;
/// Lift sketch-face hover/active fills toward the camera so they sit above committed coplanar
/// fills (which are themselves biased) and just under strokes — otherwise a hover over
/// overlapping faces renders behind/at-equal-depth with those fills and shows patchy
/// artifacts along the overlaps (#19).
const HOVER_FILL_DEPTH_BIAS: f32 = 0.09;
/// Lift an extrusion's top-cap triangles toward the camera when they lie on the target
/// plane, so the solid wins over the (separately rendered) target plane's own fill instead
/// of z-fighting with it at grazing camera angles (#29).
const SOLID_CAP_DEPTH_BIAS: f32 = 0.02;
/// Blue aura outline drawn around the selected bodies' screen-space silhouette (#145/#148):
/// one solid-color mitered stroke (stacking a bright core over a dim halo read as splotchy
/// wherever the two strokes' antialiased edges beat against each other).
pub const BODY_SILHOUETTE_COLOR: Color32 = Color32::from_rgb(95, 165, 245);

const GIZMO_OFFSET_STROKE_PX: f32 = 2.5;
const GIZMO_OFFSET_STROKE_HOVER_PX: f32 = 4.0;
/// Direction arrows on gizmo handles: flat line arrowheads pointing along each direction
/// the handle can move, stood off from the handle disc so they read as separate
/// affordances. Sizes are screen px so the arrows hold a constant on-screen size like the
/// disc handle itself. (These were briefly solid 3D cones, but the perspective scaling
/// made them flare when orbiting/zooming — flat screen-facing arrows stay stable.)
const GIZMO_ARROW_GAP_PX: f32 = 14.0;
const GIZMO_ARROW_HEAD_PX: f32 = 8.0;
const GIZMO_ARROW_WING_PX: f32 = 4.0;
const GIZMO_HANDLE_RADIUS_PX: f32 = 6.0;
const GIZMO_HOVER_INNER_RADIUS_PX: f32 = 9.0;
const GIZMO_HOVER_OUTER_RADIUS_PX: f32 = 14.0;
const GIZMO_ANGLE_CIRCLE_SEGMENTS: usize = 48;
const GIZMO_ANGLE_STROKE_PX: f32 = 1.5;
const GIZMO_ANGLE_STROKE_HOVER_PX: f32 = 2.5;
const GIZMO_ANGLE_ARROW_GAP_PX: f32 = 12.0;
const GIZMO_ANGLE_ARROW_PX: f32 = 5.0;
const GIZMO_ANGLE_WING_PX: f32 = 3.0;
const GIZMO_HANDLE_RING_STROKE_PX: f32 = 1.5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

use crate::gpu_viewport::dim_labels::GpuTextVertex;

#[derive(Clone, Debug, Default)]
pub struct ViewportScene {
    pub vertices: Vec<GpuVertex>,
    /// Ground grid, solids, and standalone lines (drawn first).
    pub indices: Vec<u32>,
    /// Committed coplanar sketch-shape fills, drawn with a stencil mask so each
    /// pixel is painted once (avoids translucent overlaps darkening — #3).
    pub sketch_fill_indices: Vec<u32>,
    /// Construction-plane fills (drawn after sketch fills, without depth write).
    pub plane_fill_indices: Vec<u32>,
    /// Strokes, selection, hover, and previews (drawn on top of plane fills).
    pub overlay_indices: Vec<u32>,
    /// Manipulation gizmos (plane/extrude offset+angle handles). Drawn last with the
    /// depth test disabled so handles stay visible even when behind a body (#36).
    pub gizmo_indices: Vec<u32>,
    /// Body edge-wireframe overlay (#33). Drawn depth-test-disabled, same as gizmos, so
    /// edges stay visible "through" a solid body in solid+wireframe shading mode.
    pub wireframe_indices: Vec<u32>,
    pub text_vertices: Vec<GpuTextVertex>,
    pub text_indices: Vec<u32>,
    /// Tracing images (#170): textured world-space quads, drawn after the opaque scene
    /// (depth-tested, no depth write) so bodies in front occlude them.
    pub images: Vec<ViewportImageQuad>,
    pub view_proj: Mat4,
    pub clear_color: [f32; 4],
}

/// One tracing image's draw data (#170). `id` keys the renderer's texture cache (stable for
/// a given image's content); `rgba` carries the decoded pixels for the first upload.
#[derive(Clone, Debug, Default)]
pub struct ViewportImageQuad {
    pub id: u64,
    /// World corners in UV order: (0,0), (1,0), (1,1), (0,1).
    pub corners: [Vec3; 4],
    pub width_px: u32,
    pub height_px: u32,
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub opacity: f32,
}

/// Decode memo for tracing images (#170): decoding a PNG/JPEG every frame would dwarf the
/// rest of the scene build. Keyed by a cheap content stamp (length + sampled bytes).
fn decoded_tracing_image(image: &crate::model::TracingImage) -> Option<(u64, u32, u32, std::sync::Arc<Vec<u8>>)> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.bytes.len().hash(&mut hasher);
    for chunk in image.bytes.chunks(4096).step_by(16) {
        chunk.hash(&mut hasher);
    }
    let id = hasher.finish();
    thread_local! {
        static DECODED: std::cell::RefCell<std::collections::HashMap<u64, Option<(u32, u32, std::sync::Arc<Vec<u8>>)>>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    DECODED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache.entry(id).or_insert_with(|| {
            image::load_from_memory(&image.bytes).ok().map(|img| {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                (w, h, std::sync::Arc::new(rgba.into_raw()))
            })
        });
        entry
            .as_ref()
            .map(|(w, h, rgba)| (id, *w, *h, rgba.clone()))
    })
}

#[derive(Clone, Copy, Debug)]
pub struct ViewportPalette {
    pub background: Color32,
    pub grid: Color32,
    pub grid_axis: Color32,
    pub x_axis: Color32,
    pub y_axis: Color32,
    pub z_axis: Color32,
    /// Shared stroke color for all solid sketch shape edges (lines, rect edges, circles).
    pub rect_line: Color32,
    /// Fully-constrained solid lines (#172): no remaining degrees of freedom.
    pub rect_line_constrained: Color32,
    pub preview: Color32,
    pub construction: Color32,
    /// Associative projections (#140): dashed like construction, but their own color.
    pub projection: Color32,
    pub dim_edge_highlight: Color32,
    pub construction_plane_fill: Color32,
    pub construction_plane_opacity: f32,
}

impl Default for ViewportPalette {
    fn default() -> Self {
        Self {
            background: Color32::from_gray(28),
            grid: Color32::from_gray(55),
            grid_axis: Color32::from_gray(90),
            x_axis: Color32::from_rgb(200, 70, 70),
            y_axis: Color32::from_rgb(70, 190, 90),
            z_axis: Color32::from_rgb(80, 140, 230),
            rect_line: Color32::from_rgb(120, 170, 240),
            rect_line_constrained: Color32::from_rgb(225, 228, 235),
            preview: Color32::from_rgb(240, 200, 120),
            construction: CONSTRUCTION_RGBA,
            projection: Color32::from_rgb(70, 200, 190),
            dim_edge_highlight: Color32::from_rgb(255, 205, 88),
            construction_plane_fill: PLANE_FILL_RGBA,
            construction_plane_opacity: DEFAULT_CONSTRUCTION_PLANE_OPACITY,
        }
    }
}

/// Hover highlight while picking a sketch face or construction-plane reference.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewportHoverHighlight {
    SketchFace(FaceId),
    PickTarget(PickTargetKind),
    /// An elements-pane row under the cursor (#161): the viewport shows the row's element,
    /// whatever its kind — bodies/extrusions get their aura in the hover color, sketch
    /// entities their usual pick highlight.
    Element(crate::hierarchy::SceneElement),
    /// A closed world-space loop rendered as a filled/outlined polygon (#16/#62/#202): used
    /// for a computed boolean-combined region (which, unlike `SketchFace`, has no `FaceId` of
    /// its own — it's just `ExtrudeFace::Boolean`'s on-demand geometry) and for a Loft tool
    /// cross-section profile under the cursor.
    ClosedLoop { world_loop: Vec<Vec3> },
}

/// Whether curved line `li`'s tangent handles should be drawn (#550): only when the curve or
/// one of its endpoints is selected or hovered, or one of its handles is being manipulated.
/// Keeps handles from cluttering and obscuring every curve in the sketch at rest.
pub(crate) fn bezier_handles_relevant(
    li: usize,
    selection: &SceneSelection,
    hover: &Option<ViewportHoverHighlight>,
    highlighted_handles: &[(usize, bool)],
) -> bool {
    use crate::hierarchy::SceneElement;
    use crate::model::ConstraintPoint;
    if highlighted_handles.iter().any(|&(l, _)| l == li) {
        return true;
    }
    if selection.is_selected(SceneElement::Line(li)) {
        return true;
    }
    if selection.iter().any(|e| {
        matches!(e, SceneElement::Point(ConstraintPoint::LineEndpoint { line, .. }) if line == li)
    }) {
        return true;
    }
    match hover {
        Some(ViewportHoverHighlight::Element(SceneElement::Line(l))) => *l == li,
        Some(ViewportHoverHighlight::PickTarget(kind)) => matches!(
            kind,
            PickTargetKind::Line(l) if *l == li
        ) || matches!(
            kind,
            PickTargetKind::Point(ConstraintPoint::LineEndpoint { line, .. }) if *line == li
        ),
        _ => false,
    }
}

/// Prospective construction plane while creating or editing.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportPlanePreview {
    pub plane: ConstructionPlane,
    pub dependents: Option<PlaneEditDependentPreview>,
    /// Extra outline while offset/angle dimension inputs are visible.
    pub dim_outline: bool,
}

/// Normal offset gizmo for the extrude tool (same arrow as the plane offset gizmo).
#[derive(Clone, Copy, Debug)]
pub struct ViewportExtrudeGizmo {
    pub origin: Vec3,
    pub normal: Vec3,
    pub offset: f32,
    pub color: Color32,
    pub hovered: bool,
}

/// The Move tool's rotation-ring gizmo (#216): a circle in the plane perpendicular to the
/// rotation axis, at the picked bodies' centroid, dragged to set the rotation angle.
#[derive(Clone, Copy, Debug)]
pub struct MoveRotationGizmo {
    pub center: Vec3,
    /// Normalized rotation axis (the ring's normal).
    pub axis: Vec3,
    pub radius: f32,
    pub color: Color32,
    pub hovered: bool,
}

/// The Revolve tool's arc gizmo (#262): an arc from the 0° direction (`zero_dir`) around
/// `axis` through `angle_deg`, with a push/pull disc handle at its far end, dragged around the
/// arc to set the sweep angle.
#[derive(Clone, Copy, Debug)]
pub struct RevolveArcGizmo {
    pub center: Vec3,
    /// Normalized sweep axis.
    pub axis: Vec3,
    /// Unit radial marking 0° (the profile's direction from the axis).
    pub zero_dir: Vec3,
    pub radius: f32,
    pub angle_deg: f32,
    pub color: Color32,
    pub hovered: bool,
}

/// Points along the arc from 0° to `angle_deg` of a [`RevolveArcGizmo`], `zero_dir` rotated
/// about `axis`. Empty if the axis is degenerate.
fn revolve_arc_points(
    center: Vec3,
    axis: Vec3,
    zero_dir: Vec3,
    radius: f32,
    angle_deg: f32,
    segments: usize,
) -> Vec<Vec3> {
    let n = axis.normalize_or_zero();
    if n == Vec3::ZERO {
        return Vec::new();
    }
    let total = angle_deg.to_radians();
    (0..=segments)
        .map(|i| {
            let t = total * i as f32 / segments as f32;
            let rot = glam::Quat::from_axis_angle(n, t);
            center + (rot * zero_dir) * radius
        })
        .collect()
}

/// A closed circle of `segments` points, centred at `center` in the plane perpendicular to
/// `axis`, for the rotation-ring gizmo (#216). Empty if the axis is degenerate.
fn ring_points(center: Vec3, axis: Vec3, radius: f32, segments: usize) -> Vec<Vec3> {
    let n = axis.normalize_or_zero();
    if n == Vec3::ZERO {
        return Vec::new();
    }
    let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = n.cross(reference).normalize_or_zero();
    let v = n.cross(u);
    (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            center + (u * t.cos() + v * t.sin()) * radius
        })
        .collect()
}

/// World-space polyline for the live chamfer/fillet corner preview (#76). See
/// [`ViewportSceneInput::vertex_treatment_preview`].
#[derive(Clone, Debug, PartialEq)]
pub struct VertexTreatmentPreviewGeom {
    pub points: Vec<Vec3>,
}

/// Construction-plane offset/angle gizmo while creating or editing a plane.
#[derive(Clone, Debug)]
pub struct ViewportPlaneGizmo {
    pub reference: PlaneReference,
    pub offset: f32,
    pub angle_deg: f32,
    pub color: Color32,
    pub hover: Option<AxisGizmoHit>,
}

#[derive(Clone, Debug)]
pub struct ViewportSceneInput<'a> {
    pub doc: &'a Document,
    pub cam: &'a Camera,
    pub viewport: UiRect,
    pub palette: ViewportPalette,
    pub sketch_session: Option<SketchSession>,
    pub selection: &'a SceneSelection,
    /// Bodies to fill in the red "cut" highlight (#213): the active tool's destructive picker
    /// contents (Revolve cut bodies, a Combine Cut's B side). Takes precedence over the blue
    /// selection fill.
    pub cut_highlight_bodies: Vec<usize>,
    /// Bodies to render dimmed/translucent because they are descendants of the operation being
    /// edited (#260), so the edit's downstream effects are visually de-emphasized.
    pub faded_bodies: Vec<usize>,
    /// Dashed ghost world-segments previewing an in-progress in-sketch repeat's duplicates
    /// (#232) — the sketch-plane equivalent of the 3D repeat ghost meshes.
    pub sketch_repeat_ghost: Vec<(Vec3, Vec3)>,
    /// Solid preview segments for an in-progress in-sketch mirror's reflection (#542): each
    /// `(a, b, dashed)` draws a solid preview-coloured line (matching the repeat/extrude/revolve
    /// preview styling), or a dashed one when its reflected source is construction geometry.
    pub sketch_mirror_ghost: Vec<(Vec3, Vec3, bool)>,
    /// Live-updated meshes for the faded descendant bodies (#260): while a gizmo edit is being
    /// dragged, `faded_bodies[bi]` renders this preview mesh (recomputed from a scratch document
    /// with the edit applied) instead of its stale committed geometry, so downstream bodies
    /// follow the drag. Keyed by body index; a faded body absent here just fades in place.
    pub edit_preview_meshes: std::collections::HashMap<usize, crate::extrude::SolidMesh>,
    pub element_visibility: &'a ElementVisibility,
    pub preview_rect: Option<PreviewRect>,
    pub preview_line: Option<Line>,
    pub preview_circle: Option<Circle>,
    /// In-progress extrusion (rendered as a translucent preview solid).
    pub preview_extrusion: Option<crate::model::Extrusion>,
    /// A prebuilt ghost-preview solid (e.g. the in-progress revolve, #revolve), drawn
    /// translucent like `preview_extrusion`'s mesh.
    pub preview_solid: Option<crate::extrude::SolidMesh>,
    /// Ghost previews of the Repeat tool's would-be instances (#223): the picked bodies' meshes
    /// translated to each instance offset along the axis, drawn translucent while the count and
    /// spacing change. Empty when the Repeat tool is idle or its configuration doesn't evaluate.
    pub repeat_ghosts: Vec<crate::extrude::SolidMesh>,
    /// Index of the extrusion currently being edited, if any. Its committed body
    /// is suppressed so only the ghost preview is shown while editing.
    pub editing_extrusion: Option<usize>,
    /// Target body when the in-progress extrusion is a **cut** (#142): its committed solid is
    /// suppressed and replaced by a translucent preview of the *cut result* (that body with
    /// `preview_extrusion` subtracted) rather than the additive block, so the preview looks
    /// like the finished cut. `None` for add/new-body extrudes (block preview) or when the
    /// kernel can't build the cut result.
    pub preview_cut_body: Option<usize>,
    /// Sweep cut preview: `(body, mesh)` replacement solids — each listed body
    /// renders this translucent cut-result mesh in place of its intact self, like
    /// `preview_cut_body` does for the in-progress extrusion.
    pub preview_cut_solids: Vec<(usize, crate::extrude::SolidMesh)>,
    /// Bezier tangent handles to draw in the gold pick-highlight color (#472): the
    /// hovered, dragged, and/or selected `(line, near_start)` handles.
    pub highlighted_bezier_handles: Vec<(usize, bool)>,
    pub plane_preview: Option<ViewportPlanePreview>,
    pub active_sketch_face: Option<FaceId>,
    pub dimension_labels: &'a [ViewportDimLabel],
    pub dim_label_view: Option<PlanarLabelView>,
    pub plane_gizmo: Option<ViewportPlaneGizmo>,
    pub extrude_gizmo: Option<ViewportExtrudeGizmo>,
    /// Push/pull gizmo for the in-progress chamfer/fillet tool; reuses the same offset-gizmo
    /// mesh as [`ViewportSceneInput::extrude_gizmo`] (#37/#38).
    pub vertex_treatment_gizmo: Option<ViewportExtrudeGizmo>,
    /// The Move tool's translation-arrow handles (#215): one per world axis, each the same
    /// offset-arrow as the extrude gizmo, anchored at the picked targets' centroid.
    pub move_gizmos: Vec<ViewportExtrudeGizmo>,
    /// The Move tool's rotation-ring gizmo (#216), shown once a rotation axis is picked.
    pub move_rotation_gizmo: Option<MoveRotationGizmo>,
    /// The Revolve tool's arc gizmo (#262), shown once profile faces and an axis are picked.
    pub revolve_arc_gizmo: Option<RevolveArcGizmo>,
    /// Live preview of the treated corner while the chamfer/fillet gizmo is being placed or
    /// dragged (#76): world-space polyline from the first line's far endpoint, through the
    /// truncated point, the bridge, the other truncated point, to the second line's far
    /// endpoint. Recomputed every frame from the live gizmo amount.
    pub vertex_treatment_preview: Option<VertexTreatmentPreviewGeom>,
    pub hover_highlight: Option<ViewportHoverHighlight>,
    /// Extra pick targets to hover-highlight in `hover_color` all at once (#559): every member of a
    /// hovered Selection-Exploder group loupe, so the whole group lights up in the 3D view.
    pub extra_pick_highlights: Vec<crate::construction::PickTargetKind>,
    pub hover_color: Color32,
    pub document_health: &'a DocumentHealth,
    pub constraint_graphics: Option<&'a [ConstraintViewportGraphic]>,
    pub constraint_connector_color: Option<Color32>,
}

impl ViewportScene {
    pub fn build(input: &ViewportSceneInput<'_>) -> Self {
        let vp = input.cam.view_proj(input.viewport);
        let mut scene = Self {
            view_proj: vp,
            clear_color: color32_to_gpu(input.palette.background),
            ..Default::default()
        };
        // Tracing images (#170): a textured quad per visible image on its host plane.
        for (ii, img) in input.doc.tracing_images.iter().enumerate() {
            if img.deleted
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Image(ii))
            {
                continue;
            }
            let Some(frame) =
                crate::face::sketch_frame(input.doc, FaceId::ConstructionPlane(img.plane))
            else {
                continue;
            };
            let Some((id, w, h, rgba)) = decoded_tracing_image(img) else {
                continue;
            };
            let at = |x: f32, y: f32| frame.origin + frame.u_axis * x + frame.v_axis * y;
            let (x0, y0) = img.origin;
            let (x1, y1) = (x0 + img.width_mm, y0 + img.height_mm);
            scene.images.push(ViewportImageQuad {
                id,
                // UV v grows downward in image space; plane-local v grows up — flip v.
                corners: [at(x0, y1), at(x1, y1), at(x1, y0), at(x0, y0)],
                width_px: w,
                height_px: h,
                rgba,
                opacity: 0.85,
            });
        }

        let mut mesh = SceneMesh::new(&mut scene);
        let sketch_dimmed = input.sketch_session.is_some();
        mesh.push_ground(
            input.cam,
            input.viewport,
            &vp,
            sketch_dimmed,
            &input.palette,
            input.doc.default_length_unit,
        );

        for (ci, circle) in input.doc.circles.iter().enumerate() {
            if !circle_alive(input.doc, ci)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Circle(ci))
            {
                continue;
            }
            let dim = input.sketch_session.is_some_and(|s| {
                !sketch_circle_is_active(input.doc, s, ci, circle.sketch)
            });
            let element = SceneElement::Circle(ci);
            mesh.set_index_layer(MeshIndexLayer::SketchFill);
            mesh.push_circle_fill(
                input.doc,
                circle,
                ci,
                input.cam,
                health_tint_color(
                    sketch_color(input.palette.rect_line, dim),
                    input.document_health.element_status(element.clone()),
                ),
                health_tint_color(
                    sketch_color(input.palette.construction, dim),
                    input.document_health.element_status(element),
                ),
                shape_fill_depth_bias_laned(ci, 1),
            );
            mesh.set_index_layer(MeshIndexLayer::Base);
        }

        // Closed loops of plain lines (#66) — fill them the same way a rect/circle face is.
        for sketch in 0..input.doc.sketches.len() {
            for lines in crate::polygon::closed_line_loops(input.doc, sketch) {
                let visible = lines.iter().all(|&li| {
                    line_alive(input.doc, li)
                        && input
                            .element_visibility
                            .effective_visible(input.doc, SceneElement::Line(li))
                });
                if !visible {
                    continue;
                }
                let Some((profile, normal)) = crate::extrude::face_profile_world(
                    input.doc,
                    &crate::model::ExtrudeFace::Polygon(lines.clone()),
                ) else {
                    continue;
                };
                let all_construction = lines
                    .iter()
                    .all(|&li| input.doc.lines.get(li).is_some_and(|l| l.construction));
                let dim = input.sketch_session.is_some_and(|s| {
                    input.doc.lines.get(lines[0]).is_some_and(|l| l.sketch != s.sketch)
                });
                let element = SceneElement::Line(lines[0]);
                mesh.set_index_layer(MeshIndexLayer::SketchFill);
                mesh.push_polygon_fill(
                    &profile,
                    normal,
                    input.cam,
                    health_tint_color(
                        sketch_color(input.palette.rect_line, dim),
                        input.document_health.element_status(element.clone()),
                    ),
                    health_tint_color(
                        sketch_color(input.palette.construction, dim),
                        input.document_health.element_status(element),
                    ),
                    all_construction,
                    shape_fill_depth_bias_laned(lines[0], 2),
                );
                mesh.set_index_layer(MeshIndexLayer::Base);
            }
        }

        // Live cut preview (#142): when the in-progress extrusion cuts a body, mesh that body
        // *with the cut subtracted* so the preview shows the finished result. Only when the
        // kernel can actually build it — otherwise `None` and the intact body + additive-block
        // preview are kept below.
        let preview_cut = input.preview_cut_body.and_then(|bi| {
            let preview = input.preview_extrusion.as_ref()?;
            crate::extrude::preview_cut_body_mesh(input.doc, bi, preview).map(|mesh| (bi, mesh))
        });

        // Solid meshes are rebuilt from the kernel on every `body_solid_mesh` call (#86);
        // compute each visible body's mesh once per frame and share it between the body
        // render below and the selection aura (#145), which used to recompute every mesh a
        // second time and visibly slowed frames while a body was selected.
        // A shadow body (a boolean operation's consumed input) renders only while hovered
        // or selected in the Elements pane; hovering the operation row ghosts all of its
        // inputs at once.
        let shadow_shown = |bi: usize| -> bool {
            if input.selection.is_selected(SceneElement::Body(bi)) {
                return true;
            }
            match &input.hover_highlight {
                Some(ViewportHoverHighlight::Element(SceneElement::Body(h))) => *h == bi,
                Some(ViewportHoverHighlight::Element(SceneElement::BooleanOp(op))) => input
                    .doc
                    .boolean_ops
                    .get(*op)
                    .is_some_and(|o| o.a.contains(&bi) || o.b.contains(&bi)),
                Some(ViewportHoverHighlight::Element(SceneElement::SliceOp(op))) => input
                    .doc
                    .slice_ops
                    .get(*op)
                    .is_some_and(|o| o.targets.contains(&bi)),
                _ => false,
            }
        };
        let body_meshes: Vec<Option<crate::extrude::SolidMesh>> = (0..input.doc.bodies.len())
            .map(|bi| {
                let body = &input.doc.bodies[bi];
                let visible = !body.deleted
                    && input
                        .element_visibility
                        .effective_visible(input.doc, SceneElement::Body(bi))
                    && (!body.shadow || shadow_shown(bi));
                if visible {
                    crate::extrude::body_solid_mesh(input.doc, bi)
                } else {
                    None
                }
            })
            .collect();

        // Extruded solid bodies (3D, depth-tested, flat-shaded).
        for (bi, body) in input.doc.bodies.iter().enumerate() {
            if let Some(editing) = input.editing_extrusion {
                if body.source.owns_extrusion(editing) {
                    continue;
                }
            }
            // The cut target is drawn as the translucent cut-result preview instead of its
            // intact committed solid.
            if preview_cut.as_ref().is_some_and(|(cut_bi, _)| *cut_bi == bi)
                || input.preview_cut_solids.iter().any(|(cut_bi, _)| *cut_bi == bi)
            {
                continue;
            }
            let Some(solid) = body_meshes[bi].as_ref() else {
                continue;
            };
            // Shadow bodies render as a translucent ghost with a wireframe, whatever the
            // shading mode — visually distinct from every real body.
            if body.shadow {
                mesh.push_solid_translucent(solid, SHADOW_BODY_FILL, SHADOW_BODY_OPACITY);
                mesh.push_solid_wireframe(
                    solid,
                    WIREFRAME_LINE_COLOR,
                    input.cam,
                    input.viewport,
                    &vp,
                );
                continue;
            }
            // A descendant of the operation being edited (#260): render it faded/translucent so
            // the edit's downstream effects are de-emphasized while the gizmo is live.
            if input.faded_bodies.contains(&bi) {
                // With a live edit preview available, show the recomputed geometry in the
                // preview style (like the extrusion/repeat previews) so the descendant follows
                // the drag; otherwise fade the stale committed solid in place.
                if let Some(preview) = input.edit_preview_meshes.get(&bi) {
                    mesh.push_solid_translucent(preview, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
                } else {
                    mesh.push_solid_translucent(solid, SOLID_FILL, FADED_BODY_OPACITY);
                }
                continue;
            }
            // A body picked into a destructive (cut) picker previews semi-transparent in the cut
            // red (#264) so the cut against the side-A bodies shows through, whatever the shading
            // mode; its red aura is drawn after `push_selection`. Otherwise a selected body fills
            // the saturated selection blue (#174).
            if input.cut_highlight_bodies.contains(&bi) {
                mesh.push_solid_translucent(solid, SOLID_FILL_CUT, TRANSPARENT_SOLID_OPACITY);
                continue;
            }
            // Selection and hover recolor the body itself (#455): the fill in shaded
            // modes, the lines in wireframe — no outline aura.
            let hovered = matches!(
                &input.hover_highlight,
                Some(ViewportHoverHighlight::Element(SceneElement::Body(h))) if *h == bi
            );
            let fill = if input.selection.is_selected(SceneElement::Body(bi)) {
                SOLID_FILL_SELECTED
            } else if hovered {
                SOLID_FILL_HOVERED
            } else {
                SOLID_FILL
            };
            let line_color = if input.selection.is_selected(SceneElement::Body(bi)) {
                BODY_SILHOUETTE_COLOR
            } else if hovered {
                SOLID_FILL_HOVERED
            } else {
                WIREFRAME_LINE_COLOR
            };
            // Sketch mode dims every body (#433): the bright face shading otherwise
            // fights the sketch lines and dimension labels drawn over it.
            let fill = if input.sketch_session.is_some() {
                scale_color(fill, SKETCH_MODE_BODY_DIM)
            } else {
                fill
            };
            let cap_plane = body
                .source
                .extrusion_indices()
                .first()
                .and_then(|&ei| input.doc.extrusions.get(ei))
                .and_then(|ext| crate::extrude::target_top_plane(input.doc, ext));
            // Shading mode (#33) picks how the committed body renders: `Solid` (today's
            // existing look) is opaque fill only; `Wireframe` is edges only, no fill;
            // `TransparentSolid` is translucent fill, no edges; `SolidWireframe` is opaque
            // fill plus an edge overlay that stays visible "through" the body (mirrors how
            // gizmos draw through bodies — depth-test disabled, see `MeshIndexLayer::Wireframe`).
            match input.cam.shading_mode() {
                crate::camera::ShadingMode::Solid => {
                    mesh.push_solid(solid, fill, input.cam, cap_plane);
                }
                crate::camera::ShadingMode::TransparentSolid => {
                    mesh.push_solid_translucent(solid, fill, TRANSPARENT_SOLID_OPACITY);
                }
                crate::camera::ShadingMode::Wireframe => {
                    mesh.push_solid_wireframe(
                        solid,
                        line_color,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::SolidWireframe => {
                    mesh.push_solid(solid, fill, input.cam, cap_plane);
                    mesh.push_solid_wireframe(
                        solid,
                        line_color,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::Realistic => {
                    mesh.push_solid_realistic(solid, fill, input.cam, cap_plane);
                }
            }
        }
        // Live preview of the in-progress extrusion (semi-transparent until committed). A cut
        // (#142) previews the whole cut *result* solid — the target body with this extrusion
        // subtracted — in place of the additive block, so it looks like the finished cut.
        if let Some((_, solid)) = &preview_cut {
            mesh.push_solid_translucent(solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        } else if let Some(preview) = input.preview_extrusion.as_ref() {
            // Cached, text-aware preview mesher (#386): rebuilding a text extrusion's
            // per-glyph kernel solids every frame made the drag unusably laggy.
            if let Some(solid) = crate::extrude::preview_extrusion_mesh(input.doc, preview) {
                mesh.push_solid_translucent(&solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
            }
        }
        if let Some(solid) = input.preview_solid.as_ref() {
            mesh.push_solid_translucent(solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        }
        for (_, solid) in &input.preview_cut_solids {
            mesh.push_solid_translucent(solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        }
        // Repeat-tool instance ghosts (#223): each would-be copy, translucent like other previews.
        for ghost in &input.repeat_ghosts {
            mesh.push_solid_translucent(ghost, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        }

        let mut plane_draws: Vec<(usize, ConstructionPlane, Color32, f32)> = Vec::new();
        for (i, plane) in input.doc.construction_planes.iter().enumerate() {
            if plane.deleted
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::ConstructionPlane(i))
            {
                continue;
            }
            let session_face = input
                .sketch_session
                .and_then(|s| input.doc.sketch_face(s.sketch));
            let active = session_face == Some(FaceId::ConstructionPlane(i));
            let color = if active {
                input.palette.dim_edge_highlight
            } else {
                input.palette.construction_plane_fill
            };
            plane_draws.push((i, plane.clone(), color, plane_camera_depth(plane, input.cam)));
        }
        plane_draws.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        mesh.set_index_layer(MeshIndexLayer::PlaneFill);
        let plane_opacity = input.palette.construction_plane_opacity;
        for (i, plane, color, _) in plane_draws {
            mesh.push_plane(&plane, i, color, plane_opacity, input.cam);
        }
        mesh.set_index_layer(MeshIndexLayer::Base);

        // Fully-constrained lines (#172) draw in their own color; the set is memoized per
        // document state inside the solver bridge.
        let constrained_lines = crate::sketch_solver::fully_constrained_lines(input.doc);
        for (li, line) in input.doc.lines.iter().enumerate() {
            if !line_alive(input.doc, li)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Line(li))
                || input.selection.is_selected(SceneElement::Line(li))
            {
                continue;
            }
            let element = SceneElement::Line(li);
            let dim = input.sketch_session.is_some_and(|s| line.sketch != s.sketch);
            let base = if line.projection.is_some() {
                sketch_color(input.palette.projection, dim)
            } else if line.construction {
                sketch_color(input.palette.construction, dim)
            } else if constrained_lines.contains(&li) {
                sketch_color(input.palette.rect_line_constrained, dim)
            } else {
                sketch_color(input.palette.rect_line, dim)
            };
            let color = health_tint_color(base, input.document_health.element_status(element));
            if let Some(points) = line_world_polyline(input.doc, line) {
                if line.construction {
                    mesh.push_dashed_polyline_segment(
                        &points,
                        color,
                        2.0,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                } else {
                    mesh.push_polyline_segment(&points, color, 2.0, input.cam, input.viewport, &vp);
                }
            }
        }

        // Sketch text (#282): draw each baked glyph contour as a closed polyline, transformed by
        // the text's origin/rotation onto its sketch plane. Selected text uses the selection color.
        for (ti, text) in input.doc.sketch_texts.iter().enumerate() {
            if text.deleted
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::SketchText(ti))
            {
                continue;
            }
            let Some(frame) = crate::face::sketch_geometry_frame(input.doc, text.sketch) else {
                continue;
            };
            let dim = input.sketch_session.is_some_and(|s| text.sketch != s.sketch);
            let selected = input.selection.is_selected(SceneElement::SketchText(ti));
            let color = if selected {
                input.palette.rect_line_constrained
            } else {
                sketch_color(input.palette.rect_line, dim)
            };
            let (sin, cos) = text.rotation.sin_cos();
            for contour in &text.contours {
                if contour.len() < 2 {
                    continue;
                }
                let mut pts: Vec<Vec3> = contour
                    .iter()
                    .map(|&(x, y)| {
                        // Rotate about the text origin, then place on the sketch plane.
                        let rx = x * cos - y * sin + text.origin.0;
                        let ry = x * sin + y * cos + text.origin.1;
                        crate::face::local_to_world(&frame, rx, ry)
                    })
                    .collect();
                if let Some(first) = pts.first().copied() {
                    pts.push(first); // close the loop
                }
                mesh.push_polyline_segment(&pts, color, 2.0, input.cam, input.viewport, &vp);
            }
            // A selected text shows its nine anchor points (#356/#408) — the corners, edge
            // midpoints, and centre — pickable with the Constraint tool to hold the text to
            // other geometry.
            if selected {
                let anchor_color = input.palette.preview;
                for &pt in &crate::text::sketch_text_anchor_points(text) {
                    let world = {
                        let rx = pt.0 * cos - pt.1 * sin + text.origin.0;
                        let ry = pt.0 * sin + pt.1 * cos + text.origin.1;
                        crate::face::local_to_world(&frame, rx, ry)
                    };
                    mesh.push_point_marker(world, anchor_color, 4.5, input.cam, input.viewport, &vp);
                }
                // A wrapped text also shows its box as a dashed outline with a width drag
                // handle on each vertical edge (#409).
                if let Some((x0, y0, x1, y1)) = crate::text::wrap_box_baseline(text) {
                    let corner = |x: f32, y: f32| {
                        let (u, v) = crate::text::baseline_to_local(text, x, y);
                        crate::face::local_to_world(&frame, u, v)
                    };
                    let corners =
                        [corner(x0, y0), corner(x1, y0), corner(x1, y1), corner(x0, y1)];
                    for i in 0..4 {
                        mesh.push_dashed_line_segment(
                            corners[i],
                            corners[(i + 1) % 4],
                            anchor_color,
                            1.2,
                            input.cam,
                            input.viewport,
                            &vp,
                        );
                    }
                    if let Some(handles) = crate::text::wrap_width_handles_local(text) {
                        for (u, v) in handles {
                            mesh.push_point_marker(
                                crate::face::local_to_world(&frame, u, v),
                                anchor_color,
                                6.5,
                                input.cam,
                                input.viewport,
                                &vp,
                            );
                        }
                    }
                }
            }
        }

        // Draggable tangent-handle markers for curved lines in the active sketch (#54): a
        // dashed guide from each endpoint to its handle, plus a disc at the handle itself.
        if let Some(session) = input.sketch_session {
            mesh.set_index_layer(MeshIndexLayer::Gizmo);
            // Handles draw in the sketch blue; a hovered/dragged/selected handle turns
            // the gold pick-highlight color and grows a little (#472).
            let handle_color = input.palette.rect_line;
            let highlight_color = crate::construction::PICK_HOVER_RGBA;
            for (li, line) in input.doc.lines.iter().enumerate() {
                if line.deleted || line.sketch != session.sketch {
                    continue;
                }
                let Some([c0, c1]) = line.bezier else {
                    continue;
                };
                // Only show a curve's tangent handles when it's relevant (#550): the curve or one
                // of its endpoints is selected or hovered, or a handle is being manipulated —
                // otherwise the handles clutter and obscure the curve.
                if !bezier_handles_relevant(
                    li,
                    input.selection,
                    &input.hover_highlight,
                    &input.highlighted_bezier_handles,
                ) {
                    continue;
                }
                let Some(frame) = sketch_geometry_frame(input.doc, line.sketch) else {
                    continue;
                };
                let p0 = crate::face::local_to_world(&frame, line.x0, line.y0);
                let p1 = crate::face::local_to_world(&frame, line.x1, line.y1);
                let h0 = crate::face::local_to_world(&frame, c0.0, c0.1);
                let h1 = crate::face::local_to_world(&frame, c1.0, c1.1);
                mesh.push_dashed_line_segment(p0, h0, handle_color, 1.5, input.cam, input.viewport, &vp);
                mesh.push_dashed_line_segment(p1, h1, handle_color, 1.5, input.cam, input.viewport, &vp);
                for (near_start, h) in [(true, h0), (false, h1)] {
                    let hot = input.highlighted_bezier_handles.contains(&(li, near_start));
                    mesh.push_point_marker(
                        h,
                        if hot { highlight_color } else { handle_color },
                        if hot { 6.5 } else { 5.0 },
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
            }
        }

        mesh.set_index_layer(MeshIndexLayer::Overlay);
        for (ci, circle) in input.doc.circles.iter().enumerate() {
            if !circle_alive(input.doc, ci)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Circle(ci))
            {
                continue;
            }
            let dim = input.sketch_session.is_some_and(|s| {
                !sketch_circle_is_active(input.doc, s, ci, circle.sketch)
            });
            let element = SceneElement::Circle(ci);
            mesh.push_circle_strokes(
                input.doc,
                circle,
                ci,
                input.cam,
                input.viewport,
                &vp,
                health_tint_color(
                    sketch_color(input.palette.rect_line, dim),
                    input.document_health.element_status(element.clone()),
                ),
                health_tint_color(
                    sketch_color(input.palette.construction, dim),
                    input.document_health.element_status(element),
                ),
            );
        }

        // Origin marker (#189): a distinct dot at the active sketch's own origin so it's
        // visible as a snappable/constrainable point — the axes cross here, but there is
        // otherwise no point to aim at, so users couldn't tell the origin was selectable.
        if let Some(session) = input.sketch_session {
            if let Some(frame) = sketch_geometry_frame(input.doc, session.sketch) {
                // Highlight (bigger, in the selection color) when the origin is selected (#189),
                // or in the hover color when hovered (#240) — the hover-highlight `Element(Origin)`
                // path can't draw it (it lacks the sketch frame), so it's handled here.
                let selected = input.selection.is_selected(SceneElement::Origin);
                let hovered = matches!(
                    input.hover_highlight,
                    Some(ViewportHoverHighlight::Element(SceneElement::Origin))
                );
                let (color, size) = if selected {
                    (input.palette.dim_edge_highlight, 8.0)
                } else if hovered {
                    (input.hover_color, 7.0)
                } else {
                    (WIREFRAME_LINE_COLOR, 5.0)
                };
                mesh.push_point_marker(frame.origin, color, size, input.cam, input.viewport, &vp);

                // A selected origin axis highlights along its direction (#189) — push_selection
                // can't draw it (it lacks the sketch frame), so it's handled here. The half-length
                // tracks the visible viewport (scaled by zoom) so the highlight always spans the
                // view without using an absolute extent large enough to fall outside clip space.
                use crate::model::{ConstraintLine, SketchAxis};
                let aspect =
                    (input.viewport.width() / input.viewport.height().max(1.0)).max(0.01);
                let (half_w, half_h) = input.cam.viewport_half_extents(aspect);
                let axis_hl = half_w.hypot(half_h) * 2.5;
                for (axis, dir) in [
                    (SketchAxis::X, frame.u_axis),
                    (SketchAxis::Y, frame.v_axis),
                ] {
                    if input
                        .selection
                        .is_selected(SceneElement::FaceEdge(ConstraintLine::OriginAxis(axis)))
                    {
                        mesh.push_line_segment(
                            frame.origin - dir * axis_hl,
                            frame.origin + dir * axis_hl,
                            input.palette.dim_edge_highlight,
                            3.0,
                            input.cam,
                            input.viewport,
                            &vp,
                        );
                    }
                }
            }
        }

        mesh.push_selection(
            input.doc,
            input.document_health,
            input.selection,
            &body_meshes,
            input.cam,
            input.viewport,
            &vp,
            input.palette.dim_edge_highlight,
        );

        // Destructive (side-B / cut) bodies read by their red translucent fill alone
        // (#455) — the outline aura is gone.

        if let Some(graphics) = input.constraint_graphics {
            if !graphics.is_empty() {
                mesh.push_constraint_connectors(
                    input.selection,
                    input.document_health,
                    graphics,
                    input
                        .constraint_connector_color
                        .unwrap_or(input.palette.dim_edge_highlight),
                    input.cam,
                    input.viewport,
                    &vp,
                );
            }
        }

        if let Some(face) = input.active_sketch_face.clone() {
            mesh.push_face_highlight(
                input.doc,
                face,
                input.palette.dim_edge_highlight,
                input.cam,
            );
        }

        if let Some(rect) = input.preview_rect.as_ref() {
            mesh.push_preview_rect(
                rect,
                input.cam,
                input.viewport,
                &vp,
                input.palette.preview,
                input.palette.construction,
            );
        }
        if let Some(line) = input.preview_line.as_ref() {
            let color = if line.construction {
                input.palette.construction
            } else {
                input.palette.preview
            };
            if let Some((a, b)) = line_world_endpoints(input.doc, line) {
                if line.construction {
                    mesh.push_dashed_line_segment(
                        a,
                        b,
                        color,
                        2.0,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                } else {
                    mesh.push_line_segment(a, b, color, 2.0, input.cam, input.viewport, &vp);
                }
            }
        }
        if let Some(circle) = input.preview_circle.as_ref() {
            let solid = if circle.construction {
                input.palette.construction
            } else {
                input.palette.preview
            };
            mesh.push_circle(
                input.doc,
                circle,
                usize::MAX,
                input.cam,
                input.viewport,
                &vp,
                solid,
                input.palette.construction,
                PREVIEW_FILL_DEPTH_BIAS,
            );
        }
        // In-sketch repeat ghost (#232): dashed copies of the picked entities at each offset.
        for &(a, b) in &input.sketch_repeat_ghost {
            mesh.push_dashed_line_segment(
                a,
                b,
                input.palette.preview,
                1.5,
                input.cam,
                input.viewport,
                &vp,
            );
        }
        // In-sketch mirror reflection preview (#542): solid preview-coloured lines like the
        // repeat/extrude/revolve previews, dashed only when the reflected source is construction.
        for &(a, b, dashed) in &input.sketch_mirror_ghost {
            if dashed {
                mesh.push_dashed_line_segment(
                    a, b, input.palette.preview, 1.5, input.cam, input.viewport, &vp,
                );
            } else {
                mesh.push_line_segment(
                    a, b, input.palette.preview, 1.5, input.cam, input.viewport, &vp,
                );
            }
        }
        if let Some(preview) = input.plane_preview.as_ref() {
            mesh.push_plane_creation_preview(
                preview,
                input.palette.preview,
                input.palette.dim_edge_highlight,
                input.cam,
                input.viewport,
                &vp,
            );
        }
        // Live chamfer/fillet corner preview (#76): a single polyline through the treated
        // corner, recomputed every frame from the live gizmo amount.
        if let Some(preview) = input.vertex_treatment_preview.as_ref() {
            mesh.push_polyline_segment(
                &preview.points,
                input.palette.preview,
                2.0,
                input.cam,
                input.viewport,
                &vp,
            );
        }

        // Gizmos go in the depth-disabled Gizmo layer so handles stay visible even when
        // a body is in front of them (#36).
        mesh.set_index_layer(MeshIndexLayer::Gizmo);
        if let Some(gizmo) = input.plane_gizmo.as_ref() {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            mesh.push_plane_gizmo(gizmo, input.cam, input.viewport, &vp, &project);
        }

        if let Some(gizmo) = input.extrude_gizmo.as_ref() {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            mesh.push_offset_gizmo(
                gizmo.origin,
                gizmo.normal,
                gizmo.offset,
                gizmo.color,
                gizmo.hovered,
                input.cam,
                input.viewport,
                &vp,
                &project,
            );
        }
        if let Some(gizmo) = input.vertex_treatment_gizmo.as_ref() {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            mesh.push_offset_gizmo(
                gizmo.origin,
                gizmo.normal,
                gizmo.offset,
                gizmo.color,
                gizmo.hovered,
                input.cam,
                input.viewport,
                &vp,
                &project,
            );
        }
        for gizmo in &input.move_gizmos {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            mesh.push_offset_gizmo(
                gizmo.origin,
                gizmo.normal,
                gizmo.offset,
                gizmo.color,
                gizmo.hovered,
                input.cam,
                input.viewport,
                &vp,
                &project,
            );
        }
        if let Some(ring) = input.move_rotation_gizmo.as_ref() {
            let points = ring_points(ring.center, ring.axis, ring.radius, 64);
            let width = if ring.hovered { 4.0 } else { 2.5 };
            mesh.push_polyline_segment(&points, ring.color, width, input.cam, input.viewport, &vp);
        }
        if let Some(arc) = input.revolve_arc_gizmo.as_ref() {
            // The swept arc from 0° to the current angle, plus a push/pull disc handle at its
            // far end (#262).
            let points = revolve_arc_points(
                arc.center,
                arc.axis,
                arc.zero_dir,
                arc.radius,
                arc.angle_deg,
                64,
            );
            let width = if arc.hovered { 4.0 } else { 2.5 };
            mesh.push_polyline_segment(&points, arc.color, width, input.cam, input.viewport, &vp);
            if let Some(&handle) = points.last() {
                let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
                if arc.hovered {
                    push_gizmo_handle_hover(
                        &mut mesh,
                        handle,
                        GIZMO_HANDLE_HOVER_RGBA,
                        input.cam,
                        input.viewport,
                        &vp,
                        &project,
                    );
                } else {
                    push_gizmo_handle(
                        &mut mesh,
                        handle,
                        arc.color,
                        input.cam,
                        input.viewport,
                        &vp,
                        &project,
                    );
                }
            }
        }
        mesh.set_index_layer(MeshIndexLayer::Overlay);

        if let Some(hover) = input.hover_highlight.as_ref() {
            mesh.push_hover_highlight(
                input.doc,
                hover,
                input.hover_color,
                &body_meshes,
                input.cam,
                input.viewport,
                &vp,
            );
        }
        // Every member of a hovered exploder group loupe lights up together (#559).
        for kind in &input.extra_pick_highlights {
            mesh.push_hover_highlight(
                input.doc,
                &ViewportHoverHighlight::PickTarget(kind.clone()),
                input.hover_color,
                &body_meshes,
                input.cam,
                input.viewport,
                &vp,
            );
        }

        if input.dim_label_view.is_some() {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            for label in input.dimension_labels {
                if label.draw_dimension_lines {
                    push_linear_dimension_world(
                        &mut mesh,
                        &label.world_geom,
                        label.color,
                        input.cam,
                        input.viewport,
                        &vp,
                        &project,
                    );
                }
            }
        }
        drop(mesh);

        for label in input.dimension_labels {
            if !label.text_vertices.is_empty() {
                let base = scene.text_vertices.len() as u32;
                scene.text_vertices.extend_from_slice(&label.text_vertices);
                scene
                    .text_indices
                    .extend(label.text_indices.iter().map(|i| i + base));
            }
        }

        scene
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MeshIndexLayer {
    #[default]
    Base,
    /// Committed coplanar sketch-shape fills. Drawn with a stencil mask so each
    /// pixel is painted exactly once, preventing translucent overlap regions from
    /// being alpha-blended twice (which made overlaps render darker — #3).
    SketchFill,
    PlaneFill,
    Overlay,
    /// Manipulation gizmos, drawn last with the depth test disabled (#36).
    Gizmo,
    /// Body edge-wireframe overlay, drawn depth-test-disabled like [`Self::Gizmo`] (#33).
    Wireframe,
}

pub(crate) struct SceneMesh<'a> {
    scene: &'a mut ViewportScene,
    index_layer: MeshIndexLayer,
}

impl<'a> SceneMesh<'a> {
    fn new(scene: &'a mut ViewportScene) -> Self {
        Self {
            scene,
            index_layer: MeshIndexLayer::Base,
        }
    }

    fn set_index_layer(&mut self, layer: MeshIndexLayer) {
        self.index_layer = layer;
    }

    fn indices_mut(&mut self) -> &mut Vec<u32> {
        match self.index_layer {
            MeshIndexLayer::Base => &mut self.scene.indices,
            MeshIndexLayer::SketchFill => &mut self.scene.sketch_fill_indices,
            MeshIndexLayer::PlaneFill => &mut self.scene.plane_fill_indices,
            MeshIndexLayer::Overlay => &mut self.scene.overlay_indices,
            MeshIndexLayer::Gizmo => &mut self.scene.gizmo_indices,
            MeshIndexLayer::Wireframe => &mut self.scene.wireframe_indices,
        }
    }

    fn push_vertex(&mut self, position: Vec3, color: Color32) {
        self.scene.vertices.push(GpuVertex {
            position: position.to_array(),
            color: color32_to_gpu(color),
        });
    }

    fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: Color32) {
        let base = self.scene.vertices.len() as u32;
        self.push_vertex(a, color);
        self.push_vertex(b, color);
        self.push_vertex(c, color);
        self.indices_mut()
            .extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn push_quad_fill(&mut self, fill_corners: [Vec3; 4], fill: Color32) {
        self.push_triangle(fill_corners[0], fill_corners[1], fill_corners[2], fill);
        self.push_triangle(fill_corners[0], fill_corners[2], fill_corners[3], fill);
    }

    /// Push a solid mesh with flat (per-triangle) two-sided shading. `cap_plane`, when the
    /// extrusion targets a face/plane, nudges triangles lying exactly on that plane (the top
    /// cap) toward the camera by a hair so they don't z-fight with the target plane's own
    /// fill at grazing angles (#29) — geometry used for export/measurement is untouched,
    /// this only biases what gets rasterized.
    fn push_solid(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        base: Color32,
        cam: &Camera,
        cap_plane: Option<(Vec3, Vec3)>,
    ) {
        let light = Vec3::new(0.35, 0.45, 0.82).normalize_or_zero();
        let eye = cam.eye();
        for tri in &solid.triangles {
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            // Two-sided: faces are lit regardless of winding direction.
            let shade = 0.4 + 0.6 * normal.dot(light).abs();
            let verts = match cap_plane {
                Some((origin, plane_normal)) if triangle_on_plane(tri, origin, plane_normal) => [
                    offset_toward_camera(tri[0], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                    offset_toward_camera(tri[1], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                    offset_toward_camera(tri[2], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                ],
                _ => *tri,
            };
            self.push_triangle(verts[0], verts[1], verts[2], scale_color(base, shade));
        }
    }

    /// Push a solid mesh with flat (per-triangle) ambient + diffuse + specular shading — a
    /// matte/satin "painted object" look, for `ShadingMode::Realistic` (#83). Unlike `push_solid`
    /// (a single Lambert-ish term), this adds a camera-dependent Blinn-Phong specular highlight
    /// via [`realistic_shade`]. Still flat-shaded per triangle (no shared vertex normals exist
    /// on `SolidMesh`), so it reads as faceted rather than smoothly lit — a known, accepted
    /// limitation given this app's flat-shaded mesh architecture. No materials/textures yet:
    /// every body uses the same fixed gloss.
    fn push_solid_realistic(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        base: Color32,
        cam: &Camera,
        cap_plane: Option<(Vec3, Vec3)>,
    ) {
        let light = Vec3::new(0.35, 0.45, 0.82).normalize_or_zero();
        let eye = cam.eye();
        for tri in &solid.triangles {
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            let centroid = (tri[0] + tri[1] + tri[2]) / 3.0;
            let view = (eye - centroid).normalize_or_zero();
            let color = realistic_shade(base, normal, light, view);
            let verts = match cap_plane {
                Some((origin, plane_normal)) if triangle_on_plane(tri, origin, plane_normal) => [
                    offset_toward_camera(tri[0], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                    offset_toward_camera(tri[1], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                    offset_toward_camera(tri[2], plane_normal, eye, SOLID_CAP_DEPTH_BIAS),
                ],
                _ => *tri,
            };
            self.push_triangle(verts[0], verts[1], verts[2], color);
        }
    }

    /// Push a solid mesh into the translucent (plane-fill) layer with two-sided
    /// shading and the given opacity, so it blends over opaque geometry.
    fn push_solid_translucent(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        base: Color32,
        opacity: f32,
    ) {
        let light = Vec3::new(0.35, 0.45, 0.82).normalize_or_zero();
        let prev = self.index_layer;
        self.set_index_layer(MeshIndexLayer::PlaneFill);
        for tri in &solid.triangles {
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            let shade = 0.4 + 0.6 * normal.dot(light).abs();
            self.push_triangle(
                tri[0],
                tri[1],
                tri[2],
                fill_color(scale_color(base, shade), opacity),
            );
        }
        self.set_index_layer(prev);
    }

    /// Push a solid mesh's unique edges as camera-facing line-quads into the
    /// [`MeshIndexLayer::Wireframe`] layer (#33). Used for `ShadingMode::Wireframe` (in
    /// place of the fill) and `ShadingMode::SolidWireframe` (as an overlay on top of the
    /// fill) — see [`solid_mesh_unique_edges`] for how shared edges are deduplicated.
    fn push_solid_wireframe(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let prev = self.index_layer;
        self.set_index_layer(MeshIndexLayer::Wireframe);
        for (a, b) in solid_mesh_unique_edges(solid) {
            self.push_line_segment(a, b, color, WIREFRAME_LINE_WIDTH_PX, cam, viewport, view_proj);
        }
        // Smooth-surface silhouettes (#158): a cylinder's sides are invisible without the
        // view-tangent lines (its wall seams are all sub-crease and rightly dropped above).
        for (a, b) in solid_mesh_smooth_silhouette_edges(solid, cam.eye()) {
            self.push_line_segment(a, b, color, WIREFRAME_LINE_WIDTH_PX, cam, viewport, view_proj);
        }
        self.set_index_layer(prev);
    }

    #[allow(dead_code)]
    fn push_quad(
        &mut self,
        corners: [Vec3; 4],
        fill_corners: [Vec3; 4],
        fill: Color32,
        stroke: Color32,
        stroke_width: f32,
        stroke_dashed: bool,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        self.push_quad_fill(fill_corners, fill);
        for (a, b) in [
            (corners[0], corners[1]),
            (corners[1], corners[2]),
            (corners[2], corners[3]),
            (corners[3], corners[0]),
        ] {
            if stroke_dashed {
                self.push_dashed_line_segment(a, b, stroke, stroke_width, cam, viewport, view_proj);
            } else {
                self.push_line_segment(a, b, stroke, stroke_width, cam, viewport, view_proj);
            }
        }
    }

    pub(crate) fn push_dashed_line_segment(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        width_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        for (wa, wb) in dashed_world_segments(
            a,
            b,
            CONSTRUCTION_DASH_LENGTH_PX,
            CONSTRUCTION_DASH_GAP_PX,
            cam,
            viewport,
            view_proj,
        ) {
            self.push_line_segment(wa, wb, color, width_px, cam, viewport, view_proj);
        }
    }

    pub(crate) fn push_line_segment(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        width_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        self.push_line_segment_with_bias(
            a,
            b,
            color,
            width_px,
            cam,
            viewport,
            view_proj,
            STROKE_DEPTH_BIAS,
        );
    }

    /// Draws a connected polyline (e.g. a sampled bezier curve) as a chain of solid segments.
    pub(crate) fn push_polyline_segment(
        &mut self,
        points: &[Vec3],
        color: Color32,
        width_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        for pair in points.windows(2) {
            self.push_line_segment(pair[0], pair[1], color, width_px, cam, viewport, view_proj);
        }
    }

    /// Draws a connected polyline (e.g. a sampled bezier curve) as a chain of dashed segments.
    pub(crate) fn push_dashed_polyline_segment(
        &mut self,
        points: &[Vec3],
        color: Color32,
        width_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        for pair in points.windows(2) {
            self.push_dashed_line_segment(pair[0], pair[1], color, width_px, cam, viewport, view_proj);
        }
    }

    fn push_point_marker(
        &mut self,
        world: Vec3,
        color: Color32,
        radius_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let project = |p: Vec3| cam.project(p, viewport, view_proj);
        push_screen_disc(
            self,
            world,
            radius_px,
            color,
            cam,
            viewport,
            view_proj,
            &project,
        );
    }

    pub(crate) fn push_line_segment_with_bias(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        width_px: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        depth_bias: f32,
    ) {
        let (a, b) = offset_segment_toward_camera(a, b, cam.eye(), depth_bias);
        let Some(quad) = line_screen_quad(a, b, width_px, cam, viewport, view_proj) else {
            return;
        };
        let base = self.scene.vertices.len() as u32;
        let gpu = color32_to_gpu(color);
        for p in quad {
            self.scene.vertices.push(GpuVertex {
                position: p.to_array(),
                color: gpu,
            });
        }
        self.indices_mut()
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn push_ground(
        &mut self,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        dim: bool,
        palette: &ViewportPalette,
        unit: LengthUnit,
    ) {
        // Keep the grid and origin axes a usable on-screen size when zoomed out for large parts
        // (#353): measure how many pixels one world-mm spans at the origin, then scale the grid
        // extent/step and axis length so the axes are always at least ~MIN_AXIS_PX pixels and the
        // grid stays a readable reference instead of collapsing to a dot.
        const MIN_AXIS_PX: f32 = 90.0;
        // The fine subdivision level appears once its lines are this far apart on
        // screen, and fades up to full strength by FINE_FULL_PX (#464) — a continuous
        // ramp, so zooming never pops lines in or out.
        const FINE_MIN_PX: f32 = 8.0;
        const FINE_FULL_PX: f32 = 32.0;
        // Pixels per world-mm along the camera's screen-horizontal, measured at the origin.
        let ppw = {
            let fwd = (cam.target - cam.eye()).normalize_or_zero();
            let mut right = fwd.cross(Vec3::Z);
            if right.length_squared() < 1e-6 {
                right = Vec3::X;
            }
            let right = right.normalize();
            match (
                cam.project(Vec3::ZERO, viewport, view_proj),
                cam.project(right * 10.0, viewport, view_proj),
            ) {
                (Some(a), Some(b)) => ((b - a).length() / 10.0).max(1e-6),
                _ => 1.0,
            }
        };
        // Two grid levels from the document's unit ladder (#464): heavy lines at the
        // coarse step, lighter subdividing lines at the fine step between them.
        let (fine_step, coarse_step) = grid_steps_for_unit(unit, FINE_MIN_PX / ppw);
        let subdiv = ((coarse_step / fine_step).round() as i64).max(2);
        let axis_len = (MIN_AXIS_PX / ppw).max(GRID_EXTENT);
        // The grid covers the **visible ground footprint** (#467), not a fixed box around
        // the origin: cast a 3×3 fan of screen points onto z = 0 and bound their hits.
        // An origin-centered box failed two ways — panning away from the origin left the
        // whole view past the grid's edge (no grid at all), and when zoomed in its lines
        // stretched so far that one endpoint fell behind the camera, where `project()`
        // culls the entire segment (grid showing lines in only one direction, or none,
        // depending on the orbit angle). Footprint-bounded lines keep both endpoints
        // near the frustum. Above-horizon rays miss the plane; the reach clamp bounds
        // the hits that remain so a horizon view gets a deep-but-finite grid.
        let eye = cam.eye();
        let anchor = glam::Vec2::new(cam.target.x, cam.target.y);
        let reach = ((eye - cam.target).length() * 8.0).max(GRID_EXTENT);
        let mut lo = anchor;
        let mut hi = anchor;
        for sy in 0..3 {
            for sx in 0..3 {
                let screen = egui::pos2(
                    viewport.min.x + viewport.width() * sx as f32 / 2.0,
                    viewport.min.y + viewport.height() * sy as f32 / 2.0,
                );
                if let Some(g) = cam.ground_point(screen, viewport, view_proj) {
                    let g = anchor + (glam::Vec2::new(g.x, g.y) - anchor).clamp_length_max(reach);
                    lo = lo.min(g);
                    hi = hi.max(g);
                }
            }
        }
        lo -= glam::Vec2::splat(coarse_step);
        hi += glam::Vec2::splat(coarse_step);
        // Solid ground (#159): one filled plane in the grid's grey (darkened so bodies and
        // sketches still read against it), pushed slightly away from the camera with the
        // same bias the grid lines use so body bottoms resting on z = 0 never z-fight it.
        // The axis lines below still draw on top for orientation.
        if cam.ground_display() == crate::camera::GroundDisplay::Solid {
            let fill = sketch_ground_color(scale_color(palette.grid, 0.55), dim);
            let corners = [
                Vec3::new(lo.x, lo.y, 0.0),
                Vec3::new(hi.x, lo.y, 0.0),
                Vec3::new(hi.x, hi.y, 0.0),
                Vec3::new(lo.x, hi.y, 0.0),
            ];
            let lifted = offset_corners_toward_camera(corners, Vec3::Z, eye, GRID_DEPTH_BIAS);
            self.push_quad_fill(lifted, fill);
        } else {
            // Index range of step-multiples covering [lo, hi], hard-capped to `max_n`
            // lines each side of the anchor. Lines stay on fixed world multiples of
            // their step, so zooming or panning never slides them.
            let line_range = |lo: f32, hi: f32, step: f32, center: f32, max_n: i64| {
                let a = (lo / step).floor() as i64;
                let b = (hi / step).ceil() as i64;
                let c = (center / step).round() as i64;
                (a.max(c - max_n), b.min(c + max_n))
            };
            let mut vertical = |x: f32, color: Color32| {
                self.push_line_segment_with_bias(
                    Vec3::new(x, lo.y, 0.0),
                    Vec3::new(x, hi.y, 0.0),
                    color,
                    1.0,
                    cam,
                    viewport,
                    view_proj,
                    GRID_DEPTH_BIAS,
                );
            };
            let heavy = sketch_ground_color(palette.grid, dim);
            let axis_heavy = sketch_ground_color(palette.grid_axis, dim);
            let (hx0, hx1) = line_range(lo.x, hi.x, coarse_step, anchor.x, 400);
            for i in hx0..=hx1 {
                vertical(i as f32 * coarse_step, if i == 0 { axis_heavy } else { heavy });
            }
            // Fine subdivisions, fading in with zoom.
            let fade = (fine_step * ppw - FINE_MIN_PX) / (FINE_FULL_PX - FINE_MIN_PX);
            let fine_color = mix_color(
                palette.background,
                sketch_ground_color(palette.grid, dim),
                0.7 * fade.clamp(0.0, 1.0),
            );
            if fade > 0.01 {
                let (fx0, fx1) = line_range(lo.x, hi.x, fine_step, anchor.x, 600);
                for i in fx0..=fx1 {
                    if i % subdiv != 0 {
                        vertical(i as f32 * fine_step, fine_color);
                    }
                }
            }
            let mut horizontal = |y: f32, color: Color32| {
                self.push_line_segment_with_bias(
                    Vec3::new(lo.x, y, 0.0),
                    Vec3::new(hi.x, y, 0.0),
                    color,
                    1.0,
                    cam,
                    viewport,
                    view_proj,
                    GRID_DEPTH_BIAS,
                );
            };
            let (hy0, hy1) = line_range(lo.y, hi.y, coarse_step, anchor.y, 400);
            for j in hy0..=hy1 {
                horizontal(j as f32 * coarse_step, if j == 0 { axis_heavy } else { heavy });
            }
            if fade > 0.01 {
                let (fy0, fy1) = line_range(lo.y, hi.y, fine_step, anchor.y, 600);
                for j in fy0..=fy1 {
                    if j % subdiv != 0 {
                        horizontal(j as f32 * fine_step, fine_color);
                    }
                }
            }
        }
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(axis_len, 0.0, 0.0),
            sketch_ground_color(palette.x_axis, dim),
            2.0,
            cam,
            viewport,
            view_proj,
            GRID_DEPTH_BIAS,
        );
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(0.0, axis_len, 0.0),
            sketch_ground_color(palette.y_axis, dim),
            2.0,
            cam,
            viewport,
            view_proj,
            GRID_DEPTH_BIAS,
        );
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, axis_len),
            sketch_ground_color(palette.z_axis, dim),
            2.0,
            cam,
            viewport,
            view_proj,
            GRID_DEPTH_BIAS,
        );
    }

    /// Draw the rectangle tool's live drag-preview (translucent quad + closed edge strokes).
    fn push_preview_rect(
        &mut self,
        preview: &PreviewRect,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        solid: Color32,
        construction: Color32,
    ) {
        let corners = preview.corners;
        let normal = (corners[1] - corners[0])
            .cross(corners[3] - corners[0])
            .normalize_or_zero();
        let fill_corners =
            offset_corners_toward_camera(corners, normal, cam.eye(), PREVIEW_FILL_DEPTH_BIAS);
        let stroke = if preview.construction { construction } else { solid };
        let fill = if preview.construction {
            fill_color(construction, CONSTRUCTION_FILL_OPACITY)
        } else {
            fill_color(solid, SOLID_FILL_OPACITY)
        };
        self.push_quad_fill(fill_corners, fill);
        for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            if preview.construction {
                self.push_dashed_line_segment(
                    corners[i], corners[j], stroke, 1.5, cam, viewport, view_proj,
                );
            } else {
                self.push_line_segment(corners[i], corners[j], stroke, 1.5, cam, viewport, view_proj);
            }
        }
    }

    fn push_circle_fill(
        &mut self,
        doc: &Document,
        circle: &Circle,
        _index: usize,
        cam: &Camera,
        solid: Color32,
        construction: Color32,
        fill_depth_bias: f32,
    ) {
        let Some(perimeter) = circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS) else {
            return;
        };
        let frame = sketch_geometry_frame(doc, circle.sketch).expect("circle sketch frame");
        let eye = cam.eye();
        let center = offset_toward_camera(
            crate::face::local_to_world(&frame, circle.cx, circle.cy),
            frame.normal,
            eye,
            fill_depth_bias,
        );
        let fill = if circle.construction {
            fill_color(construction, CONSTRUCTION_FILL_OPACITY)
        } else {
            fill_color(solid, SOLID_FILL_OPACITY)
        };
        for window in perimeter.windows(2) {
            let a = offset_toward_camera(window[0], frame.normal, eye, fill_depth_bias);
            let b = offset_toward_camera(window[1], frame.normal, eye, fill_depth_bias);
            self.push_triangle(center, a, b, fill);
        }
    }

    /// Fill for a closed loop of plain lines (#66), ear-clipped for concave boundaries.
    fn push_polygon_fill(
        &mut self,
        profile: &[Vec3],
        normal: Vec3,
        cam: &Camera,
        solid: Color32,
        construction: Color32,
        all_construction: bool,
        fill_depth_bias: f32,
    ) {
        if profile.len() < 3 {
            return;
        }
        let eye = cam.eye();
        let fill = if all_construction {
            fill_color(construction, CONSTRUCTION_FILL_OPACITY)
        } else {
            fill_color(solid, SOLID_FILL_OPACITY)
        };
        let lifted: Vec<Vec3> = profile
            .iter()
            .map(|&p| offset_toward_camera(p, normal, eye, fill_depth_bias))
            .collect();
        for [a, b, c] in crate::polygon::triangulate_planar(profile, normal) {
            self.push_triangle(lifted[a], lifted[b], lifted[c], fill);
        }
    }

    fn push_circle_strokes(
        &mut self,
        doc: &Document,
        circle: &Circle,
        _index: usize,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        solid: Color32,
        construction: Color32,
    ) {
        let Some(perimeter) = circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS) else {
            return;
        };
        let stroke = if circle.construction {
            construction
        } else {
            solid
        };
        for window in perimeter.windows(2) {
            if circle.construction {
                self.push_dashed_line_segment(
                    window[0],
                    window[1],
                    stroke,
                    1.5,
                    cam,
                    viewport,
                    view_proj,
                );
            } else {
                self.push_line_segment(window[0], window[1], stroke, 1.5, cam, viewport, view_proj);
            }
        }
        // A small marker at the center so it's visible and clickable as a constraint point
        // (#198) — a circle otherwise has no drawn geometry at its center to aim at.
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            self.push_point_marker(center, stroke, 4.0, cam, viewport, view_proj);
        }
    }

    fn push_circle(
        &mut self,
        doc: &Document,
        circle: &Circle,
        index: usize,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        solid: Color32,
        construction: Color32,
        fill_depth_bias: f32,
    ) {
        self.push_circle_fill(doc, circle, index, cam, solid, construction, fill_depth_bias);
        self.push_circle_strokes(doc, circle, index, cam, viewport, view_proj, solid, construction);
    }

    fn push_plane_outline(
        &mut self,
        plane: &ConstructionPlane,
        color: Color32,
        stroke_width: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
        self.push_quad_outline(corners, color, stroke_width, cam, viewport, view_proj);
    }

    fn push_quad_outline(
        &mut self,
        corners: [Vec3; 4],
        color: Color32,
        stroke_width: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        for (a, b) in [
            (corners[0], corners[1]),
            (corners[1], corners[2]),
            (corners[2], corners[3]),
            (corners[3], corners[0]),
        ] {
            self.push_line_segment(a, b, color, stroke_width, cam, viewport, view_proj);
        }
    }

    fn push_plane_creation_preview(
        &mut self,
        preview: &ViewportPlanePreview,
        preview_color: Color32,
        dim_edge_color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        const PREVIEW_STROKE: f32 = 2.0;
        self.push_plane_outline(
            &preview.plane,
            preview_color,
            PREVIEW_STROKE,
            cam,
            viewport,
            view_proj,
        );
        if preview.dim_outline {
            self.push_plane_outline(
                &preview.plane,
                dim_edge_color,
                PREVIEW_STROKE,
                cam,
                viewport,
                view_proj,
            );
        }
        let Some(dependents) = preview.dependents.as_ref() else {
            return;
        };
        for (_, plane) in &dependents.planes {
            self.push_plane_outline(plane, preview_color, PREVIEW_STROKE, cam, viewport, view_proj);
        }
        for &(a, b) in &dependents.lines {
            self.push_line_segment(a, b, preview_color, PREVIEW_STROKE, cam, viewport, view_proj);
        }
    }

    fn push_plane(
        &mut self,
        plane: &ConstructionPlane,
        index: usize,
        color: Color32,
        opacity: f32,
        cam: &Camera,
    ) {
        let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
        let fill_bias = plane_fill_depth_bias(index);
        let eye = cam.eye();
        let fill_corners = offset_corners_toward_camera(corners, plane.normal, eye, fill_bias);
        let fill = fill_color(color, opacity);
        self.push_quad_fill(fill_corners, fill);
    }

    fn push_selection(
        &mut self,
        doc: &Document,
        health: &DocumentHealth,
        selection: &SceneSelection,
        _body_meshes: &[Option<crate::extrude::SolidMesh>],
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        base_color: Color32,
    ) {
        if selection.is_empty() {
            return;
        }

        for element in selection.iter() {
            let color = health_tint_color(base_color, health.element_status(element.clone()));
            let dashed = selection_highlight_dashed(doc, element.clone()) == Some(true);
            match element {
                SceneElement::Line(index) => {
                    if !line_alive(doc, index) {
                        continue;
                    }
                    if let Some(line) = doc.lines.get(index) {
                        if let Some(points) = line_world_polyline(doc, line) {
                            if dashed {
                                self.push_dashed_polyline_segment(
                                    &points, color, 3.0, cam, viewport, view_proj,
                                );
                            } else {
                                self.push_polyline_segment(&points, color, 3.0, cam, viewport, view_proj);
                            }
                        }
                    }
                }
                SceneElement::Circle(index) => {
                    if !circle_alive(doc, index) {
                        continue;
                    }
                    if let Some(circle) = doc.circles.get(index) {
                        if let Some(perimeter) =
                            circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS)
                        {
                            for window in perimeter.windows(2) {
                                if dashed {
                                    self.push_dashed_line_segment(
                                        window[0],
                                        window[1],
                                        color,
                                        3.0,
                                        cam,
                                        viewport,
                                        view_proj,
                                    );
                                } else {
                                    self.push_line_segment(
                                        window[0],
                                        window[1],
                                        color,
                                        3.0,
                                        cam,
                                        viewport,
                                        view_proj,
                                    );
                                }
                            }
                        }
                    }
                }
                SceneElement::Constraint(index) => {
                    if !constraint_alive(doc, index) {
                        continue;
                    }
                    if let Some((a, b)) = constraint_segment_endpoints(doc, index) {
                        self.push_line_segment(a, b, color, 3.0, cam, viewport, view_proj);
                    }
                }
                SceneElement::Point(point) => {
                    if let Some(world) = crate::construction::point_world_position(doc, point) {
                        self.push_point_marker(world, color, 6.0, cam, viewport, view_proj);
                    }
                }
                // Selected 3D body sub-elements (#156): drawn depth-test-disabled like their
                // hover highlights (#153), so a concave joint can't bury the selection mark.
                SceneElement::BodyEdge { a, b, .. } => {
                    let restore = self.index_layer;
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                    self.push_line_segment(
                        crate::hierarchy::dequantize_body_point(a),
                        crate::hierarchy::dequantize_body_point(b),
                        color,
                        4.0,
                        cam,
                        viewport,
                        view_proj,
                    );
                    self.set_index_layer(restore);
                }
                SceneElement::BodyVertex { p, .. } => {
                    let restore = self.index_layer;
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                    self.push_point_marker(
                        crate::hierarchy::dequantize_body_point(p),
                        color,
                        6.0,
                        cam,
                        viewport,
                        view_proj,
                    );
                    self.set_index_layer(restore);
                }
                // A selected body face (#555/#557): re-find the coplanar-triangle group whose
                // quantized centroid+normal matches the stored key, then fill + stroke it in the
                // selection color, depth-test-disabled like the edge/vertex marks above.
                SceneElement::BodyFace { body, centroid, normal } => {
                    if let Some(solid) = crate::extrude::body_solid_mesh(doc, body) {
                        let q = crate::hierarchy::quantize_body_point;
                        let group = solid_mesh_coplanar_faces(&solid).into_iter().find(|tris| {
                            let count = (tris.len() * 3).max(1) as f32;
                            let c = tris.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
                            let n = (tris[0][1] - tris[0][0])
                                .cross(tris[0][2] - tris[0][0])
                                .normalize_or_zero();
                            q(c) == centroid && q(n) == normal
                        });
                        if let Some(tris) = group {
                            let restore = self.index_layer;
                            self.set_index_layer(MeshIndexLayer::Wireframe);
                            let eye = cam.eye();
                            let n = (tris[0][1] - tris[0][0])
                                .cross(tris[0][2] - tris[0][0])
                                .normalize_or_zero();
                            let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                            let lift =
                                |p: Vec3| offset_toward_camera(p, n, eye, HOVER_FILL_DEPTH_BIAS);
                            for tri in &tris {
                                self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                            }
                            for (a, b) in crate::construction::coplanar_face_boundary(&tris) {
                                self.push_line_segment(a, b, color, 3.0, cam, viewport, view_proj);
                            }
                            self.set_index_layer(restore);
                        }
                    }
                }
                // A selected face edge (#199): highlight the edge segment so selecting it gives
                // feedback. Depth-test-disabled like body edges (#153) — it lies on the body
                // surface, so a plain depth test would z-fight it away.
                SceneElement::FaceEdge(crate::model::ConstraintLine::FaceEdge { face, index }) => {
                    if let Ok((a, b)) =
                        crate::geometric_constraints::face_edge_world(doc, &face, index)
                    {
                        let restore = self.index_layer;
                        self.set_index_layer(MeshIndexLayer::Wireframe);
                        self.push_line_segment(a, b, color, 4.0, cam, viewport, view_proj);
                        self.set_index_layer(restore);
                    }
                }
                // A selected body recolors in the main pass (#455: fill in shaded modes,
                // lines in wireframe) — no outline aura. A selected extrusion recolors
                // just its own solid within the (possibly merged) body.
                SceneElement::Body(_) => {}
                SceneElement::Extrusion(index) => {
                    self.push_sub_body_recolor(doc, index, BODY_SILHOUETTE_COLOR, cam, viewport, view_proj);
                }
                _ => {}
            }
        }
    }

    /// Recolor one extrusion's own solid inside its body (#455, replacing the aura): a
    /// translucent overlay of its mesh in shaded modes, its feature edges in wireframe.
    fn push_sub_body_recolor(
        &mut self,
        doc: &Document,
        extrusion: usize,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let Some(ext) = doc.extrusions.get(extrusion).filter(|e| !e.deleted) else {
            return;
        };
        let Some(mesh) = crate::extrude::extrusion_mesh(doc, ext) else {
            return;
        };
        if matches!(cam.shading_mode(), crate::camera::ShadingMode::Wireframe) {
            self.push_solid_wireframe(&mesh, color, cam, viewport, view_proj);
        } else {
            // The Overlay layer's toward-camera bias keeps this from z-fighting the body.
            let restore = self.index_layer;
            self.set_index_layer(MeshIndexLayer::Overlay);
            self.push_solid_translucent(&mesh, color, 0.45);
            self.set_index_layer(restore);
        }
    }



    fn push_constraint_connectors(
        &mut self,
        selection: &SceneSelection,
        health: &DocumentHealth,
        graphics: &[ConstraintViewportGraphic],
        base_color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        for graphic in graphics {
            let color = constraint_annotation_color(health, graphic.constraint_index, base_color);
            let selected =
                selection.is_selected(SceneElement::Constraint(graphic.constraint_index));
            let width = if selected { 2.5 } else { 1.5 };
            for connector in &graphic.connectors {
                self.push_dashed_line_segment(
                    connector.a,
                    connector.b,
                    color,
                    width,
                    cam,
                    viewport,
                    view_proj,
                );
            }
        }
    }

    fn push_face_highlight(
        &mut self,
        doc: &Document,
        face: FaceId,
        color: Color32,
        cam: &Camera,
    ) {
        match face {
            FaceId::ConstructionPlane(index) => {
                if let Some(plane) = doc.construction_planes.get(index) {
                    self.push_construction_plane_hover_fill(plane, index, color, 0.12, cam);
                }
            }
            _ => self.push_sketch_face_hover(doc, face, color, 0.12, cam),
        }
    }

    fn push_construction_plane_hover_fill(
        &mut self,
        plane: &ConstructionPlane,
        index: usize,
        color: Color32,
        fill_multiplier: f32,
        cam: &Camera,
    ) {
        let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
        let bias = plane_fill_depth_bias(index) + HOVER_PLANE_DEPTH_LIFT;
        let fill_corners =
            offset_corners_toward_camera(corners, plane.normal, cam.eye(), bias);
        self.push_quad_fill(fill_corners, color.gamma_multiply(fill_multiplier));
    }

    fn push_sketch_face_hover(
        &mut self,
        doc: &Document,
        face: FaceId,
        color: Color32,
        fill_multiplier: f32,
        cam: &Camera,
    ) {
        let fill = color.gamma_multiply(fill_multiplier);
        let eye = cam.eye();
        match face {
            FaceId::Circle(index) => {
                if let Some(circle) = doc.circles.get(index) {
                    if let Some(perimeter) =
                        circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS)
                    {
                        let frame =
                            sketch_geometry_frame(doc, circle.sketch).expect("circle frame");
                        let lift = |p: Vec3| {
                            offset_toward_camera(p, frame.normal, eye, HOVER_FILL_DEPTH_BIAS)
                        };
                        let center =
                            lift(crate::face::local_to_world(&frame, circle.cx, circle.cy));
                        for window in perimeter.windows(2) {
                            self.push_triangle(center, lift(window[0]), lift(window[1]), fill);
                        }
                    }
                }
            }
            FaceId::Polygon(lines) => {
                if let Some((poly, _)) =
                    crate::extrude::face_profile_world(doc, &crate::model::ExtrudeFace::Polygon(lines))
                {
                    if poly.len() >= 3 {
                        let normal =
                            (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
                        let lift =
                            |p: Vec3| offset_toward_camera(p, normal, eye, HOVER_FILL_DEPTH_BIAS);
                        for i in 1..poly.len() - 1 {
                            self.push_triangle(
                                lift(poly[0]),
                                lift(poly[i]),
                                lift(poly[i + 1]),
                                fill,
                            );
                        }
                    }
                }
            }
            FaceId::ExtrudeCap {
                extrusion,
                profile,
                top,
            } => {
                if let Some(poly) =
                    crate::extrude::cap_polygon_world(doc, extrusion, &profile, top)
                {
                    if poly.len() >= 3 {
                        let normal =
                            (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
                        let lift =
                            |p: Vec3| offset_toward_camera(p, normal, eye, HOVER_FILL_DEPTH_BIAS);
                        for i in 1..poly.len() - 1 {
                            self.push_triangle(
                                lift(poly[0]),
                                lift(poly[i]),
                                lift(poly[i + 1]),
                                fill,
                            );
                        }
                    }
                }
            }
            FaceId::ExtrudeSide {
                extrusion,
                profile,
                edge,
            } => {
                if let Some(quad) =
                    crate::extrude::side_quad_world(doc, extrusion, &profile, edge as usize)
                {
                    let normal =
                        (quad[1] - quad[0]).cross(quad[2] - quad[0]).normalize_or_zero();
                    let lift =
                        |p: Vec3| offset_toward_camera(p, normal, eye, HOVER_FILL_DEPTH_BIAS);
                    self.push_triangle(lift(quad[0]), lift(quad[1]), lift(quad[2]), fill);
                    self.push_triangle(lift(quad[0]), lift(quad[2]), lift(quad[3]), fill);
                }
            }
            FaceId::ConstructionPlane(_) => {}
        }
    }

    fn push_hover_highlight(
        &mut self,
        doc: &Document,
        hover: &ViewportHoverHighlight,
        color: Color32,
        body_meshes: &[Option<crate::extrude::SolidMesh>],
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let project = |w: Vec3| cam.project(w, viewport, view_proj);
        match hover {
            ViewportHoverHighlight::SketchFace(face) => match face {
                FaceId::ConstructionPlane(index) => {
                    if let Some(plane) = doc.construction_planes.get(*index) {
                        self.push_construction_plane_hover_fill(
                            plane,
                            *index,
                            color,
                            FACE_HOVER_FILL_MULTIPLIER,
                            cam,
                        );
                    }
                }
                _ => {
                    self.push_sketch_face_hover(doc, face.clone(), color, FACE_HOVER_FILL_MULTIPLIER, cam);
                    self.push_sketch_face_hover_border(
                        doc,
                        face.clone(),
                        color,
                        2.0,
                        cam,
                        viewport,
                        view_proj,
                    );
                }
            },
            ViewportHoverHighlight::Element(element) => {
                self.push_element_hover(doc, element.clone(), color, body_meshes, cam, viewport, view_proj);
            }
            ViewportHoverHighlight::PickTarget(kind) => {
                // Pick-target highlights draw depth-test-disabled (#153): a hovered edge or
                // vertex in a concave joint would otherwise have a chunk of its highlight
                // buried inside the adjoining faces (the small camera-ward bias can't clear
                // a wall rising beside the edge). Hover means "this is what a click picks" —
                // it must always be fully visible.
                let restore_layer = self.index_layer;
                self.set_index_layer(MeshIndexLayer::Wireframe);
                self.push_pick_target_highlight(
                    doc,
                    kind,
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
                self.set_index_layer(restore_layer);
            }
            ViewportHoverHighlight::ClosedLoop { world_loop } => {
                if world_loop.len() >= 3 {
                    let eye = cam.eye();
                    let normal = (world_loop[1] - world_loop[0])
                        .cross(world_loop[2] - world_loop[0])
                        .normalize_or_zero();
                    let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                    let lift = |p: Vec3| offset_toward_camera(p, normal, eye, HOVER_FILL_DEPTH_BIAS);
                    for i in 1..world_loop.len() - 1 {
                        self.push_triangle(
                            lift(world_loop[0]),
                            lift(world_loop[i]),
                            lift(world_loop[i + 1]),
                            fill,
                        );
                    }
                    let n = world_loop.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        self.push_line_segment(
                            world_loop[i],
                            world_loop[j],
                            color,
                            2.0,
                            cam,
                            viewport,
                            view_proj,
                        );
                    }
                }
            }
        }
    }

    fn push_sketch_face_hover_border(
        &mut self,
        doc: &Document,
        face: FaceId,
        color: Color32,
        width: f32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        match face {
            FaceId::Circle(index) => {
                if let Some(circle) = doc.circles.get(index) {
                    if let Some(perimeter) =
                        circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS)
                    {
                        for window in perimeter.windows(2) {
                            self.push_line_segment(
                                window[0],
                                window[1],
                                color,
                                width,
                                cam,
                                viewport,
                                view_proj,
                            );
                        }
                    }
                }
            }
            FaceId::Polygon(lines) => {
                if let Some((poly, _)) =
                    crate::extrude::face_profile_world(doc, &crate::model::ExtrudeFace::Polygon(lines))
                {
                    let n = poly.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        self.push_line_segment(
                            poly[i], poly[j], color, width, cam, viewport, view_proj,
                        );
                    }
                }
            }
            FaceId::ExtrudeCap {
                extrusion,
                profile,
                top,
            } => {
                if let Some(poly) =
                    crate::extrude::cap_polygon_world(doc, extrusion, &profile, top)
                {
                    let n = poly.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        self.push_line_segment(
                            poly[i], poly[j], color, width, cam, viewport, view_proj,
                        );
                    }
                }
            }
            FaceId::ExtrudeSide {
                extrusion,
                profile,
                edge,
            } => {
                if let Some(quad) =
                    crate::extrude::side_quad_world(doc, extrusion, &profile, edge as usize)
                {
                    for i in 0..quad.len() {
                        let j = (i + 1) % quad.len();
                        self.push_line_segment(
                            quad[i], quad[j], color, width, cam, viewport, view_proj,
                        );
                    }
                }
            }
            FaceId::ConstructionPlane(_) => {}
        }
    }

    /// Viewport highlight for an elements-pane hover (#161): per element kind, in `color`.
    /// Drawn depth-test-disabled like other pick highlights (#153).
    fn push_element_hover(
        &mut self,
        doc: &Document,
        element: crate::hierarchy::SceneElement,
        color: Color32,
        _body_meshes: &[Option<crate::extrude::SolidMesh>],
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        use crate::hierarchy::SceneElement;
        let project = |w: Vec3| cam.project(w, viewport, view_proj);
        let restore = self.index_layer;
        self.set_index_layer(MeshIndexLayer::Wireframe);
        match element {
            SceneElement::Line(index) => {
                self.push_pick_target_highlight(
                    doc,
                    &PickTargetKind::Line(index),
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            SceneElement::Circle(index) => {
                self.push_pick_target_highlight(
                    doc,
                    &PickTargetKind::Circle(index),
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            SceneElement::Point(point) => {
                self.push_pick_target_highlight(
                    doc,
                    &PickTargetKind::Point(point),
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            SceneElement::ConstructionPlane(index) => {
                if let Some(plane) = doc.construction_planes.get(index) {
                    self.push_construction_plane_hover_fill(
                        plane,
                        index,
                        color,
                        FACE_HOVER_FILL_MULTIPLIER,
                        cam,
                    );
                }
            }
            // A sketch highlights as all of its entities.
            SceneElement::Sketch(sketch) => {
                for (li, line) in doc.lines.iter().enumerate() {
                    if !line.deleted && line.sketch == sketch {
                        self.push_pick_target_highlight(
                            doc,
                            &PickTargetKind::Line(li),
                            color,
                            cam,
                            viewport,
                            view_proj,
                            &project,
                        );
                    }
                }
                for (ci, circle) in doc.circles.iter().enumerate() {
                    if !circle.deleted && circle.sketch == sketch {
                        self.push_pick_target_highlight(
                            doc,
                            &PickTargetKind::Circle(ci),
                            color,
                            cam,
                            viewport,
                            view_proj,
                            &project,
                        );
                    }
                }
            }
            SceneElement::Constraint(index) => {
                if let Some((a, b)) = crate::constraints::constraint_segment_endpoints(doc, index)
                {
                    self.push_segment_hover(a, b, color, cam, viewport, view_proj, &project);
                }
            }
            SceneElement::BodyEdge { a, b, .. } => {
                self.push_segment_hover(
                    crate::hierarchy::dequantize_body_point(a),
                    crate::hierarchy::dequantize_body_point(b),
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            SceneElement::BodyVertex { p, .. } => {
                push_screen_disc(
                    self,
                    crate::hierarchy::dequantize_body_point(p),
                    6.0,
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            // A hovered body face (#555): re-find the coplanar group by its quantized key and
            // reuse the pick-target face highlight (fill + boundary stroke).
            SceneElement::BodyFace { body, centroid, normal } => {
                if let Some(solid) = crate::extrude::body_solid_mesh(doc, body) {
                    let q = crate::hierarchy::quantize_body_point;
                    let group = solid_mesh_coplanar_faces(&solid).into_iter().find(|tris| {
                        let count = (tris.len() * 3).max(1) as f32;
                        let c = tris.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
                        let n = (tris[0][1] - tris[0][0])
                            .cross(tris[0][2] - tris[0][0])
                            .normalize_or_zero();
                        q(c) == centroid && q(n) == normal
                    });
                    if let Some(tris) = group {
                        let n = (tris[0][1] - tris[0][0])
                            .cross(tris[0][2] - tris[0][0])
                            .normalize_or_zero();
                        self.push_pick_target_highlight(
                            doc,
                            &PickTargetKind::BodyFace {
                                body,
                                triangles: tris,
                                normal: n,
                            },
                            color,
                            cam,
                            viewport,
                            view_proj,
                            &project,
                        );
                    }
                }
            }
            // A hovered sketch text (#307): trace its glyph outlines in the hover color, so
            // the Extrude tool's "click picks the whole string" affordance is visible.
            SceneElement::SketchText(ti) => {
                let Some(text) = doc.sketch_texts.get(ti).filter(|t| !t.deleted) else {
                    return;
                };
                let Some(frame) = crate::face::sketch_geometry_frame(doc, text.sketch) else {
                    return;
                };
                let (sin, cos) = text.rotation.sin_cos();
                for contour in &text.contours {
                    if contour.len() < 2 {
                        continue;
                    }
                    let mut pts: Vec<Vec3> = contour
                        .iter()
                        .map(|&(x, y)| {
                            let rx = x * cos - y * sin + text.origin.0;
                            let ry = x * sin + y * cos + text.origin.1;
                            crate::face::local_to_world(&frame, rx, ry)
                        })
                        .collect();
                    if let Some(first) = pts.first().copied() {
                        pts.push(first);
                    }
                    self.push_polyline_segment(&pts, color, 3.0, cam, viewport, view_proj);
                }
            }
            SceneElement::FaceEdge(_)
            | SceneElement::Origin
            | SceneElement::Image(_)
            | SceneElement::BooleanOp(_)
            | SceneElement::MoveOp(_)
            | SceneElement::MirrorOp(_)
            | SceneElement::RepeatOp(_)
            | SceneElement::SketchRepeatOp(_)
            | SceneElement::SketchOffsetOp(_)
            | SceneElement::SketchMirrorOp(_)
            | SceneElement::SketchVertexTreatmentOp(_)
            | SceneElement::SketchSliceOp(_)
            | SceneElement::SliceOp(_)
            | SceneElement::EdgeTreatmentOp(_)
            | SceneElement::Revolution(_)
            | SceneElement::SweepOp(_)
            | SceneElement::Component(_) => {}
            // A hovered body recolors in the main pass (#455); a hovered extrusion
            // recolors just its own solid.
            SceneElement::Body(_) => {}
            SceneElement::Extrusion(index) => {
                self.push_sub_body_recolor(doc, index, color, cam, viewport, view_proj);
            }
        }
        self.set_index_layer(restore);
    }

    fn push_pick_target_highlight(
        &mut self,
        doc: &Document,
        kind: &PickTargetKind,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        match kind {
            PickTargetKind::Point(point) => {
                if let Some(world) = crate::construction::point_world_position(doc, point.clone()) {
                    push_screen_disc(
                        self,
                        world,
                        6.0,
                        color,
                        cam,
                        viewport,
                        view_proj,
                        project,
                    );
                }
            }
            PickTargetKind::Line(index) => {
                if let Some(line) = doc.lines.get(*index) {
                    if let Some(points) = line_world_polyline(doc, line) {
                        self.push_polyline_hover(&points, color, cam, viewport, view_proj, project);
                    }
                }
            }
            PickTargetKind::Circle(index) => {
                if let Some(circle) = doc.circles.get(*index) {
                    self.push_segment_hover_ring(doc, circle, color, cam, viewport, view_proj);
                }
            }
            PickTargetKind::BodyEdge { a, b, .. } => {
                self.push_segment_hover(*a, *b, color, cam, viewport, view_proj, project);
            }
            PickTargetKind::BodyFace {
                triangles, normal, ..
            } => {
                let eye = cam.eye();
                let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                let lift =
                    |p: Vec3| offset_toward_camera(p, *normal, eye, HOVER_FILL_DEPTH_BIAS);
                for tri in triangles {
                    self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                }
                for (a, b) in crate::construction::coplanar_face_boundary(triangles) {
                    self.push_line_segment(a, b, color, 3.0, cam, viewport, view_proj);
                }
            }
            PickTargetKind::BodyVertex { position, .. } => {
                push_screen_disc(self, *position, 5.0, color, cam, viewport, view_proj, project);
            }
            PickTargetKind::GlobalAxis(axis) => {
                let (a, b) = global_axis_segment(*axis);
                let axis_color = axis.color().gamma_multiply(1.25);
                self.push_segment_hover(a, b, axis_color, cam, viewport, view_proj, project);
            }
            PickTargetKind::ConstructionPlane(index) => {
                if let Some(plane) = doc.construction_planes.get(*index) {
                    self.push_construction_plane_hover_fill(
                        plane,
                        *index,
                        color,
                        FACE_HOVER_FILL_MULTIPLIER,
                        cam,
                    );
                }
            }
            PickTargetKind::Ground(p) => {
                push_ground_hover_marker(self, *p, color, cam, viewport, view_proj, project);
            }
            // A constraint's hover highlight is its badge glowing in the 2D annotation overlay
            // (#568), not a world-geometry marker — nothing to push into the 3D scene here.
            PickTargetKind::Constraint(_) => {}
        }
    }

    fn push_segment_hover(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        _project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        // Just the line — no endpoint discs: those big circles obscure the very edge being
        // highlighted, making it hard to see what a click will pick.
        self.push_line_segment(a, b, color, 4.0, cam, viewport, view_proj);
    }

    /// Hover highlight for a (possibly curved) polyline: the sampled path only. No discs at the
    /// endpoints — they obscure the line they mark (a vertex highlight is drawn only for an actual
    /// point pick).
    fn push_polyline_hover(
        &mut self,
        points: &[Vec3],
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        _project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        for pair in points.windows(2) {
            self.push_line_segment(pair[0], pair[1], color, 4.0, cam, viewport, view_proj);
        }
    }

    fn push_segment_hover_ring(
        &mut self,
        doc: &Document,
        circle: &Circle,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        if let Some(perimeter) = circle_world_perimeter(doc, circle, CIRCLE_SEGMENTS) {
            for window in perimeter.windows(2) {
                self.push_line_segment(
                    window[0],
                    window[1],
                    color,
                    3.0,
                    cam,
                    viewport,
                    view_proj,
                );
            }
        }
    }
}

fn push_ground_hover_marker(
    mesh: &mut SceneMesh<'_>,
    point: Vec3,
    color: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    push_screen_ring(mesh, point, 8.0, color, 2.0, cam, viewport, view_proj, project);
    let (tangent, bitangent) = camera_disc_basis(point, cam);
    let arm = pixels_to_world_distance(project, point, tangent, 6.0);
    mesh.push_line_segment(
        point - tangent * arm,
        point + tangent * arm,
        color,
        2.0,
        cam,
        viewport,
        view_proj,
    );
    mesh.push_line_segment(
        point - bitangent * arm,
        point + bitangent * arm,
        color,
        2.0,
        cam,
        viewport,
        view_proj,
    );
}

pub fn fill_color(base: Color32, opacity: f32) -> Color32 {
    base.gamma_multiply(opacity)
}

/// Test-only convenience for the lane-0 (rectangle) depth bias. Production code calls
/// `shape_fill_depth_bias_laned` directly with the appropriate lane.
#[cfg(test)]
pub fn shape_fill_depth_bias(index: usize) -> f32 {
    shape_fill_depth_bias_laned(index, 0)
}

/// Depth bias for a coplanar sketch-shape fill. `index` separates shapes of the same type;
/// `lane` (0 = rectangles, 1 = circles) adds a half-step so two *different* shape types never
/// land on the same bias — otherwise e.g. rect 0 and circle 0 are coplanar with identical
/// depth and z-fight ("jaggies" where a circle sits inside a rectangle).
pub fn shape_fill_depth_bias_laned(index: usize, lane: usize) -> f32 {
    SHAPE_FILL_DEPTH_BIAS_BASE
        + (index % SHAPE_FILL_DEPTH_BIAS_MODULO) as f32 * SHAPE_FILL_DEPTH_BIAS_STEP
        + lane as f32 * SHAPE_FILL_DEPTH_BIAS_STEP * 0.5
}

pub fn plane_fill_depth_bias(index: usize) -> f32 {
    PLANE_FILL_DEPTH_BIAS - index as f32 * SHAPE_FILL_DEPTH_BIAS_STEP * 0.25
}

fn plane_camera_depth(plane: &ConstructionPlane, cam: &Camera) -> f32 {
    let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
    let center = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
    (cam.eye() - center).length()
}

fn offset_segment_toward_camera(a: Vec3, b: Vec3, eye: Vec3, bias: f32) -> (Vec3, Vec3) {
    if bias == 0.0 {
        return (a, b);
    }
    let mid = (a + b) * 0.5;
    let to_cam = (eye - mid).normalize_or_zero();
    (a + to_cam * bias, b + to_cam * bias)
}

/// Whether every vertex of `tri` lies (within tolerance) on the plane through `origin`
/// with unit-ish `normal`.
fn triangle_on_plane(tri: &[Vec3; 3], origin: Vec3, normal: Vec3) -> bool {
    let n = normal.normalize_or_zero();
    if n.length_squared() < 1e-8 {
        return false;
    }
    tri.iter().all(|p| (*p - origin).dot(n).abs() < 1e-3)
}

/// Quantize a world position to a hashable key so coincident vertices (within a tight
/// tolerance) compare equal, letting [`solid_mesh_unique_edges`] dedupe the edge shared by
/// two adjacent triangles even though `SolidMesh` stores triangles as raw positions rather
/// than an indexed vertex buffer.
fn quantize_vertex(v: Vec3) -> (i64, i64, i64) {
    const SCALE: f32 = 1000.0; // 0.001 world-unit precision.
    (
        (v.x * SCALE).round() as i64,
        (v.y * SCALE).round() as i64,
        (v.z * SCALE).round() as i64,
    )
}

/// Cosine of the smallest angle between two triangles' face normals that counts as a real
/// feature edge (crease) between them, rather than an internal tessellation seam — either the
/// diagonal splitting a flat face into triangles (#82) or the small angle between adjacent
/// facets approximating a smooth curved surface (#101).
///
/// cos(15°): this app's curved-surface tessellations all facet finer than that — a
/// `CIRCLE_SEGMENTS` (48-gon) cylinder wall meets at 7.5°, a `BEZIER_SEGMENTS` (24-segment)
/// fillet/bezier arc at ~3.75° per 90°, and OCCT's `OCCT_DEFLECTION` (0.05) linear-deflection
/// meshing stays under 15° for arc radii ≳ 6 — while genuine feature edges are far larger
/// (box corners 90°, chamfer bevels ≥ ~30°). Trade-off: very small-radius OCCT fillets
/// (r ≲ 6, where 0.05 deflection allows facet angles up to OCCT's 0.5 rad angular-deflection
/// cap ≈ 28.6°) can still show a few seams. Shared deliberately with body-edge *picking*
/// (#31, `construction.rs`): pickable edges stay exactly the edges wireframe draws, and facet
/// seams were never stable references anyway (they move whenever tessellation changes).
const WIREFRAME_CREASE_COS_THRESHOLD: f32 = 0.965_926; // cos(15°)

/// Extract the *feature* edges of a triangle-soup solid mesh (#33/#82): an edge is kept only
/// if it's a mesh boundary (used by just one triangle) or a real crease — shared by two or
/// more triangles whose face normals meaningfully differ. An edge shared only by coplanar
/// triangles (the internal diagonals ear-clipping/faceting adds to make a flat face, or the
/// facets approximating a circle/curve) is dropped, so wireframe view shows the shape's real
/// flat faces rather than its internal triangulation. Performance: this walks all triangles
/// once per frame, which is fine at this app's scale (small CAD models, not high-poly meshes)
/// — not worth caching for a first cut.
pub fn solid_mesh_unique_edges(solid: &crate::extrude::SolidMesh) -> Vec<(Vec3, Vec3)> {
    type EdgeKey = ((i64, i64, i64), (i64, i64, i64));
    let mut by_edge: std::collections::HashMap<EdgeKey, (Vec3, Vec3, Vec<Vec3>)> =
        std::collections::HashMap::new();
    for tri in &solid.triangles {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        for &(i, j) in &[(0usize, 1usize), (1, 2), (2, 0)] {
            let a = tri[i];
            let b = tri[j];
            let ka = quantize_vertex(a);
            let kb = quantize_vertex(b);
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            by_edge
                .entry(key)
                .or_insert_with(|| (a, b, Vec::new()))
                .2
                .push(normal);
        }
    }
    by_edge
        .into_values()
        .filter(|(_, _, normals)| is_feature_edge(normals))
        .map(|(a, b, _)| (a, b))
        .collect()
}

/// View-dependent **silhouette** edges of a solid mesh for an orthographic projection along
/// `view_dir` (#319): a manifold edge whose two adjacent faces face opposite ways (one toward
/// the viewer, one away) is on the outline — e.g. the straight sides of a cylinder seen from
/// the side, which are not crease edges and so are missed by [`solid_mesh_unique_edges`].
/// Naked edges (only one adjacent face) are always included.
pub fn solid_mesh_silhouette_edges(
    solid: &crate::extrude::SolidMesh,
    view_dir: Vec3,
) -> Vec<(Vec3, Vec3)> {
    type EdgeKey = ((i64, i64, i64), (i64, i64, i64));
    let mut by_edge: std::collections::HashMap<EdgeKey, (Vec3, Vec3, Vec<f32>)> =
        std::collections::HashMap::new();
    let v = view_dir.normalize_or_zero();
    for tri in &solid.triangles {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        let facing = normal.dot(v);
        for &(i, j) in &[(0usize, 1usize), (1, 2), (2, 0)] {
            let (a, b) = (tri[i], tri[j]);
            let (ka, kb) = (quantize_vertex(a), quantize_vertex(b));
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            by_edge.entry(key).or_insert_with(|| (a, b, Vec::new())).2.push(facing);
        }
    }
    by_edge
        .into_values()
        .filter(|(_, _, facings)| match facings.as_slice() {
            [_] => true,                       // naked boundary edge
            [f0, f1] => f0.signum() != f1.signum(), // facing flips → silhouette
            _ => false,
        })
        .map(|(a, b, _)| (a, b))
        .collect()
}









/// Group a solid mesh's triangles into planar faces (#144): maximal sets of triangles connected
/// through shared edges whose two adjacent triangles are coplanar (normals agree within
/// [`WIREFRAME_CREASE_COS_THRESHOLD`], the same crease test [`solid_mesh_unique_edges`] uses).
/// Each returned face is its list of world-space triangles, so the picker can hover-highlight a
/// whole box side or cylinder cap as one face instead of a bare triangle.
pub fn solid_mesh_coplanar_faces(solid: &crate::extrude::SolidMesh) -> Vec<Vec<[Vec3; 3]>> {
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    let n = solid.triangles.len();
    let normals: Vec<Vec3> = solid
        .triangles
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero())
        .collect();
    let mut parent: Vec<usize> = (0..n).collect();

    type EdgeKey = ((i64, i64, i64), (i64, i64, i64));
    let mut by_edge: std::collections::HashMap<EdgeKey, Vec<usize>> = std::collections::HashMap::new();
    for (ti, tri) in solid.triangles.iter().enumerate() {
        for &(i, j) in &[(0usize, 1usize), (1, 2), (2, 0)] {
            let ka = quantize_vertex(tri[i]);
            let kb = quantize_vertex(tri[j]);
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            by_edge.entry(key).or_default().push(ti);
        }
    }
    for tris in by_edge.values() {
        for a in 0..tris.len() {
            for b in (a + 1)..tris.len() {
                // Two-sided shading (`push_solid`) means coplanar triangles can be wound
                // oppositely, so compare normals by absolute dot — same as `is_feature_edge`.
                if normals[tris[a]].dot(normals[tris[b]]).abs() >= WIREFRAME_CREASE_COS_THRESHOLD {
                    let ra = find(&mut parent, tris[a]);
                    let rb = find(&mut parent, tris[b]);
                    if ra != rb {
                        parent[ra] = rb;
                    }
                }
            }
        }
    }

    let mut groups: std::collections::HashMap<usize, Vec<[Vec3; 3]>> = std::collections::HashMap::new();
    for ti in 0..n {
        let root = find(&mut parent, ti);
        groups.entry(root).or_default().push(solid.triangles[ti]);
    }
    groups.into_values().collect()
}

/// View-dependent silhouette edges on *smooth* surfaces (#158): edges whose two adjacent
/// triangles are tessellation-smooth (below the crease threshold, so [`solid_mesh_unique_edges`]
/// drops them) but where the surface turns away from the camera — one triangle front-facing,
/// the other back-facing. These are the lines "tangent to the sight of the camera" that make a
/// cylinder's sides visible in wireframe; they move as the camera orbits, so they're rebuilt
/// per frame with the rest of the wireframe overlay. Adjacent normals are sign-aligned before
/// the facing comparison, so inconsistent triangle winding (this app's meshes are shaded
/// two-sided) can't flip the test.
pub fn solid_mesh_smooth_silhouette_edges(
    solid: &crate::extrude::SolidMesh,
    eye: Vec3,
) -> Vec<(Vec3, Vec3)> {
    type EdgeKey = ((i64, i64, i64), (i64, i64, i64));
    struct EdgeFaces {
        a: Vec3,
        b: Vec3,
        // (normal, centroid) of up to two adjacent triangles; more means non-manifold — skip.
        faces: Vec<(Vec3, Vec3)>,
    }
    let mut by_edge: std::collections::HashMap<EdgeKey, EdgeFaces> = std::collections::HashMap::new();
    for tri in &solid.triangles {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        let centroid = (tri[0] + tri[1] + tri[2]) / 3.0;
        for &(i, j) in &[(0usize, 1usize), (1, 2), (2, 0)] {
            let (a, b) = (tri[i], tri[j]);
            let ka = quantize_vertex(a);
            let kb = quantize_vertex(b);
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            by_edge
                .entry(key)
                .or_insert_with(|| EdgeFaces { a, b, faces: Vec::new() })
                .faces
                .push((normal, centroid));
        }
    }
    by_edge
        .into_values()
        .filter_map(|edge| {
            let [(n1, c1), (n2, c2)] = edge.faces.as_slice() else {
                return None;
            };
            // Smooth pair only — creases are already drawn as permanent feature edges.
            if n1.dot(*n2).abs() < WIREFRAME_CREASE_COS_THRESHOLD {
                return None;
            }
            // Align the second normal with the first so winding doesn't flip the facing.
            let n2_aligned = if n1.dot(*n2) < 0.0 { -*n2 } else { *n2 };
            let f1 = n1.dot(eye - *c1);
            let f2 = n2_aligned.dot(eye - *c2);
            (f1 * f2 < 0.0).then_some((edge.a, edge.b))
        })
        .collect()
}

/// An edge is a real feature edge if it's a mesh boundary (one adjacent triangle) or any pair
/// of its adjacent triangles' normals diverge beyond [`WIREFRAME_CREASE_COS_THRESHOLD`].
/// Compares normals by absolute dot product: this mesh's triangles are shaded two-sided (see
/// `push_solid`'s `.abs()`), so two triangles can be genuinely coplanar yet wound in opposite
/// directions (anti-parallel normals) — that must still count as flat, not a crease.
fn is_feature_edge(normals: &[Vec3]) -> bool {
    if normals.len() <= 1 {
        return true;
    }
    for i in 0..normals.len() {
        for j in (i + 1)..normals.len() {
            if normals[i].dot(normals[j]).abs() < WIREFRAME_CREASE_COS_THRESHOLD {
                return true;
            }
        }
    }
    false
}

pub fn offset_toward_camera(pos: Vec3, normal: Vec3, eye: Vec3, bias: f32) -> Vec3 {
    if bias == 0.0 {
        return pos;
    }
    let n = normal.normalize_or_zero();
    if n.length_squared() < 1e-8 {
        return pos;
    }
    let toward_camera = if n.dot(eye - pos) >= 0.0 { n } else { -n };
    pos + toward_camera * bias
}

fn offset_corners_toward_camera(
    corners: [Vec3; 4],
    normal: Vec3,
    eye: Vec3,
    bias: f32,
) -> [Vec3; 4] {
    [
        offset_toward_camera(corners[0], normal, eye, bias),
        offset_toward_camera(corners[1], normal, eye, bias),
        offset_toward_camera(corners[2], normal, eye, bias),
        offset_toward_camera(corners[3], normal, eye, bias),
    ]
}

pub(crate) fn color32_to_gpu(color: Color32) -> [f32; 4] {
    let [r, g, b, a] = color.to_array();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

fn push_linear_dimension_world(
    mesh: &mut SceneMesh<'_>,
    world: &LinearDimensionWorldGeom,
    color: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    mesh.push_line_segment(
        world.ext_a_near,
        world.ext_a_far,
        color,
        LINE_WIDTH,
        cam,
        viewport,
        view_proj,
    );
    mesh.push_line_segment(
        world.ext_b_near,
        world.ext_b_far,
        color,
        LINE_WIDTH,
        cam,
        viewport,
        view_proj,
    );
    mesh.push_line_segment(
        world.dim_a,
        world.dim_b,
        color,
        LINE_WIDTH,
        cam,
        viewport,
        view_proj,
    );
    push_arrowhead_world(
        mesh,
        world,
        world.dim_a,
        -world.along_world,
        color,
        cam,
        viewport,
        view_proj,
        project,
    );
    push_arrowhead_world(
        mesh,
        world,
        world.dim_b,
        world.along_world,
        color,
        cam,
        viewport,
        view_proj,
        project,
    );
}

fn push_arrowhead_world(
    mesh: &mut SceneMesh<'_>,
    world: &LinearDimensionWorldGeom,
    tip: Vec3,
    dir: Vec3,
    color: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let along = dir.normalize_or_zero();
    if along.length_squared() < 1e-8 {
        return;
    }
    let arrow_len = pixels_to_world_distance(project, tip, along, ARROW_LENGTH);
    let arrow_wing = pixels_to_world_distance(project, tip, along, ARROW_WING);
    let mut side = dimension_arrow_wing_world(along, world.outward_world);
    if side.length_squared() < 1e-8 {
        let to_cam = (cam.eye() - tip).normalize_or_zero();
        side = along.cross(to_cam).normalize_or_zero();
    }
    if side.length_squared() < 1e-8 {
        return;
    }
    let base = tip - along * arrow_len;
    mesh.push_line_segment(
        tip,
        base + side * arrow_wing,
        color,
        LINE_WIDTH,
        cam,
        viewport,
        view_proj,
    );
    mesh.push_line_segment(
        tip,
        base - side * arrow_wing,
        color,
        LINE_WIDTH,
        cam,
        viewport,
        view_proj,
    );
}

impl<'a> SceneMesh<'a> {
    fn push_plane_gizmo(
        &mut self,
        gizmo: &ViewportPlaneGizmo,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        match &gizmo.reference {
            PlaneReference::Face { origin, normal, .. } => self.push_offset_gizmo(
                *origin,
                *normal,
                gizmo.offset,
                gizmo.color,
                gizmo.hover == Some(AxisGizmoHit::Offset),
                cam,
                viewport,
                view_proj,
                project,
            ),
            PlaneReference::Axis {
                origin,
                direction,
                ..
            } => self.push_axis_plane_gizmo(
                *origin,
                *direction,
                gizmo.offset,
                gizmo.angle_deg,
                gizmo.color,
                gizmo.hover,
                cam,
                viewport,
                view_proj,
                project,
            ),
        }
    }

    fn push_offset_gizmo(
        &mut self,
        origin: Vec3,
        normal: Vec3,
        offset: f32,
        color: Color32,
        hovered: bool,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let n = normal.normalize_or_zero();
        let tip = origin + n * gizmo_display_offset(offset);
        let stroke = if hovered {
            GIZMO_OFFSET_STROKE_HOVER_PX
        } else {
            GIZMO_OFFSET_STROKE_PX
        };
        let stroke_color = if hovered {
            GIZMO_HANDLE_HOVER_RGBA
        } else {
            color
        };
        if project(origin).is_some() && project(tip).is_some() {
            // Stop the connecting line short of the back-pointing arrow so it doesn't run
            // through it — both arrows should read as unobstructed affordances (#250). The
            // back arrow's far end sits `GAP + HEAD` px toward the origin from the tip.
            let back_gap = pixels_to_world_distance(
                project,
                tip,
                -n,
                GIZMO_ARROW_GAP_PX + GIZMO_ARROW_HEAD_PX,
            );
            let disp = (tip - origin).dot(n);
            if disp - back_gap > 0.0 {
                let line_end = tip - n * back_gap;
                self.push_line_segment(
                    origin, line_end, stroke_color, stroke, cam, viewport, view_proj,
                );
            }
            // The handle drags along both normal directions: one arrow each way, stood
            // off from the handle disc.
            for sign in [1.0f32, -1.0] {
                let dir = n * sign;
                let gap = pixels_to_world_distance(
                    project,
                    tip,
                    dir,
                    GIZMO_ARROW_GAP_PX + GIZMO_ARROW_HEAD_PX,
                );
                push_gizmo_arrowhead(
                    self,
                    tip + dir * gap,
                    dir,
                    GIZMO_ARROW_HEAD_PX,
                    GIZMO_ARROW_WING_PX,
                    stroke,
                    stroke_color,
                    cam,
                    viewport,
                    view_proj,
                    project,
                );
            }
        }
        if hovered {
            push_gizmo_handle_hover(self, tip, GIZMO_HANDLE_HOVER_RGBA, cam, viewport, view_proj, project);
        } else {
            push_gizmo_handle(self, tip, color, cam, viewport, view_proj, project);
        }
    }

    fn push_axis_plane_gizmo(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        offset: f32,
        angle_deg: f32,
        color: Color32,
        hover: Option<AxisGizmoHit>,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        let normal = axis_normal(direction, angle_deg);
        self.push_offset_gizmo(
            origin,
            normal,
            offset,
            color,
            hover == Some(AxisGizmoHit::Offset),
            cam,
            viewport,
            view_proj,
            project,
        );

        let axis = direction.normalize_or_zero();
        let perp = axis_reference_perp(axis);
        let angle_hovered = hover == Some(AxisGizmoHit::Angle);
        let circle_color = if angle_hovered {
            GIZMO_HANDLE_HOVER_RGBA.gamma_multiply(0.9)
        } else {
            color.gamma_multiply(0.85)
        };
        let circle_stroke = if angle_hovered {
            GIZMO_ANGLE_STROKE_HOVER_PX
        } else {
            GIZMO_ANGLE_STROKE_PX
        };
        let mut prev: Option<Vec3> = None;
        for i in 0..=GIZMO_ANGLE_CIRCLE_SEGMENTS {
            let a = i as f32 / GIZMO_ANGLE_CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
            let dir = Quat::from_axis_angle(axis, a) * perp;
            let pt = origin + dir * AXIS_ANGLE_GIZMO_RADIUS_MM;
            if let Some(p0) = prev {
                self.push_line_segment(p0, pt, circle_color, circle_stroke, cam, viewport, view_proj);
            }
            prev = Some(pt);
        }

        let handle = axis_angle_handle(origin, direction, angle_deg);
        let handle_dir = (handle - origin).normalize_or_zero();
        let tangent = axis.cross(handle_dir).normalize_or_zero();
        let angle_color = if angle_hovered {
            GIZMO_HANDLE_HOVER_RGBA
        } else {
            color
        };
        if angle_hovered {
            push_gizmo_handle_hover(
                self,
                handle,
                GIZMO_HANDLE_HOVER_RGBA,
                cam,
                viewport,
                view_proj,
                project,
            );
        } else {
            push_gizmo_handle(self, handle, color, cam, viewport, view_proj, project);
        }
        // The angle handle drags along the circle both ways: one arrow along each
        // tangent direction, stood off from the handle disc.
        for sign in [-1.0f32, 1.0] {
            let along = tangent * sign;
            let gap = pixels_to_world_distance(
                project,
                handle,
                along,
                GIZMO_ANGLE_ARROW_GAP_PX + GIZMO_ANGLE_ARROW_PX,
            );
            push_gizmo_arrowhead(
                self,
                handle + along * gap,
                along,
                GIZMO_ANGLE_ARROW_PX,
                GIZMO_ANGLE_WING_PX,
                2.0,
                angle_color,
                cam,
                viewport,
                view_proj,
                project,
            );
        }
    }
}

/// Flat line arrowhead for a gizmo handle: a V at `tip` pointing along `along_world`,
/// drawn screen-facing (the wing plane tracks the camera) so it never flares with
/// perspective the way a solid cone did. Sized in screen px at the tip.
fn push_gizmo_arrowhead(
    mesh: &mut SceneMesh<'_>,
    tip: Vec3,
    along_world: Vec3,
    head_px: f32,
    wing_px: f32,
    stroke_px: f32,
    color: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let along = along_world.normalize_or_zero();
    if along.length_squared() < 1e-8 {
        return;
    }
    let eye = cam.eye();
    let to_cam = (eye - tip).normalize_or_zero();
    let mut side = along.cross(to_cam);
    if side.length_squared() < 1e-8 {
        side = along.cross(cam.view_up_hint());
    }
    if side.length_squared() < 1e-8 {
        return;
    }
    side = side.normalize();
    let arrow_len = pixels_to_world_distance(project, tip, along, head_px);
    let arrow_wing = pixels_to_world_distance(project, tip, side, wing_px);
    let base = tip - along * arrow_len;
    mesh.push_line_segment(
        tip,
        base + side * arrow_wing,
        color,
        stroke_px,
        cam,
        viewport,
        view_proj,
    );
    mesh.push_line_segment(
        tip,
        base - side * arrow_wing,
        color,
        stroke_px,
        cam,
        viewport,
        view_proj,
    );
}
fn push_gizmo_handle(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    color: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    push_screen_disc(
        mesh,
        center,
        GIZMO_HANDLE_RADIUS_PX,
        color,
        cam,
        viewport,
        view_proj,
        project,
    );
    push_screen_ring(
        mesh,
        center,
        GIZMO_HANDLE_RADIUS_PX,
        color.gamma_multiply(0.5),
        GIZMO_HANDLE_RING_STROKE_PX,
        cam,
        viewport,
        view_proj,
        project,
    );
}

fn push_gizmo_handle_hover(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    accent: Color32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    push_screen_disc(
        mesh,
        center,
        GIZMO_HOVER_INNER_RADIUS_PX,
        accent.gamma_multiply(0.35),
        cam,
        viewport,
        view_proj,
        project,
    );
    push_screen_ring(
        mesh,
        center,
        GIZMO_HOVER_INNER_RADIUS_PX,
        accent,
        2.5,
        cam,
        viewport,
        view_proj,
        project,
    );
    push_screen_ring(
        mesh,
        center,
        GIZMO_HOVER_OUTER_RADIUS_PX,
        accent.gamma_multiply(0.75),
        1.5,
        cam,
        viewport,
        view_proj,
        project,
    );
}

fn push_screen_disc(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    radius_px: f32,
    color: Color32,
    cam: &Camera,
    _viewport: UiRect,
    _view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let (tangent, bitangent) = camera_disc_basis(center, cam);
    let radius = pixels_to_world_distance(project, center, tangent, radius_px);
    if radius < 1e-6 {
        return;
    }
    const SEGMENTS: usize = 16;
    let base = mesh.scene.vertices.len() as u32;
    mesh.push_vertex(center, color);
    for i in 0..SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let p = center + tangent * a.cos() * radius + bitangent * a.sin() * radius;
        mesh.push_vertex(p, color);
    }
    for i in 0..SEGMENTS {
        let next = (i + 1) % SEGMENTS;
        mesh.scene
            .indices
            .extend_from_slice(&[base, base + 1 + i as u32, base + 1 + next as u32]);
    }
}

fn push_screen_ring(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    radius_px: f32,
    color: Color32,
    stroke_px: f32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let (tangent, bitangent) = camera_disc_basis(center, cam);
    let radius = pixels_to_world_distance(project, center, tangent, radius_px);
    if radius < 1e-6 {
        return;
    }
    const SEGMENTS: usize = 24;
    let mut prev: Option<Vec3> = None;
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let p = center + tangent * a.cos() * radius + bitangent * a.sin() * radius;
        if let Some(p0) = prev {
            mesh.push_line_segment(p0, p, color, stroke_px, cam, viewport, view_proj);
        }
        prev = Some(p);
    }
}

fn camera_disc_basis(center: Vec3, cam: &Camera) -> (Vec3, Vec3) {
    let eye = cam.eye();
    let to_cam = (eye - center).normalize_or_zero();
    let mut tangent = to_cam.cross(Vec3::Z);
    if tangent.length_squared() < 1e-8 {
        tangent = to_cam.cross(Vec3::X);
    }
    tangent = tangent.normalize_or_zero();
    let bitangent = to_cam.cross(tangent).normalize_or_zero();
    (tangent, bitangent)
}

fn sketch_color(color: Color32, dim: bool) -> Color32 {
    if dim {
        color.gamma_multiply(SKETCH_DIMMED)
    } else {
        color
    }
}

/// Scale an RGB color by `factor` (for flat shading), keeping alpha.
fn scale_color(color: Color32, factor: f32) -> Color32 {
    let f = factor.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (color.r() as f32 * f) as u8,
        (color.g() as f32 * f) as u8,
        (color.b() as f32 * f) as u8,
        color.a(),
    )
}

/// Blend `color` toward white by `amount` (0 = unchanged, 1 = white) — used to add a specular
/// highlight on top of the already-lit base color in [`realistic_shade`].
fn lighten_color(color: Color32, amount: f32) -> Color32 {
    let t = amount.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (color.r() as f32 + (255.0 - color.r() as f32) * t) as u8,
        (color.g() as f32 + (255.0 - color.g() as f32) * t) as u8,
        (color.b() as f32 + (255.0 - color.b() as f32) * t) as u8,
        color.a(),
    )
}

/// Ambient/diffuse/specular weights for `ShadingMode::Realistic` (#83) — a fixed matte/satin
/// "painted object" look; no per-material tuning yet.
const REALISTIC_AMBIENT: f32 = 0.30;
const REALISTIC_DIFFUSE: f32 = 0.55;
const REALISTIC_SPECULAR: f32 = 0.35;
const REALISTIC_SHININESS: f32 = 24.0;
/// Weight of the camera-attached "headlight" diffuse term (#102). The fixed light sits
/// above-ish the scene, so from horizontal views a camera-facing wall got no diffuse at all
/// and rendered at the ambient floor — nearly black, *less* readable than `Solid` mode. The
/// headlight guarantees a face square to the camera reaches ambient + 0.7·diffuse ≈ 0.69
/// total, on par with `Solid`'s ~0.67 for the same face. Combined with `max()` (not summed)
/// so the fixed light still dominates wherever it lands, preserving per-face contrast/shape.
const REALISTIC_HEADLIGHT: f32 = 0.70;

/// Blinn-Phong-ish flat shading for one triangle face, two-sided (the normal is flipped to
/// face the camera first, matching `push_solid`'s two-sided convention): ambient + diffuse +
/// a camera-dependent specular highlight, instead of `push_solid`'s single Lambert-ish term.
/// The diffuse term takes the stronger of the fixed scene light and a camera headlight
/// (#102), so surfaces the user is looking at are always reasonably lit.
fn realistic_shade(base: Color32, normal: Vec3, light: Vec3, view: Vec3) -> Color32 {
    let n = if normal.dot(view) < 0.0 { -normal } else { normal };
    let fixed_diffuse = n.dot(light).max(0.0);
    let headlight_diffuse = REALISTIC_HEADLIGHT * n.dot(view).max(0.0);
    let diffuse = fixed_diffuse.max(headlight_diffuse);
    let half = (light + view).normalize_or_zero();
    let specular = n.dot(half).max(0.0).powf(REALISTIC_SHININESS);
    let intensity = REALISTIC_AMBIENT + REALISTIC_DIFFUSE * diffuse;
    let shaded = scale_color(base, intensity.min(1.0));
    lighten_color(shaded, REALISTIC_SPECULAR * specular)
}

pub fn sketch_ground_color(color: Color32, in_sketch: bool) -> Color32 {
    if in_sketch {
        color.gamma_multiply(SKETCH_GROUND_DIMMED)
    } else {
        color
    }
}

fn sketch_circle_is_active(
    doc: &Document,
    session: SketchSession,
    circle_index: usize,
    circle_sketch: crate::model::SketchId,
) -> bool {
    if circle_sketch == session.sketch {
        return true;
    }
    if let Some(FaceId::Circle(face_index)) = doc.sketch_face(session.sketch) {
        return circle_index == face_index;
    }
    false
}

fn line_world_endpoints(doc: &Document, line: &Line) -> Option<(Vec3, Vec3)> {
    let frame = sketch_geometry_frame(doc, line.sketch)?;
    let a = crate::face::local_to_world(&frame, line.x0, line.y0);
    let b = crate::face::local_to_world(&frame, line.x1, line.y1);
    Some((a, b))
}

/// World-space polyline approximation of a line, sampled with [`crate::model::BEZIER_SEGMENTS`]
/// segments for a curved line, or just its two endpoints for a straight one.
fn line_world_polyline(doc: &Document, line: &Line) -> Option<Vec<Vec3>> {
    let frame = sketch_geometry_frame(doc, line.sketch)?;
    Some(
        line.sample_local(crate::model::BEZIER_SEGMENTS)
            .into_iter()
            .map(|(u, v)| crate::face::local_to_world(&frame, u, v))
            .collect(),
    )
}

fn world_t_at_screen_fraction(
    a: Vec3,
    b: Vec3,
    fraction: f32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
) -> Option<f32> {
    let sa = cam.project(a, viewport, view_proj)?;
    let sb = cam.project(b, viewport, view_proj)?;
    let axis = sb - sa;
    let len = axis.length();
    if len < 1e-3 {
        return None;
    }
    let dir = axis / len;
    let target_along = fraction.clamp(0.0, 1.0) * len;
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        let p = cam.project(a.lerp(b, mid), viewport, view_proj)?;
        let along = (p - sa).dot(dir);
        if along < target_along {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo + hi) * 0.5)
}

/// Split a world segment into dash spans using screen-space lengths (matches egui dashed lines).
pub fn dashed_world_segments(
    a: Vec3,
    b: Vec3,
    dash_length_px: f32,
    gap_length_px: f32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
) -> Vec<(Vec3, Vec3)> {
    let Some(sa) = cam.project(a, viewport, view_proj) else {
        return Vec::new();
    };
    let Some(sb) = cam.project(b, viewport, view_proj) else {
        return Vec::new();
    };
    let len = (sb - sa).length();
    if len < 1e-3 || dash_length_px <= 0.0 {
        return Vec::new();
    }
    let period = (dash_length_px + gap_length_px).max(1e-3);
    let mut segments = Vec::new();
    let mut pos = 0.0f32;
    while pos < len {
        let dash_start = pos;
        let dash_end = (pos + dash_length_px).min(len);
        if dash_end > dash_start + 1e-3 {
            let f0 = dash_start / len;
            let f1 = dash_end / len;
            if let (Some(u0), Some(u1)) = (
                world_t_at_screen_fraction(a, b, f0, cam, viewport, view_proj),
                world_t_at_screen_fraction(a, b, f1, cam, viewport, view_proj),
            ) {
                segments.push((a.lerp(b, u0), a.lerp(b, u1)));
            }
        }
        pos += period;
    }
    segments
}

/// Build a camera-facing line ribbon in world space for a given screen width.
pub fn line_screen_quad(
    a: Vec3,
    b: Vec3,
    width_px: f32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
) -> Option<[Vec3; 4]> {
    let _ = view_proj;
    let sa = cam.project(a, viewport, view_proj)?;
    let sb = cam.project(b, viewport, view_proj)?;
    if (sa - sb).length() < 1e-3 {
        return None;
    }
    let dir = (b - a).normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }
    let mid = (a + b) * 0.5;
    let eye = cam.eye();
    let to_cam = (eye - mid).normalize_or_zero();
    let mut perp = dir.cross(to_cam);
    if perp.length_squared() < 1e-8 {
        perp = dir.cross(cam.view_up_hint());
    }
    if perp.length_squared() < 1e-8 {
        return None;
    }
    perp = perp.normalize();
    let aspect = (viewport.width() / viewport.height().max(1.0)).max(0.01);
    let (_, half_h) = cam.viewport_half_extents(aspect);
    let world_per_px = 2.0 * half_h / viewport.height().max(1.0);
    let half_width = width_px * 0.5 * world_per_px;
    Some([
        a + perp * half_width,
        a - perp * half_width,
        b - perp * half_width,
        b + perp * half_width,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::AppState;
    use crate::model::FaceId;
    use egui::Rect as UiRect;

    fn test_viewport() -> UiRect {
        UiRect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0))
    }

    fn build_scene_with_shading(
        state: &AppState,
        mode: crate::camera::ShadingMode,
    ) -> ViewportScene {
        let mut cam = state.cam.clone();
        cam.set_shading_mode(mode);
        ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
    }

    fn state_with_one_body() -> AppState {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        let rect_lines = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            0.0,
            0.0,
            10.0,
            5.0,
            [false; 4],
        );
        state.apply(Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 7.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        });
        assert_eq!(state.doc.bodies.len(), 1);
        state
    }

    #[test]
    fn solid_shading_fills_body_with_no_wireframe_overlay() {
        use crate::camera::ShadingMode;

        let state = state_with_one_body();
        let scene = build_scene_with_shading(&state, ShadingMode::Solid);
        assert!(
            scene.wireframe_indices.is_empty(),
            "solid mode should not populate the wireframe overlay layer"
        );
    }

    #[test]
    fn wireframe_shading_skips_fill_and_populates_wireframe_layer() {
        use crate::camera::ShadingMode;

        let state = state_with_one_body();
        let solid = build_scene_with_shading(&state, ShadingMode::Solid);
        let wireframe = build_scene_with_shading(&state, ShadingMode::Wireframe);

        assert!(
            !wireframe.wireframe_indices.is_empty(),
            "wireframe mode should populate the wireframe overlay layer"
        );
        assert!(
            wireframe.indices.len() < solid.indices.len(),
            "wireframe mode ({}) should skip the body's fill triangles present in solid mode ({})",
            wireframe.indices.len(),
            solid.indices.len()
        );
    }

    #[test]
    fn solid_wireframe_shading_keeps_fill_and_adds_wireframe_overlay() {
        use crate::camera::ShadingMode;

        let state = state_with_one_body();
        let solid = build_scene_with_shading(&state, ShadingMode::Solid);
        let solid_wireframe = build_scene_with_shading(&state, ShadingMode::SolidWireframe);

        assert_eq!(
            solid_wireframe.indices.len(),
            solid.indices.len(),
            "solid+wireframe should keep the same opaque fill as solid mode"
        );
        assert!(
            !solid_wireframe.wireframe_indices.is_empty(),
            "solid+wireframe mode should also populate the wireframe overlay layer"
        );
    }

    #[test]
    fn realistic_shading_fills_body_with_no_wireframe_overlay() {
        use crate::camera::ShadingMode;

        let state = state_with_one_body();
        let solid = build_scene_with_shading(&state, ShadingMode::Solid);
        let realistic = build_scene_with_shading(&state, ShadingMode::Realistic);
        assert_eq!(
            realistic.indices.len(),
            solid.indices.len(),
            "realistic mode should fill the same triangles as solid mode, just shaded differently"
        );
        assert!(
            realistic.wireframe_indices.is_empty(),
            "realistic mode should not populate the wireframe overlay layer"
        );
    }

    #[test]
    fn realistic_shade_lights_a_face_toward_the_light_brighter_than_one_facing_away() {
        let base = Color32::from_rgb(200, 200, 200);
        let light = Vec3::new(0.0, 0.0, 1.0);
        let view = Vec3::new(0.0, 0.0, 1.0);
        let lit = realistic_shade(base, Vec3::Z, light, view);
        let unlit = realistic_shade(base, Vec3::new(1.0, 0.0, 0.0), light, view);
        assert!(
            lit.r() > unlit.r(),
            "a face pointing at the light should be brighter than one perpendicular to it"
        );
    }

    #[test]
    fn realistic_shade_adds_a_specular_highlight_near_the_reflection_direction() {
        let base = Color32::from_rgb(150, 150, 150);
        let light = Vec3::new(0.0, 0.0, 1.0);
        let view = Vec3::new(0.0, 0.0, 1.0);
        // The half-vector of light and view is straight up, so a face whose normal matches it
        // sits right at the specular peak and should be lighter than a face merely lit
        // face-on to the light but shaded with no floor-on specular contribution possible
        // (e.g. tilted away from the half-vector).
        let at_peak = realistic_shade(base, Vec3::Z, light, view);
        let off_peak = realistic_shade(base, Vec3::new(0.6, 0.0, 0.8).normalize(), light, view);
        assert!(
            at_peak.r() >= off_peak.r(),
            "the specular peak should be at least as bright as an off-peak angle"
        );
    }

    #[test]
    fn realistic_shade_never_darkens_below_ambient() {
        let base = Color32::from_rgb(180, 90, 40);
        // Facing directly away from both light and camera: diffuse and specular are both zero.
        let shaded = realistic_shade(base, Vec3::new(0.0, 1.0, 0.0), Vec3::Z, Vec3::Z);
        let ambient_only = scale_color(base, REALISTIC_AMBIENT);
        assert_eq!(shaded, ambient_only);
    }

    #[test]
    fn transparent_solid_shading_moves_body_into_the_translucent_layer() {
        use crate::camera::ShadingMode;

        let state = state_with_one_body();
        let solid = build_scene_with_shading(&state, ShadingMode::Solid);
        let transparent = build_scene_with_shading(&state, ShadingMode::TransparentSolid);

        assert!(
            transparent.plane_fill_indices.len() > solid.plane_fill_indices.len(),
            "transparent solid mode should push the body into the translucent (plane-fill) layer"
        );
        assert!(
            transparent.indices.len() < solid.indices.len(),
            "transparent solid mode should not also push the body into the opaque base layer"
        );
        assert!(transparent.wireframe_indices.is_empty());
    }

    #[test]
    fn solid_mesh_unique_edges_drops_the_coplanar_diagonal() {
        // Two coplanar triangles forming a unit-square quad, split along one diagonal (#82):
        // the shared diagonal is an internal triangulation seam, not a real edge of the flat
        // face, so it should be dropped — leaving just the 4 perimeter edges, not 5.
        let solid = crate::extrude::SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ],
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ],
            ],
        };
        let edges = solid_mesh_unique_edges(&solid);
        assert_eq!(edges.len(), 4, "expected 4 perimeter edges, got {edges:?}");
    }

    #[test]
    fn solid_mesh_unique_edges_ignores_triangle_winding() {
        // The same shared (coplanar) edge traversed in opposite directions by its two
        // triangles must still be recognized as one edge and dropped, regardless of winding —
        // otherwise it would double-count as two never-matching edges instead of vanishing.
        let solid = crate::extrude::SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ],
                [
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ],
            ],
        };
        let edges = solid_mesh_unique_edges(&solid);
        assert_eq!(edges.len(), 4, "expected 4 edges, got {edges:?}");
    }

    #[test]
    fn solid_mesh_unique_edges_keeps_a_real_crease() {
        // Two non-coplanar triangles sharing an edge (like two faces meeting at a cube
        // corner) — that shared edge is a real feature edge and must be kept.
        let solid = crate::extrude::SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                ],
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                    Vec3::new(0.0, 1.0, 1.0),
                ],
            ],
        };
        let edges = solid_mesh_unique_edges(&solid);
        let shared = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0));
        assert!(
            edges.contains(&shared) || edges.contains(&(shared.1, shared.0)),
            "the shared crease edge should be kept, got {edges:?}"
        );
    }

    #[test]
    fn solid_mesh_unique_edges_drops_cylinder_wall_seams_but_keeps_the_rims() {
        // A CIRCLE_SEGMENTS-gon prism approximating a cylinder (#101): adjacent wall facets
        // meet at only 360/48 = 7.5° — tessellation smoothness, not a feature edge — so the
        // vertical wall seams must be dropped, while the two rims (wall meets cap at 90°)
        // must be kept. Every kept edge is therefore horizontal (constant z).
        let n = crate::extrude::CIRCLE_SEGMENTS;
        let (r, h) = (12.0f32, 10.0f32);
        let pt = |i: usize, z: f32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(r * a.cos(), r * a.sin(), z)
        };
        let mut triangles = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            // Wall quad split into two triangles, plus one fan triangle per cap.
            triangles.push([pt(i, 0.0), pt(j, 0.0), pt(j, h)]);
            triangles.push([pt(i, 0.0), pt(j, h), pt(i, h)]);
            triangles.push([Vec3::new(0.0, 0.0, 0.0), pt(j, 0.0), pt(i, 0.0)]);
            triangles.push([Vec3::new(0.0, 0.0, h), pt(i, h), pt(j, h)]);
        }
        let solid = crate::extrude::SolidMesh { triangles };
        let edges = solid_mesh_unique_edges(&solid);
        assert!(
            edges.iter().all(|(a, b)| (a.z - b.z).abs() < 1e-6),
            "wall seams (non-horizontal edges) should be dropped, got {edges:?}"
        );
        assert_eq!(edges.len(), 2 * n, "expected only the two {n}-segment rims");
    }

    #[test]
    fn solid_mesh_silhouette_edges_finds_the_cylinder_sides() {
        // The same cylinder prism; viewed along -Y (front), the leftmost/rightmost vertical
        // wall seams are on the silhouette (their two wall facets face opposite ways), so at
        // least two near-vertical edges are reported (#319).
        let n = crate::extrude::CIRCLE_SEGMENTS;
        let (r, h) = (12.0f32, 10.0f32);
        let pt = |i: usize, z: f32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(r * a.cos(), r * a.sin(), z)
        };
        let mut triangles = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            triangles.push([pt(i, 0.0), pt(j, 0.0), pt(j, h)]);
            triangles.push([pt(i, 0.0), pt(j, h), pt(i, h)]);
            triangles.push([Vec3::new(0.0, 0.0, 0.0), pt(j, 0.0), pt(i, 0.0)]);
            triangles.push([Vec3::new(0.0, 0.0, h), pt(i, h), pt(j, h)]);
        }
        let solid = crate::extrude::SolidMesh { triangles };
        let sil = solid_mesh_silhouette_edges(&solid, -Vec3::Y);
        let vertical: Vec<_> = sil
            .iter()
            .filter(|(a, b)| (a.z - b.z).abs() > h * 0.5 && (a.x - b.x).abs() < r * 0.2)
            .collect();
        assert!(
            vertical.len() >= 2,
            "cylinder should have ≥2 vertical silhouette sides, got {}",
            vertical.len()
        );
    }

    #[test]
    fn solid_mesh_coplanar_faces_groups_a_split_quad_into_one_face() {
        // The same diagonally-split square as the edge test: two coplanar triangles must merge
        // into a single planar face (#144), not stay two separate triangle "faces".
        let solid = crate::extrude::SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ],
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ],
            ],
        };
        let faces = solid_mesh_coplanar_faces(&solid);
        assert_eq!(faces.len(), 1, "coplanar quad should be one face, got {faces:?}");
        assert_eq!(faces[0].len(), 2, "the face keeps both of its triangles");
    }

    #[test]
    fn solid_mesh_coplanar_faces_keeps_a_crease_as_two_faces() {
        // Two triangles sharing an edge but meeting at a crease must stay two distinct faces.
        let solid = crate::extrude::SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                ],
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                    Vec3::new(0.0, 1.0, 1.0),
                ],
            ],
        };
        assert_eq!(solid_mesh_coplanar_faces(&solid).len(), 2);
    }

    #[test]
    fn solid_mesh_coplanar_faces_finds_six_faces_on_a_box() {
        // A unit cube (12 triangles, two per side) must resolve to exactly its 6 planar faces.
        let c = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(0.0, 1.0, 1.0),
        ];
        let quad = |a: usize, b: usize, d: usize, e: usize| {
            vec![[c[a], c[b], c[d]], [c[a], c[d], c[e]]]
        };
        let mut triangles = Vec::new();
        for face in [
            quad(0, 1, 2, 3), // bottom
            quad(4, 5, 6, 7), // top
            quad(0, 1, 5, 4), // front
            quad(1, 2, 6, 5), // right
            quad(2, 3, 7, 6), // back
            quad(3, 0, 4, 7), // left
        ] {
            triangles.extend(face);
        }
        let solid = crate::extrude::SolidMesh { triangles };
        assert_eq!(solid_mesh_coplanar_faces(&solid).len(), 6);
    }

    /// #158: a cylinder viewed from the side must expose its two view-tangent wall seams as
    /// silhouette edges (its wall seams are smooth, so the permanent wireframe drops them all).
    #[test]
    fn cylinder_side_view_has_vertical_smooth_silhouette_edges() {
        let n = crate::extrude::CIRCLE_SEGMENTS;
        let (r, h) = (12.0f32, 10.0f32);
        let pt = |i: usize, z: f32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(r * a.cos(), r * a.sin(), z)
        };
        let mut triangles = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            triangles.push([pt(i, 0.0), pt(j, 0.0), pt(j, h)]);
            triangles.push([pt(i, 0.0), pt(j, h), pt(i, h)]);
            triangles.push([Vec3::new(0.0, 0.0, 0.0), pt(j, 0.0), pt(i, 0.0)]);
            triangles.push([Vec3::new(0.0, 0.0, h), pt(i, h), pt(j, h)]);
        }
        let solid = crate::extrude::SolidMesh { triangles };

        // Side view from +x: the tangent seams sit near y = ±r.
        let eye = Vec3::new(1000.0, 0.0, h / 2.0);
        let edges = solid_mesh_smooth_silhouette_edges(&solid, eye);
        assert!(
            (2..=4).contains(&edges.len()),
            "expected the two view-tangent seams (plus at most grazing neighbors), got {}",
            edges.len()
        );
        for (a, b) in &edges {
            assert!(
                (a.z - b.z).abs() > h * 0.9,
                "silhouette edges on the wall are vertical seams, got {a:?}-{b:?}"
            );
            assert!(
                a.y.abs() > r * 0.9,
                "tangent seams sit near y = ±r for a +x view, got y = {}",
                a.y
            );
        }

        // Looking straight down the axis there is no wall silhouette (the rims cover it).
        let edges = solid_mesh_smooth_silhouette_edges(&solid, Vec3::new(0.0, 0.0, 1000.0));
        assert!(edges.is_empty(), "no smooth silhouette from head-on, got {}", edges.len());
    }

    #[test]
    fn is_feature_edge_ignores_smooth_tessellation_angles_but_keeps_chamfer_angles() {
        // The crease threshold (#101) must sit between the largest smooth-tessellation facet
        // angle (48-gon cylinder wall: 7.5°) and the smallest genuine feature angle this app
        // produces (shallow chamfer bevels: ~30°).
        let tilted = |deg: f32| {
            let r = deg.to_radians();
            Vec3::new(r.sin(), 0.0, r.cos())
        };
        assert!(
            !is_feature_edge(&[Vec3::Z, tilted(7.5)]),
            "a 48-gon cylinder's 7.5° facet seam is not a feature edge"
        );
        assert!(
            is_feature_edge(&[Vec3::Z, tilted(30.0)]),
            "a 30° chamfer bevel crease is a feature edge"
        );
    }

    #[test]
    fn realistic_shade_headlights_a_camera_facing_face_the_fixed_light_misses() {
        // #102: a face square to the camera but perpendicular to the fixed light used to sit
        // at the ambient floor (nearly black head-on). The headlight term must lift it well
        // above ambient.
        let base = Color32::from_rgb(200, 200, 200);
        let light = Vec3::Z;
        let view = Vec3::new(0.0, -1.0, 0.0);
        let facing_camera = realistic_shade(base, Vec3::new(0.0, -1.0, 0.0), light, view);
        let ambient_only = scale_color(base, REALISTIC_AMBIENT);
        assert!(
            facing_camera.r() as i32 >= ambient_only.r() as i32 + 40,
            "camera-facing face should be clearly brighter than the ambient floor: {} vs {}",
            facing_camera.r(),
            ambient_only.r()
        );
    }

    #[test]
    fn plane_creation_preview_adds_outline_geometry() {
        let state = AppState::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let base = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let preview_plane = state.doc.construction_planes[0].clone();
        let with_preview = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: Some(ViewportPlanePreview {
                plane: preview_plane,
                dependents: None,
                dim_outline: false,
            }),
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            with_preview.overlay_indices.len() > base.overlay_indices.len(),
            "plane creation preview should add outline triangles"
        );
    }

    /// #550: a curve's tangent handles show only when the curve/its endpoints are selected or
    /// hovered, or a handle is being manipulated — not for every curve at rest.
    #[test]
    fn bezier_handles_show_only_when_relevant() {
        use crate::construction::PickTargetKind;
        use crate::hierarchy::SceneElement;
        use crate::model::{ConstraintPoint, LineEnd};
        use crate::selection::SceneSelection;

        let empty = SceneSelection::default();
        // At rest: nothing selected or hovered → no handles.
        assert!(!bezier_handles_relevant(0, &empty, &None, &[]));
        // A dragged/selected handle keeps its curve's handles up.
        assert!(bezier_handles_relevant(0, &empty, &None, &[(0, true)]));
        assert!(!bezier_handles_relevant(1, &empty, &None, &[(0, true)]));
        // The curve selected.
        let mut sel = SceneSelection::default();
        sel.insert(SceneElement::Line(0));
        assert!(bezier_handles_relevant(0, &sel, &None, &[]));
        assert!(!bezier_handles_relevant(1, &sel, &None, &[]));
        // One of its endpoints selected.
        let mut sel_pt = SceneSelection::default();
        sel_pt.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: 2,
            end: LineEnd::End,
        }));
        assert!(bezier_handles_relevant(2, &sel_pt, &None, &[]));
        assert!(!bezier_handles_relevant(3, &sel_pt, &None, &[]));
        // Hovering the curve (as a pick target).
        let hover = Some(ViewportHoverHighlight::PickTarget(PickTargetKind::Line(4)));
        assert!(bezier_handles_relevant(4, &empty, &hover, &[]));
        assert!(!bezier_handles_relevant(5, &empty, &hover, &[]));
    }

    #[test]
    fn hover_highlight_adds_mesh_geometry() {
        let state = AppState::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let base = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: crate::construction::PICK_HOVER_RGBA,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let with_hover = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: Some(ViewportHoverHighlight::SketchFace(
                FaceId::ConstructionPlane(0),
            )),
            extra_pick_highlights: Vec::new(),
            hover_color: crate::construction::PICK_HOVER_RGBA,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let hover_indices =
            with_hover.overlay_indices.len() - base.overlay_indices.len();
        assert_eq!(
            hover_indices, 6,
            "construction-plane hover should add only a biased fill quad"
        );
    }

    /// #153: pick-target hovers (an edge/vertex/point about to be selected) draw in the
    /// depth-test-disabled layer, so a concave joint's adjoining faces can never bury a
    /// chunk of the highlight inside the body.
    #[test]
    fn pick_target_hover_draws_depth_test_disabled() {
        let state = AppState::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |hover: Option<ViewportHoverHighlight>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        let hovered = build(Some(ViewportHoverHighlight::PickTarget(
            crate::construction::PickTargetKind::BodyEdge {
                body: 0,
                a: Vec3::new(0.0, 0.0, 0.0),
                b: Vec3::new(50.0, 0.0, 0.0),
            },
        )));
        assert!(
            hovered.wireframe_indices.len() > base.wireframe_indices.len(),
            "edge hover must land in the depth-disabled (wireframe) layer"
        );
        assert_eq!(
            hovered.overlay_indices.len(),
            base.overlay_indices.len(),
            "edge hover must not draw in the depth-tested overlay layer"
        );
    }

    #[test]
    fn plane_gizmo_adds_mesh_geometry() {
        use crate::construction::PlaneReference;

        let state = AppState::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let base = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let gizmo = ViewportPlaneGizmo {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "XY".into(),
            },
            offset: 12.0,
            angle_deg: 0.0,
            color: Color32::from_rgb(240, 200, 120),
            hover: None,
        };
        let with_gizmo = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,

            plane_gizmo: Some(gizmo),
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            with_gizmo.indices.len() > base.indices.len(),
            "plane gizmo should add triangles to the viewport scene"
        );
    }

    #[test]
    fn extrude_gizmo_adds_mesh_geometry() {
        let state = AppState::default();
        let cam = state.cam.clone();
        let base = build_scene_for_doc(&state);
        let mut input = ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        };
        input.extrude_gizmo = Some(ViewportExtrudeGizmo {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            offset: 12.0,
            color: Color32::from_rgb(240, 200, 120),
            hovered: false,
        });
        let with_gizmo = ViewportScene::build(&input);
        assert!(
            with_gizmo.indices.len() > base.indices.len(),
            "extrude gizmo should add triangles to the viewport scene"
        );
    }

    /// #464: grid steps follow the document's unit system — powers of ten of a mm for
    /// metric, the quarter-inch/inch/foot ladder for imperial — with the heavy step
    /// always an exact multiple of the fine one.
    #[test]
    fn grid_steps_follow_document_units() {
        let close = |(a, b): (f32, f32), (x, y): (f32, f32)| {
            assert!((a - x).abs() < x * 1e-4, "fine {a} != {x}");
            assert!((b - y).abs() < y * 1e-4, "coarse {b} != {y}");
        };
        close(grid_steps_for_unit(LengthUnit::Mm, 4.0), (10.0, 100.0));
        close(grid_steps_for_unit(LengthUnit::Mm, 10.0), (10.0, 100.0));
        close(grid_steps_for_unit(LengthUnit::Cm, 0.05), (0.1, 1.0));
        close(grid_steps_for_unit(LengthUnit::M, 400.0), (1000.0, 10000.0));
        // 1 in fine under a 1 ft heavy line.
        close(grid_steps_for_unit(LengthUnit::In, 20.0), (25.4, 304.8));
        // Quarter inches under whole inches.
        close(grid_steps_for_unit(LengthUnit::In, 5.0), (6.35, 25.4));
        close(grid_steps_for_unit(LengthUnit::Ft, 20.0), (25.4, 304.8));
        // Zoomed way out: 10 ft under 100 ft.
        close(grid_steps_for_unit(LengthUnit::Ft, 400.0), (3048.0, 30480.0));
        // Degenerate input falls back to a sane metric default.
        close(grid_steps_for_unit(LengthUnit::Mm, f32::NAN), (10.0, 100.0));
    }

    #[test]
    fn scene_always_includes_ground_grid_and_clear_color() {
        let state = AppState::default();
        let cam = state.cam.clone();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(!scene.vertices.is_empty());
        assert!(!scene.indices.is_empty());
        assert_eq!(scene.clear_color[0], color32_to_gpu(Color32::from_gray(28))[0]);
    }

    fn count_opaque_stroke_vertices(scene: &ViewportScene, stroke: Color32) -> usize {
        let gpu = color32_to_gpu(stroke);
        scene
            .vertices
            .iter()
            .filter(|v| {
                v.color[3] > 0.99
                    && (v.color[0] - gpu[0]).abs() < 0.02
                    && (v.color[1] - gpu[1]).abs() < 0.02
                    && (v.color[2] - gpu[2]).abs() < 0.02
            })
            .count()
    }

    fn build_scene_for_doc(state: &AppState) -> ViewportScene {
        let cam = state.cam.clone();
        ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
    }

    fn commit_test_rectangle(state: &mut AppState) {
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::ZERO,
            texts: ["10".into(), "5".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(10.0, 5.0, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        state.apply(crate::actions::Action::CommitRectangle);
    }

    fn commit_test_line(state: &mut AppState) {
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 0.0,
            bezier: None,
            dimension: None,
        });
    }

    /// Three lines (0, 1, 2) closed into a triangle via Coincident constraints (#66).
    fn commit_test_triangle_loop(state: &mut AppState) {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, LineEnd};

        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 0.0,
            bezier: None,
            dimension: None,
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 10.0,
            y0: 0.0,
            x1: 5.0,
            y1: 8.0,
            bezier: None,
            dimension: None,
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 5.0,
            y0: 8.0,
            x1: 0.0,
            y1: 0.0,
            bezier: None,
            dimension: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        let coincident = |a, b| Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(a),
                b: ConstraintEntity::Point(b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        };
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        state.doc.constraints.push(coincident(point(0, LineEnd::End), point(1, LineEnd::Start)));
        state.doc.constraints.push(coincident(point(1, LineEnd::End), point(2, LineEnd::Start)));
        state.doc.constraints.push(coincident(point(2, LineEnd::End), point(0, LineEnd::Start)));
    }

    /// Rectangle and circle both at index 0 on the ground plane, overlapping (#3).
    fn commit_overlapping_rect_and_circle(state: &mut AppState) {
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::ZERO,
            texts: ["80".into(), "50".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(80.0, 50.0, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        state.apply(crate::actions::Action::CommitRectangle);
        state.creating_circle = Some(crate::actions::CreatingCircle {
            origin: glam::Vec3::new(40.0, 25.0, 0.0),
            text: "40".into(),
            last_mouse: glam::Vec3::new(60.0, 25.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            anchor: crate::actions::CircleAnchor::Center,
        });
        state.apply(crate::actions::Action::CommitCircle);
    }

    #[test]
    fn editing_an_extrusion_hides_its_committed_body() {
        use crate::actions::{Action, Tool};
        use crate::model::ExtrudeFace;

        let mut state = AppState::default();
        commit_test_rectangle(&mut state);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
        });
        state.apply(Action::SetExtrudeDistance { distance: 7.0 });
        state.apply(Action::CommitExtrusion);
        assert_eq!(state.doc.bodies.len(), 1);

        let cam = state.cam.clone();
        let build = |editing: Option<usize>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport: test_viewport(),
                palette: ViewportPalette::default(),
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: editing,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };

        let with_body = build(None);
        let editing = build(Some(0));
        assert!(
            editing.vertices.len() < with_body.vertices.len(),
            "editing scene ({}) should drop the committed body geometry present without editing ({})",
            editing.vertices.len(),
            with_body.vertices.len()
        );
    }

    #[test]
    fn extruded_body_adds_solid_triangles() {
        let mut state = AppState::default();
        commit_test_rectangle(&mut state);
        let sketch = state.doc.lines[0].sketch;
        let before = build_scene_for_doc(&state).vertices.len();

        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 8.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        });

        let scene = build_scene_for_doc(&state);
        // A box solid adds 12 triangles = 36 vertices.
        assert!(
            scene.vertices.len() >= before + 36,
            "extruded body should add solid triangles: {} -> {}",
            before,
            scene.vertices.len()
        );
    }

    #[test]
    fn extruded_top_cap_on_slanted_target_plane_is_biased_toward_camera() {
        let mut state = AppState::default();
        commit_test_rectangle(&mut state);
        let sketch = state.doc.lines[0].sketch;

        let plane_origin = Vec3::new(0.0, 0.0, 12.0);
        let plane_normal = Vec3::new(0.0, 0.4, 1.0).normalize();
        let mut slanted = crate::face::default_xy_plane();
        slanted.origin = plane_origin;
        slanted.normal = plane_normal;
        state.doc.construction_planes.push(slanted);

        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 6.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        });
        state.doc.extrusions[0].target = Some(crate::model::ExtrudeTarget::Plane(1));

        let raw = crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[0]).unwrap();
        let cap_vertex = *raw
            .triangles
            .iter()
            .flat_map(|t| t.iter())
            .find(|p| ((**p - plane_origin).dot(plane_normal)).abs() < 1e-3)
            .expect("expected at least one top-cap vertex on the target plane");

        let raw_unbiased_count = raw
            .triangles
            .iter()
            .flat_map(|t| t.iter())
            .filter(|p| (**p - cap_vertex).length() < 1e-5)
            .count();

        let scene = build_scene_for_doc(&state);
        let eye = state.cam.eye();
        let biased = offset_toward_camera(cap_vertex, plane_normal, eye, SOLID_CAP_DEPTH_BIAS);
        let scene_unbiased_count = scene
            .vertices
            .iter()
            .filter(|v| (Vec3::from(v.position) - cap_vertex).length() < 1e-5)
            .count();

        assert!(
            scene
                .vertices
                .iter()
                .any(|v| (Vec3::from(v.position) - biased).length() < 1e-4),
            "expected a rasterized vertex at the camera-biased cap position {biased:?}"
        );
        assert!(
            scene_unbiased_count < raw_unbiased_count,
            "expected fewer unbiased copies of the cap corner after biasing: raw={raw_unbiased_count} scene={scene_unbiased_count}"
        );

        let base_vertex = *raw
            .triangles
            .iter()
            .flat_map(|t| t.iter())
            .find(|p| ((**p - plane_origin).dot(plane_normal)).abs() > 1.0)
            .expect("expected a non-cap vertex");
        assert!(
            scene
                .vertices
                .iter()
                .any(|v| (Vec3::from(v.position) - base_vertex).length() < 1e-4),
            "non-cap vertices should be rasterized at their raw position"
        );
    }

    #[test]
    fn extrude_preview_to_slanted_target_plane_shows_slanted_top() {
        // The in-progress (uncommitted) ghost preview should show the actual slanted shape
        // once the gizmo has snapped to a slanted target plane (#63).
        let mut state = AppState::default();
        commit_test_rectangle(&mut state);
        let sketch = state.doc.lines[0].sketch;

        let plane_origin = Vec3::new(0.0, 0.0, 12.0);
        let plane_normal = Vec3::new(0.0, 0.4, 1.0).normalize();
        let mut slanted = crate::face::default_xy_plane();
        slanted.origin = plane_origin;
        slanted.normal = plane_normal;
        state.doc.construction_planes.push(slanted);

        let preview = crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 6.0,
            target: Some(crate::model::ExtrudeTarget::Plane(1)),
            expression: String::new(),
            name: None,
            deleted: false,
            symmetric: false,
            edge_treatments: Vec::new(),
        };

        let cam = state.cam.clone();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: Some(preview.clone()),
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            editing_extrusion: None,
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });

        let raw = crate::extrude::extrusion_mesh(&state.doc, &preview).unwrap();
        let cap_heights: Vec<f32> = raw
            .triangles
            .iter()
            .flat_map(|t| t.iter())
            .filter(|p| ((**p - plane_origin).dot(plane_normal)).abs() < 1e-3)
            .map(|p| p.z)
            .collect();
        let zmin = cap_heights.iter().cloned().fold(f32::MAX, f32::min);
        let zmax = cap_heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(
            zmax - zmin > 1.0,
            "expected the raw preview mesh itself to be slanted, spread {}",
            zmax - zmin
        );
        assert!(
            scene.vertices.len() >= raw.triangles.len() * 3,
            "expected the slanted preview solid's triangles in the rasterized scene"
        );
    }

    #[test]
    fn overlapping_rect_and_circle_on_ground_plane_have_distinct_fill_depths() {
        let mut state = AppState::default();
        commit_overlapping_rect_and_circle(&mut state);
        let scene = build_scene_for_doc(&state);
        let cam = Camera::default();
        let eye = cam.eye();
        let sketch = state.doc.lines[0].sketch;
        let frame = crate::face::sketch_geometry_frame(&state.doc, sketch).expect("sketch frame");
        let overlap = Vec3::new(40.0, 25.0, 0.0);
        // The rectangle is a `Polygon` whose fill uses lane 2 keyed by its first line index;
        // the circle's fill uses lane 1 keyed by its circle index. #3 keeps overlapping
        // coplanar shapes on distinct depth biases so they never z-fight.
        let rect_bias = shape_fill_depth_bias_laned(0, 2);
        let circle_bias = shape_fill_depth_bias_laned(0, 1);
        assert!(
            (rect_bias - circle_bias).abs() > 1e-6,
            "rect and circle fills must not share a depth bias: rect={rect_bias} circle={circle_bias}"
        );
        let rect_corner = offset_toward_camera(Vec3::ZERO, frame.normal, eye, rect_bias);
        let circle_center = offset_toward_camera(overlap, frame.normal, eye, circle_bias);

        let rect_mesh_z = mesh_z_closest_to(&scene, rect_corner).expect("rectangle fill in mesh");
        let circle_mesh_z =
            mesh_z_closest_to(&scene, circle_center).expect("circle fill in mesh");
        assert!(
            (rect_mesh_z - rect_corner.z).abs() < 1e-4,
            "rectangle mesh z {rect_mesh_z} should match biased corner {}",
            rect_corner.z
        );
        assert!(
            (circle_mesh_z - circle_center.z).abs() < 1e-4,
            "circle mesh z {circle_mesh_z} should match biased center {}",
            circle_center.z
        );
        assert!(
            (circle_mesh_z - rect_mesh_z).abs() > 1e-5,
            "mesh depths must differ where shapes overlap (rect={rect_mesh_z} circle={circle_mesh_z})"
        );
    }

    #[test]
    fn committed_sketch_fills_go_in_stencil_masked_layer() {
        // Committed coplanar sketch fills route into the dedicated stencil-masked
        // sketch_fill layer so each pixel is painted once (#3).
        let mut state = AppState::default();
        commit_overlapping_rect_and_circle(&mut state);
        let scene = build_scene_for_doc(&state);
        assert!(
            !scene.sketch_fill_indices.is_empty(),
            "committed rect + circle fills should populate the stencil-masked layer"
        );
        let frame =
            crate::face::sketch_geometry_frame(&state.doc, state.doc.lines[0].sketch).unwrap();
        let cam = Camera::default();
        let overlap = offset_toward_camera(
            Vec3::new(40.0, 25.0, 0.0),
            frame.normal,
            cam.eye(),
            shape_fill_depth_bias_laned(0, 0),
        );
        assert!(mesh_z_closest_to(&scene, overlap).is_some());
    }

    #[test]
    fn hovering_a_sketch_face_lifts_its_fill_off_the_plane() {
        let mut state = AppState::default();
        commit_test_rectangle(&mut state);
        let cam = state.cam.clone();
        let base = build_scene_for_doc(&state);
        let with_hover = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: Some(ViewportHoverHighlight::SketchFace(FaceId::Polygon(vec![
                0, 1, 2, 3,
            ]))),
            extra_pick_highlights: Vec::new(),
            hover_color: crate::construction::PICK_HOVER_RGBA,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let added = &with_hover.vertices[base.vertices.len()..];
        assert!(!added.is_empty(), "hover should add geometry");
        let fill_verts = added
            .iter()
            .filter(|v| (v.position[2] - HOVER_FILL_DEPTH_BIAS).abs() < 1e-4)
            .count();
        assert!(
            fill_verts >= 6,
            "expected the hover fill lifted to z={HOVER_FILL_DEPTH_BIAS}, found {fill_verts} such vertices"
        );
    }

    #[test]
    fn construction_planes_render_fill_without_edge_strokes() {
        use crate::hierarchy::SceneElement;

        let mut hidden = AppState::default();
        hidden
            .element_visibility
            .set_visible(SceneElement::ConstructionPlane(0), false);

        let with_plane = build_scene_for_doc(&AppState::default());
        let without_plane = build_scene_for_doc(&hidden);
        let plane_indices =
            with_plane.plane_fill_indices.len() - without_plane.plane_fill_indices.len();
        assert_eq!(
            plane_indices, 6,
            "each construction plane should add only two fill triangles"
        );
    }

    #[test]
    fn solid_line_strokes_use_rectangle_stroke_color() {
        let mut state = AppState::default();
        commit_test_line(&mut state);
        let scene = build_scene_for_doc(&state);
        let strokes =
            count_opaque_stroke_vertices(&scene, ViewportPalette::default().rect_line);
        assert!(
            strokes > 0,
            "a solid line should render with the shared rect/circle/line stroke color"
        );
    }

    #[test]
    fn closed_line_loop_gets_a_sketch_fill_like_a_rect_or_circle() {
        let mut state = AppState::default();
        commit_test_triangle_loop(&mut state);
        let scene = build_scene_for_doc(&state);
        assert!(
            !scene.sketch_fill_indices.is_empty(),
            "a closed triangle of lines should fill the same as a rect/circle face (#66)"
        );
    }

    #[test]
    fn rectangle_adds_fill_and_edge_triangles() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::ZERO,
            texts: ["10".into(), "5".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(10.0, 5.0, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        state.apply(crate::actions::Action::CommitRectangle);
        let cam = state.cam.clone();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(scene.vertices.len() >= 8);
        assert!(scene.indices.len() >= 18);
    }

    #[test]
    fn circle_uses_more_segments_than_old_cpu_path() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.creating_circle = Some(crate::actions::CreatingCircle {
            origin: glam::Vec3::ZERO,
            text: "20".into(),
            last_mouse: glam::Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            anchor: crate::actions::CircleAnchor::Center,
        });
        state.apply(crate::actions::Action::CommitCircle);
        let cam = state.cam.clone();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(scene.indices.len() > CIRCLE_SEGMENTS);
    }

    #[test]
    fn sketch_dimmed_geometry_stays_readable() {
        let base = Color32::from_rgb(120, 170, 240);
        let dimmed = sketch_color(base, true);
        let legacy = base.gamma_multiply(0.28);
        assert!(
            dimmed.r() > legacy.r(),
            "outside-sketch geometry should be brighter than the old 0.28 multiplier"
        );
        assert!(
            dimmed.r() < base.r(),
            "outside-sketch geometry should still be de-emphasized"
        );
    }

    #[test]
    fn sketch_ground_stays_brighter_than_other_dimmed_geometry() {
        let base = Color32::from_rgb(120, 170, 240);
        let ground = sketch_ground_color(base, true);
        let other = sketch_color(base, true);
        assert!(ground.r() > other.r());
        assert!(ground.r() < base.r());
    }

    #[test]
    fn shape_fill_depth_bias_increases_with_index() {
        assert!(shape_fill_depth_bias(2) > shape_fill_depth_bias(1));
        assert!(shape_fill_depth_bias(1) > shape_fill_depth_bias(0));
        assert!(shape_fill_depth_bias(0) > plane_fill_depth_bias(0));
    }

    #[test]
    fn stroke_depth_bias_beats_shape_fill_bias() {
        assert!(STROKE_DEPTH_BIAS > shape_fill_depth_bias(0));
        assert!(STROKE_DEPTH_BIAS > plane_fill_depth_bias(0));
    }

    /// #143: the committed shape-fill band must stay strictly below both the hover fill and the
    /// strokes for *every* shape index (not just low ones), so a hover over overlapping coplanar
    /// faces never z-fights with a high-index committed fill and lines/strokes stay on top.
    #[test]
    fn committed_fill_band_stays_below_hover_and_strokes() {
        for index in 0..64usize {
            for lane in 0..3usize {
                let bias = shape_fill_depth_bias_laned(index, lane);
                assert!(
                    bias < HOVER_FILL_DEPTH_BIAS,
                    "shape fill (index {index}, lane {lane}) bias {bias} reaches the hover layer {HOVER_FILL_DEPTH_BIAS}"
                );
                assert!(
                    bias < STROKE_DEPTH_BIAS,
                    "shape fill (index {index}, lane {lane}) bias {bias} reaches the stroke layer {STROKE_DEPTH_BIAS}"
                );
            }
        }
        assert!(HOVER_FILL_DEPTH_BIAS < STROKE_DEPTH_BIAS);
    }

    fn mesh_z_closest_to(scene: &ViewportScene, target: Vec3) -> Option<f32> {
        // Committed sketch fills live in the stencil-masked sketch_fill layer (#3).
        scene
            .sketch_fill_indices
            .iter()
            .map(|&index| Vec3::from_array(scene.vertices[index as usize].position))
            .min_by(|a, b| {
                (a - target)
                    .length_squared()
                    .partial_cmp(&(b - target).length_squared())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.z)
    }

    #[test]
    fn coplanar_shape_types_never_share_a_depth_bias() {
        // The original bug: a rectangle and a circle at the same per-type index got the
        // identical bias and z-fought. Lanes must keep every (index, lane) pair distinct.
        for index in 0..16usize {
            let rect = shape_fill_depth_bias_laned(index, 0);
            // No circle at any index may equal this rectangle's bias.
            for other in 0..16usize {
                let circle = shape_fill_depth_bias_laned(other, 1);
                assert!(
                    (rect - circle).abs() > 1e-6,
                    "rect {index} and circle {other} share bias {rect}"
                );
            }
        }
        // Rect 0 (the reported case) is specifically separated from circle 0.
        assert!(
            (shape_fill_depth_bias_laned(0, 0) - shape_fill_depth_bias_laned(0, 1)).abs() > 1e-6
        );
    }

    #[test]
    fn stroke_depth_bias_beats_grid_depth_bias() {
        assert!(STROKE_DEPTH_BIAS > GRID_DEPTH_BIAS);
    }

    fn count_indices_with_color(scene: &ViewportScene, indices: &[u32], color: Color32) -> usize {
        let target = color32_to_gpu(color);
        indices
            .iter()
            .filter(|&&index| scene.vertices[index as usize].color == target)
            .count()
    }

    #[test]
    fn selected_line_uses_highlight_color_only() {
        use crate::model::{FaceId, Line, ShapeKind};
        use crate::selection::SceneSelection;

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);

        let palette = ViewportPalette::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let empty_selection = SceneSelection::default();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            SceneElement::Line(0),
            false,
        );

        let unselected = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette,
            sketch_session: None,
            selection: &empty_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let selected_scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette,
            sketch_session: None,
            selection: &selected,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });

        let unselected_base = count_indices_with_color(
            &unselected,
            &unselected.indices,
            palette.rect_line,
        );
        let selected_base = count_indices_with_color(
            &selected_scene,
            &selected_scene.indices,
            palette.rect_line,
        );
        let selected_highlight = count_indices_with_color(
            &selected_scene,
            &selected_scene.overlay_indices,
            palette.dim_edge_highlight,
        );

        assert!(
            unselected_base > 0,
            "unselected line should render in the base layer"
        );
        assert_eq!(
            selected_base, 0,
            "selected line should not render with base stroke color"
        );
        assert!(
            selected_highlight > 0,
            "selected line should render with highlight color"
        );
    }






    /// #161: hovering an elements-pane row highlights that element in the viewport — a
    /// hovered body draws its aura in the hover color (depth-disabled layer).
    #[test]
    fn pane_hover_element_highlights_body_in_viewport() {
        let state = state_with_one_body();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |hover: Option<ViewportHoverHighlight>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        let hovered = build(Some(ViewportHoverHighlight::Element(
            crate::hierarchy::SceneElement::Body(0),
        )));
        // Hover recolors the body fill (#455): shaded triangles pick up the hover fill's
        // channel ratios (r > g > b), which the plain render never produces.
        let hover_tinted = |scene: &ViewportScene| {
            scene
                .vertices
                .iter()
                .filter(|v| {
                    let [r, g, b, a] = v.color;
                    a > 0.0 && r > g && g > b && r > 0.3
                })
                .count()
        };
        assert!(
            hover_tinted(&hovered) > hover_tinted(&base),
            "hovering a body must recolor its fill"
        );
    }

    /// #174: a selected body's fill shifts to the saturated selection blue — some base-layer
    /// vertex carries the selected hue (flat shading preserves channel ratios), and none does
    /// when unselected.
    #[test]
    fn selected_body_fill_uses_saturated_blue() {
        let state = state_with_one_body();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(0),
            false,
        );
        let has_selected_hue = |scene: &ViewportScene| {
            scene.vertices.iter().any(|v| {
                let [r, g, b, _] = v.color;
                // scale_color multiplies all channels by one shade factor, so the ratios of
                // SOLID_FILL_SELECTED (112:152:224) survive shading.
                b > 0.05
                    && (r / b - 112.0 / 224.0).abs() < 0.02
                    && (g / b - 152.0 / 224.0).abs() < 0.02
            })
        };
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |sel: &SceneSelection| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: None,
                selection: sel,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(&SceneSelection::default());
        let with_selection = build(&selected);
        assert!(!has_selected_hue(&base), "unselected body must keep the neutral fill");
        assert!(has_selected_hue(&with_selection), "selected body must use the saturated blue");
    }

    /// #213: a body in a destructive picker's `cut_highlight_bodies` fills the red cut hue,
    /// taking precedence over the blue selection fill.
    #[test]
    fn cut_highlight_body_fill_uses_red() {
        let state = state_with_one_body();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(0),
            false,
        );
        // SOLID_FILL_CUT (210:120:120) has r > g ≈ b; flat shading preserves channel ratios.
        let has_cut_hue = |scene: &ViewportScene| {
            scene.vertices.iter().any(|v| {
                let [r, g, b, _] = v.color;
                r > 0.05
                    && (g / r - 120.0 / 210.0).abs() < 0.02
                    && (b / r - 120.0 / 210.0).abs() < 0.02
            })
        };
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |cut: Vec<usize>, sel: &SceneSelection| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: None,
                selection: sel,
                cut_highlight_bodies: cut,
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_mirror_ghost: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        assert!(
            !has_cut_hue(&build(Vec::new(), &selected)),
            "a merely-selected body must not use the red cut fill"
        );
        assert!(
            has_cut_hue(&build(vec![0], &selected)),
            "a cut-highlighted body must fill red even when also selected"
        );
    }


    #[test]
    fn constraint_connectors_add_overlay_geometry() {
        use crate::constraint_viewport::viewport_constraints_for_selection;
        use crate::hierarchy::ElementVisibility;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, FaceId, Line, ShapeKind};
        use crate::selection::SceneSelection;

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(0),
            false,
        );
        let graphics = viewport_constraints_for_selection(
            &state.doc,
            &ElementVisibility::default(),
            &selection,
            &std::collections::HashSet::new(),
        );
        let cam = state.cam.clone();
        let without = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let with = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: Some(&graphics),
            constraint_connector_color: Some(Color32::from_rgb(255, 205, 88)),
        });
        assert!(with.overlay_indices.len() > without.overlay_indices.len());
        assert_eq!(graphics.len(), 1);
    }

    #[test]
    fn element_strokes_sit_closer_to_camera_than_coplanar_grid() {
        let cam = Camera::default();
        let eye = cam.eye();
        let on_plane = Vec3::new(10.0, 10.0, 0.0);
        let grid = offset_toward_camera(on_plane, Vec3::Z, eye, GRID_DEPTH_BIAS);
        let (stroke_a, _) =
            offset_segment_toward_camera(on_plane, on_plane + Vec3::X, eye, STROKE_DEPTH_BIAS);
        assert!(
            (eye - stroke_a).length() < (eye - grid).length(),
            "element strokes should render above coplanar grid lines"
        );
    }

    #[test]
    fn ground_grid_sits_behind_coincident_unbiased_geometry_from_above_and_below() {
        // #78: an extruded body's base cap sits exactly at z=0, unbiased, same as the ground
        // sketch plane it was drawn on. The grid must lose that depth tie regardless of which
        // side of the plane the camera is on, or it z-fights with (and can appear to slice
        // through) the body when viewed from below.
        let on_plane = Vec3::new(5.0, 5.0, 0.0);
        for eye in [Vec3::new(0.0, -20.0, 20.0), Vec3::new(0.0, -20.0, -20.0)] {
            let grid = offset_toward_camera(on_plane, Vec3::Z, eye, GRID_DEPTH_BIAS);
            let unbiased_body_cap = on_plane;
            assert!(
                (eye - grid).length() > (eye - unbiased_body_cap).length(),
                "grid should sit behind coincident unbiased geometry when viewed from {eye:?}"
            );
        }
    }

    #[test]
    fn line_segments_are_biased_toward_camera_over_coplanar_fills() {
        let cam = Camera::default();
        let eye = cam.eye();
        let on_plane = Vec3::new(10.0, 0.0, 0.0);
        let fill = offset_toward_camera(on_plane, Vec3::Z, eye, shape_fill_depth_bias(0));
        let (stroke_a, stroke_b) =
            offset_segment_toward_camera(on_plane, on_plane + Vec3::X, eye, STROKE_DEPTH_BIAS);
        let fill_dist = (eye - fill).length();
        let stroke_dist = (eye - stroke_a).length();
        assert!(
            stroke_dist < fill_dist,
            "strokes should sit closer to the camera than coplanar face fills"
        );
        assert_eq!(stroke_a.z, stroke_b.z);
    }

    #[test]
    fn shape_fills_sit_above_coplanar_plane_toward_camera() {
        let cam = Camera::default();
        let eye = cam.eye();
        let on_plane = Vec3::new(10.0, 10.0, 0.0);
        let plane = offset_toward_camera(on_plane, Vec3::Z, eye, plane_fill_depth_bias(0));
        let shape = offset_toward_camera(on_plane, Vec3::Z, eye, shape_fill_depth_bias(0));
        assert!(shape.z > plane.z);
    }

    #[test]
    fn hover_fill_sits_above_committed_fills_and_below_strokes() {
        let cam = Camera::default();
        let eye = cam.eye();
        let on_plane = Vec3::new(10.0, 10.0, 0.0);
        // Even a handful of stacked coplanar fills stay behind the hover lift.
        let committed = offset_toward_camera(on_plane, Vec3::Z, eye, shape_fill_depth_bias_laned(4, 1));
        let hover = offset_toward_camera(on_plane, Vec3::Z, eye, HOVER_FILL_DEPTH_BIAS);
        let stroke = offset_toward_camera(on_plane, Vec3::Z, eye, STROKE_DEPTH_BIAS);
        assert!((eye - hover).length() < (eye - committed).length(), "hover above committed fills");
        assert!((eye - stroke).length() < (eye - hover).length(), "strokes above hover fill");
    }

    #[test]
    fn higher_shape_index_wins_coplanar_overlap() {
        let cam = Camera::default();
        let eye = cam.eye();
        let p = Vec3::new(0.0, 0.0, 0.0);
        let a = offset_toward_camera(p, Vec3::Z, eye, shape_fill_depth_bias(0));
        let b = offset_toward_camera(p, Vec3::Z, eye, shape_fill_depth_bias(3));
        assert!(b.z > a.z);
    }

    #[test]
    fn committed_dimension_labels_add_text_and_line_geometry() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.creating_rect = Some(crate::actions::CreatingRect {
            origin: glam::Vec3::ZERO,
            texts: ["40".into(), "20".into()],
            focused: 0,
            last_mouse: glam::Vec3::new(40.0, 20.0, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
            anchor: crate::actions::RectAnchor::Corner,
        });
        state.apply(crate::actions::Action::CommitRectangle);
        let session = state.sketch_session.unwrap();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let view = crate::dimensions::PlanarLabelView::from_camera_and_plane(&cam, glam::Vec3::Z);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        // The rectangle is four lines plus geometric constraints (#66); the width dimension
        // is the first constraint that carries an evaluated length.
        let width_dim = (0..state.doc.constraints.len())
            .find(|&i| crate::constraints::constraint_evaluated_length(&state.doc, i).is_some())
            .expect("rectangle should have a width dimension constraint");
        let (a, b) = crate::constraints::constraint_segment_endpoints(&state.doc, width_dim).unwrap();
        let world = crate::dimensions::linear_dimension_world_geom(
            a,
            b,
            glam::Vec3::Y,
            5.0,
            1.0,
            2.0,
        );
        let label_text = crate::constraints::constraint_evaluated_length(&state.doc, width_dim)
            .map(crate::value::format_length_display)
            .unwrap();
        let (text_vertices, text_indices) = crate::gpu_viewport::build_planar_label_mesh(
            &ctx,
            &world,
            &view,
            &label_text,
            Color32::WHITE,
            &project,
        );
        let dim_label = crate::gpu_viewport::ViewportDimLabel {
            world_geom: world,
            color: Color32::WHITE,
            text_vertices,
            text_indices,
            draw_dimension_lines: true,
        };
        let vertex_count_before = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: Some(session),
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
        .vertices
        .len();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: Some(session),
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: std::slice::from_ref(&dim_label),
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(!scene.text_vertices.is_empty());
        assert!(!scene.text_indices.is_empty());
        assert!(scene.vertices.len() > vertex_count_before);
    }

    #[test]
    fn dashed_world_segments_use_six_pixel_dashes_and_four_pixel_gaps() {
        let cam = Camera::default();
        let viewport = test_viewport();
        let vp = cam.view_proj(viewport);
        let a = Vec3::new(-80.0, 5.0, 0.0);
        let b = Vec3::new(80.0, 5.0, 0.0);
        let pa = cam.project(a, viewport, &vp).unwrap();
        let pb = cam.project(b, viewport, &vp).unwrap();
        let screen_len = (pb - pa).length();
        let segments = dashed_world_segments(
            a,
            b,
            CONSTRUCTION_DASH_LENGTH_PX,
            CONSTRUCTION_DASH_GAP_PX,
            &cam,
            viewport,
            &vp,
        );
        let expected = ((screen_len + CONSTRUCTION_DASH_GAP_PX) / 10.0).ceil() as usize;
        assert!(segments.len() >= expected.saturating_sub(1));
        assert!(segments.len() <= expected + 1);
        for (wa, wb) in &segments {
            let wa_s = cam.project(*wa, viewport, &vp).unwrap();
            let wb_s = cam.project(*wb, viewport, &vp).unwrap();
            let dash_px = (wb_s - wa_s).length();
            assert!(dash_px <= CONSTRUCTION_DASH_LENGTH_PX + 0.5);
            assert!(dash_px > 0.5);
        }
    }

    #[test]
    fn construction_line_produces_more_gpu_segments_than_solid_line() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let session = state.sketch_session.unwrap();
        let line = crate::model::Line::from_local_endpoints(
            session.sketch,
            0.0,
            0.0,
            80.0,
            0.0,
        );
        let mut construction = line.clone();
        construction.construction = true;
        let mut solid = line;
        solid.construction = false;
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let mut dashed_doc = state.doc.clone();
        dashed_doc.lines = vec![construction];
        let mut solid_doc = state.doc.clone();
        solid_doc.lines = vec![solid];
        let scene_fields = (
            &cam,
            viewport,
            ViewportPalette::default(),
            Some(session),
            &state.scene_selection,
            &state.element_visibility,
        );
        let grid_indices = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: scene_fields.0,
            viewport: scene_fields.1,
            palette: scene_fields.2,
            sketch_session: scene_fields.3,
            selection: scene_fields.4,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
        .indices
        .len();
        let dashed_scene = ViewportScene::build(&ViewportSceneInput {
            doc: &dashed_doc,
            cam: scene_fields.0,
            viewport: scene_fields.1,
            palette: scene_fields.2,
            sketch_session: scene_fields.3,
            selection: scene_fields.4,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let solid_scene = ViewportScene::build(&ViewportSceneInput {
            doc: &solid_doc,
            cam: scene_fields.0,
            viewport: scene_fields.1,
            palette: scene_fields.2,
            sketch_session: scene_fields.3,
            selection: scene_fields.4,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let dashed_line_indices = dashed_scene.indices.len().saturating_sub(grid_indices);
        let solid_line_indices = solid_scene.indices.len().saturating_sub(grid_indices);
        assert!(dashed_line_indices > solid_line_indices);
    }

    #[test]
    fn line_screen_quad_has_four_corners() {
        let cam = Camera::default();
        let viewport = test_viewport();
        let vp = cam.view_proj(viewport);
        let quad = line_screen_quad(
            Vec3::ZERO,
            Vec3::new(100.0, 0.0, 0.0),
            2.0,
            &cam,
            viewport,
            &vp,
        )
        .expect("visible segment");
        assert_ne!(quad[0], quad[1]);
        assert_ne!(quad[2], quad[3]);
    }
}
/// Manual perf probe for the selection aura (#145): `cargo test aura_perf_probe -- --ignored
/// --nocapture` prints scene-build times with and without a selected body. Ignored by
/// default (timing-based, machine-dependent) — for eyeballing regressions, not CI.
#[cfg(test)]
mod perf_probe {
    use super::*;
    use crate::actions::{Action, AppState, Tool};
    use crate::hierarchy::{ElementVisibility, SceneElement};
    use crate::selection::SceneSelection;

    #[test]
    #[ignore]
    fn aura_perf_probe() {
        // A round body (many triangles) plus a second body, big viewport.
        let mut state = AppState::default();
        state.apply(Action::BeginSketch { face: crate::model::FaceId::ConstructionPlane(0), viewport: None });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.circles.push(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: crate::model::ExtrudeFace::Circle(0) });
        state.apply(Action::SetExtrudeDistance { distance: 60.0 });
        state.apply(Action::CommitExtrusion);
        state.apply(Action::ExitSketch);

        let viewport = UiRect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1700.0, 1700.0));
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(0), false);
        let empty = SceneSelection::default();

        let build = |sel: &SceneSelection| {
            let input = ViewportSceneInput {
                doc: &state.doc,
                cam: &state.cam,
                viewport,
                sketch_session: None,
                element_visibility: &ElementVisibility::default(),
                selection: sel,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
                editing_extrusion: None,
                preview_cut_body: None,
                preview_cut_solids: Vec::new(),
                highlighted_bezier_handles: Vec::new(),
                plane_preview: None,
                active_sketch_face: None,
                palette: ViewportPalette::default(),
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                move_gizmos: Vec::new(),
                move_rotation_gizmo: None,
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &crate::document_health::DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            };
            ViewportScene::build(&input)
        };
        // Warm up, then time.
        for _ in 0..3 { build(&empty); build(&selection); }
        let n = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..n { build(&empty); }
        let base = t0.elapsed() / n;
        let t1 = std::time::Instant::now();
        for _ in 0..n { build(&selection); }
        let with_aura = t1.elapsed() / n;
        println!("scene build: base {base:?}  with aura {with_aura:?}  aura delta {:?}", with_aura.saturating_sub(base));
    }
}

#[cfg(test)]
mod cut_preview_tests {
    use super::*;
    use crate::actions::{Action, AppState, Tool};
    use crate::hierarchy::ElementVisibility;
    use crate::selection::SceneSelection;

    fn one_body_state() -> AppState {
        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: crate::model::ExtrudeFace::Circle(0),
        });
        state.apply(Action::SetExtrudeDistance { distance: 60.0 });
        state.apply(Action::CommitExtrusion);
        state.apply(Action::ExitSketch);
        state
    }

    fn scene_with_cut(state: &AppState, cut_highlight_bodies: Vec<usize>) -> ViewportScene {
        let viewport = UiRect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 800.0));
        let selection = SceneSelection::default();
        let input = ViewportSceneInput {
            doc: &state.doc,
            cam: &state.cam,
            viewport,
            sketch_session: None,
            element_visibility: &ElementVisibility::default(),
            selection: &selection,
            cut_highlight_bodies,
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_mirror_ghost: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            editing_extrusion: None,
            preview_cut_body: None,
            preview_cut_solids: Vec::new(),
            highlighted_bezier_handles: Vec::new(),
            plane_preview: None,
            active_sketch_face: None,
            palette: ViewportPalette::default(),
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            move_gizmos: Vec::new(),
            move_rotation_gizmo: None,
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &crate::document_health::DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        };
        ViewportScene::build(&input)
    }

    /// #264/#455: a body picked into the destructive (side-B / cut) picker recolors red
    /// (translucent fill) — the outline aura is gone, so the recolor alone must show.
    #[test]
    fn cut_body_recolors_red() {
        let state = one_body_state();
        let plain = scene_with_cut(&state, Vec::new());
        let cut = scene_with_cut(&state, vec![0]);
        let red_tinted = |scene: &ViewportScene| {
            scene
                .vertices
                .iter()
                .filter(|v| {
                    let [r, g, b, a] = v.color;
                    a > 0.0 && r > g * 1.2 && r > b * 1.2
                })
                .count()
        };
        assert!(
            red_tinted(&cut) > red_tinted(&plain),
            "cut body should recolor red: {} vs {}",
            red_tinted(&cut),
            red_tinted(&plain)
        );
    }
}
