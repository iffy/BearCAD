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
    GIZMO_HANDLE_HOVER_RGBA, PickTargetKind, PlaneEditDependentPreview,
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
pub const DEFAULT_CONSTRUCTION_PLANE_OPACITY: f32 = 0.18;
/// Lift plane fills toward the camera so they win over the ground grid. Kept at zero: any
/// bias here visibly detaches a plane from the body it passes through — a sphere created on
/// a plane read as poking through it (#1088). The grid wins its own overlap via the depth
/// bias on the ground pipeline instead.
pub const PLANE_FILL_DEPTH_BIAS: f32 = 0.0;
/// Base depth lift for sketch shape fills toward the camera.
/// Base fill color for extruded solid bodies (shaded per triangle).
pub const SOLID_FILL: Color32 = Color32::from_rgb(150, 168, 196);

/// The fill a body renders in: its material's colour (#834). A body with no material of
/// its own is made of the document's first material (#924, **Unobtainium** in a fresh
/// document), and falls back to [`SOLID_FILL`] only when there isn't one.
pub fn body_material_fill(doc: &crate::model::Document, body: &crate::model::Body) -> Color32 {
    body.material
        .and_then(|mi| doc.materials.get(mi))
        .or_else(|| doc.default_material().and_then(|mi| doc.materials.get(mi)))
        .map(|m| Color32::from_rgb(m.color[0], m.color[1], m.color[2]))
        .unwrap_or(SOLID_FILL)
}
/// Base fill for an imported unit's materialized body (#724): a warmer tone than
/// [`SOLID_FILL`], so pointing at read-only unit geometry is visibly different from
/// pointing at the document's own bodies.
pub const UNIT_SOLID_FILL: Color32 = Color32::from_rgb(178, 162, 144);
/// The wash an element's **outputs** wear while its Elements-pane row is hovered (#977): what
/// this step made, what this component holds, what this joint joins. Deliberately not the plain
/// hover colour — that says "this is the thing under your cursor", and the thing under your
/// cursor is the row, not the body. A history operation isn't in the 3D view at all, so its
/// outputs are the only thing it *can* light.
pub const DERIVED_OUTPUT_HIGHLIGHT: Color32 = Color32::from_rgb(170, 130, 240);

/// Green glow for the dimensions/geometry driven by the parameter hovered or focused in
/// the Parameters pane (#620).
pub const PARAMETER_HIGHLIGHT: Color32 = Color32::from_rgb(90, 220, 130);
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
/// Fill for the **mobile** side of a joint being made or edited (#992) — green. A joint's two
/// sides do different things, and while the tool is up telling them apart is the whole question;
/// lighting both in the selection blue answered it with "these two", which is what you already
/// knew. Only while the tool is previewing: a committed joint's parts are ordinary bodies again.
pub const SOLID_FILL_JOINT_MOBILE: Color32 = Color32::from_rgb(104, 200, 128);
/// Fill for the **fixed** (held) side of a joint being made or edited (#992) — blue.
pub const SOLID_FILL_JOINT_FIXED: Color32 = Color32::from_rgb(96, 150, 226);
/// Highlighted fill for the in-progress extrusion preview.
pub const SOLID_PREVIEW_FILL: Color32 = Color32::from_rgb(120, 215, 230);
/// Opacity of the in-progress extrusion preview body (before it is committed).
pub const SOLID_PREVIEW_OPACITY: f32 = 0.4;
/// The Move tool's Face Snap rotation gizmo (#1426): yellow, matching the A→A connector
/// so the whole Face Snap preview reads as one motion.
pub const MOVE_ROTATION_GIZMO: Color32 = crate::theme::MOVE_CONNECTOR;
/// The arc between the rotation gizmo's start line and its current handle (#1361): yellow, the
/// "how far you have turned" reading that everything else in the Move tool shares.
pub const MOVE_ROTATION_ARC: Color32 = Color32::from_rgb(255, 225, 90);
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
///
/// Solid ground (#159/#1295/#1301) does **not** use this bias: a world-space lift mis-places the
/// plane (#1088/#1121). Solid ground is a dedicated no-depth-write shader pass (like the grid)
/// so coplanar construction planes composite without z-fighting; body faces on z = 0 still
/// re-draw after plane fills (`body_over_plane`, #1215).
pub const GRID_DEPTH_BIAS: f32 = -0.05;
/// Solid ground fill (#159/#1295): dark grey-blue, readable against the near-black viewport
/// background and distinct from pure black. Not derived from the grid grey (that scaled too
/// dark and read as black).
pub const SOLID_GROUND_COLOR: Color32 = Color32::from_rgb(42, 50, 64);
/// Contact shadows on the build plane (#1041): dark and mostly transparent, so the grid and
/// the ground's own colour still read through rather than being blacked out.
pub const GROUND_SHADOW_FILL: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 90);
/// Lift a contact shadow off the plane it lies on, so it doesn't z-fight the ground fill.
pub const GROUND_SHADOW_DEPTH_BIAS: f32 = 0.04;
/// On-screen width of the origin X/Y/Z axes, in pixels (#1072).
pub const ORIGIN_AXIS_WIDTH_PX: f32 = 2.0;
/// Hover thickness for a world origin axis (#1124) — same as [`push_segment_hover`].
pub const ORIGIN_AXIS_HOVER_WIDTH_PX: f32 = 4.0;
/// Selected thickness: **2×** the hover thickness, and drawn through bodies (#1124).
pub const ORIGIN_AXIS_SELECTED_WIDTH_PX: f32 = ORIGIN_AXIS_HOVER_WIDTH_PX * 2.0;
/// Lift strokes toward the camera so lines draw over coplanar face fills and grid.
pub const STROKE_DEPTH_BIAS: f32 = 0.10;
/// Lift construction-plane hover fills above the plane surface (avoids z-fighting).
/// Kept at zero: the overlay pipeline's own depth bias is enough to prevent z-fighting with
/// the plane fill, and any world-space lift would make the hover highlight show through
/// bodies that sit on the plane (#1090). Body-coplanar face fills do **not** use this path —
/// they go on the depth-disabled wireframe layer instead (#1139).
const HOVER_PLANE_DEPTH_LIFT: f32 = 0.0;
/// Lift sketch-face hover/active fills toward the camera so they sit above committed coplanar
/// fills (which are themselves biased) and just under strokes — otherwise a hover over
/// overlapping faces renders behind/at-equal-depth with those fills and shows patchy
/// artifacts along the overlaps (#19). For faces coplanar with a solid body the fill is also
/// depth-test-disabled (#1139); the lift still helps when a face is only coplanar with other
/// sketch fills (e.g. on a construction plane).
const HOVER_FILL_DEPTH_BIAS: f32 = 0.09;
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
const GIZMO_HANDLE_RING_STROKE_PX: f32 = 1.5;
/// Fading arcs either side of a rotation gizmo's handle (#1405): each extends this many
/// degrees out from the handle around the circle of rotation, fading out toward its tip.
const GIZMO_ROTATION_FADE_ARC_DEG: f32 = 30.0;
/// How many screen segments sample each fading rotation arc (#1405).
const GIZMO_ROTATION_FADE_ARC_SEGMENTS: usize = 20;
/// How far off each side of the rotation handle its direction arrows sit (#1405/#1421), in px.
/// Same stand-off as the Move translation arrows: gap from the disc plus the arrowhead.
const GIZMO_ROTATION_ARROW_OFFSET_PX: f32 = GIZMO_ARROW_GAP_PX + GIZMO_ARROW_HEAD_PX;
/// Radial line from the body centre to a rotation handle (#1419): thinner than the fade arcs.
const GIZMO_ROTATION_RADIAL_STROKE_PX: f32 = 1.0;

/// Original-position radial is dashed; the handle's current radial is solid (#1419).
#[cfg(test)]
fn rotation_radial_is_dashed(original: bool) -> bool {
    original
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    /// World-space normal in `xyz` and the lighting model in `w` (#1037) — see
    /// [`ShadingModel`]. Everything that isn't a solid body (lines, fills, text, gizmos,
    /// the grid) is `ShadingModel::Unlit`, and the shader hands its colour straight
    /// through, exactly as before per-pixel lighting existed.
    pub normal: [f32; 4],
}

/// Which lighting the fragment shader applies to a vertex (#1037), passed in
/// [`GpuVertex::normal`]`.w`. Kept in step with the `MODE_*` constants in `shader.wgsl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadingModel {
    /// Colour passes through untouched — 2D chrome, lines, fills, text, gizmos.
    Unlit = 0,
    /// Two-sided ambient + diffuse, the `Solid` mode look.
    Lambert = 1,
    /// Ambient + diffuse + Blinn-Phong specular, the `Realistic` mode look (#83).
    Realistic = 2,
}

impl ShadingModel {
    fn as_w(self) -> f32 {
        self as u8 as f32
    }
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
    /// Contact shadows on the build plane (#1041): each body's triangles projected onto
    /// z = 0 along the scene's fixed light. Drawn through a stencil so the silhouette's own
    /// overlaps paint once — a self-overlapping shadow blended twice reads as blotches.
    pub shadow_indices: Vec<u32>,
    /// Construction-plane fills and translucent solids — shadow bodies, previews, ghosts,
    /// faded bodies — drawn after the opaque scene, so they tint what they cover. A datum
    /// plane is a reference that shows through the bodies it bisects (#1087).
    pub plane_fill_indices: Vec<u32>,
    /// Opaque body triangles that lie on a construction plane (or an extrusion's target
    /// plane), re-drawn **after** plane fills (#1215). Coplanar solid/plane pairs z-fight
    /// under floating-point depth; a second pass of just those faces restores the solid
    /// colour without world-space, pipeline, or frag-depth bias (those mis-place planes,
    /// #1088/#1121). Shares vertices with the base solid draw.
    pub body_over_plane_indices: Vec<u32>,
    /// Strokes, selection, hover, and previews (drawn on top of plane fills).
    pub overlay_indices: Vec<u32>,
    /// Manipulation gizmos (plane/extrude offset+angle handles). Drawn last with the
    /// depth test disabled so handles stay visible even when behind a body (#36).
    pub gizmo_indices: Vec<u32>,
    /// Body edge-wireframe overlay (#33). Drawn depth-test-disabled, same as gizmos, so
    /// edges stay visible "through" a solid body in solid+wireframe shading mode.
    pub wireframe_indices: Vec<u32>,
    /// The selection/hover outline mask (#1110/#1155): triangles of selected and hovered
    /// bodies, drawn flat into an offscreen mask (R = selected, G = hovered). A later
    /// fullscreen pass dilates the mask and strokes the silhouette band (blue selected,
    /// yellow hovered) **on top of** the fill recolour — both effects always apply.
    pub mask_indices: Vec<u32>,
    pub text_vertices: Vec<GpuTextVertex>,
    pub text_indices: Vec<u32>,
    /// Tracing images (#170): textured world-space quads, drawn after the opaque scene
    /// (depth-tested, no depth write) so bodies in front occlude them.
    pub images: Vec<ViewportImageQuad>,
    /// The world origin axes (#1072): screen-space-widened quads. `position` is the corner's
    /// own world endpoint, `normal.xyz` the segment's other endpoint, and `normal.w` the
    /// signed half-width in **pixels** — the vertex shader projects both ends and steps
    /// sideways by that many pixels, so an axis is the same width however far away and
    /// however steeply it recedes. Drawn with `vs_axis`/`fs_axis` (round caps, #1202).
    pub axis_vertices: Vec<GpuVertex>,
    pub axis_indices: Vec<u32>,
    /// Depth-tested sketch / overlay strokes widened in **screen space** (#1157), same packing
    /// as [`Self::axis_vertices`]. A camera-facing world ribbon of constant thickness reads as
    /// a freestanding 3D rectangle when a body face is viewed at a grazing angle; these keep
    /// depth on the face endpoints and let `vs_axis` step sideways in pixels. `fs_axis` clips
    /// each fragment to a round-capped capsule so coincident joints meet cleanly (#1202).
    /// Drawn after the opaque base so bodies occlude them correctly.
    pub stroke_vertices: Vec<GpuVertex>,
    pub stroke_indices: Vec<u32>,
    /// The ground grid (#1073), when one is showing: a single footprint quad whose fragment
    /// shader draws the lattice. Thick world-space line quads could not stay thin — one
    /// viewed edge-on foreshortens into a wedge and one viewed close up swells — so the
    /// lines are measured in **pixels**, per fragment, from the screen-space derivative of
    /// the world position. The grid draws on either side of the ground (#1370); only the
    /// solid fill is hidden when the camera is under the ground (#1300).
    pub grid: Option<ViewportGrid>,
    /// Solid ground fill (#159/#1295/#1301): one footprint quad drawn by a dedicated shader
    /// pass — depth-tested, no depth write — so coplanar construction planes and body
    /// bottoms never z-fight it. Hidden when the camera is under the ground (#1300).
    pub solid_ground: Option<ViewportSolidGround>,
    pub view_proj: Mat4,
    /// Camera eye in world space — the fragment shader's view-dependent lighting terms
    /// need it, and it can't be recovered from `view_proj` cheaply enough per pixel (#1037).
    pub eye: Vec3,
    pub clear_color: [f32; 4],
}

/// Solid ground footprint (#159/#1295/#1301). Drawn like the grid: early, depth-tested, no
/// depth write — painter's order under bodies and translucent plane fills, so coplanar
/// geometry never z-fights without geometric or pipeline bias.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSolidGround {
    pub corners: [Vec3; 4],
    /// Premultiplied RGBA for the fill (`SOLID_GROUND_COLOR`, possibly sketch-dimmed).
    pub color: [f32; 4],
}

/// The shader-drawn ground grid (#1073). One quad covering the visible ground footprint,
/// plus the lattice parameters its fragment shader needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportGrid {
    /// The footprint's four world corners, already nudged away from the camera by
    /// [`GRID_DEPTH_BIAS`] so real coincident geometry wins the depth test.
    pub corners: [Vec3; 4],
    /// World spacing of the subdividing lines and of the heavy ones (#464).
    pub fine_step: f32,
    pub coarse_step: f32,
    /// How far the fine level has faded in with zoom, 0..1 — a continuous ramp, so a
    /// subdivision never pops into existence (#464).
    pub fine_fade: f32,
    /// Horizontal distance from the eye's ground projection (world mm) where the lattice
    /// starts softening (#1123). Past [`fade_end_mm`] it is fully transparent.
    pub fade_start_mm: f32,
    /// Horizontal distance where the lattice has fully faded (#1123).
    pub fade_end_mm: f32,
    /// Line widths in **pixels**, which is the whole point: constant on screen at any
    /// distance and any grazing angle.
    pub fine_width_px: f32,
    pub coarse_width_px: f32,
    pub axis_width_px: f32,
    pub fine_color: [f32; 4],
    pub coarse_color: [f32; 4],
    /// The x = 0 and y = 0 lines, drawn heavier than their coarse neighbours.
    pub axis_color: [f32; 4],
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

/// A tracing image's four world corners, in UV order: (0,0), (1,0), (1,1), (0,1) — v flipped,
/// since image v grows downward and plane-local v grows up. One definition (#977), shared by the
/// textured quad and by the hover outline its Elements-pane row draws.
fn tracing_image_corners(
    doc: &Document,
    image: crate::model::TracingImageKey,
) -> Option<[Vec3; 4]> {
    let img = doc.tracing_images.get(image)?;
    let frame = crate::face::sketch_frame(doc, FaceId::ConstructionPlane(img.plane))?;
    let at = |x: f32, y: f32| frame.origin + frame.u_axis * x + frame.v_axis * y;
    let (x0, y0) = img.origin;
    let (x1, y1) = (x0 + img.width_mm, y0 + img.height_mm);
    Some([at(x0, y1), at(x1, y1), at(x1, y0), at(x0, y0)])
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
    /// Solid sketch strokes on a **body face** outside sketch mode (#1149/#1153/#1167): dark
    /// blue-grey so they contrast on light body materials. Plane sketches keep [`Self::rect_line`].
    pub rect_line_on_body: Color32,
    /// Body-face strokes while a sketch is open (#1167): light blue-grey. Sketch mode dims
    /// bodies, so the dark on-body stroke vanishes; this stays readable. Also used outside
    /// sketch when the face material is dark (adaptive contrast).
    pub rect_line_on_body_in_sketch: Color32,
    /// Fully-constrained solid lines (#172): no remaining degrees of freedom.
    pub rect_line_constrained: Color32,
    pub preview: Color32,
    pub construction: Color32,
    /// Associative projections (#140/#1186): solid cyan (construction-like, but not dashed).
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
            rect_line_on_body: Color32::from_rgb(50, 60, 78),
            rect_line_on_body_in_sketch: Color32::from_rgb(170, 190, 225),
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
    /// A closed world-space region rendered as a filled/outlined polygon (#16/#62/#202): used
    /// for a computed boolean-combined region (which, unlike `SketchFace`, has no `FaceId` of
    /// its own — it's just `ExtrudeFace::Boolean`'s on-demand geometry) and for a Loft tool
    /// cross-section profile under the cursor. `holes` are interior loops the fill leaves
    /// empty (#942), so hovering a wall/ring shows the wall, not the shape around it.
    ClosedLoop {
        world_loop: Vec<Vec3>,
        holes: Vec<Vec<Vec3>>,
    },
    /// A whole analytic edge given as its segments (#807): a hole's rim is many chords in the
    /// mesh but one edge to the tools, so it highlights as one.
    Curve { segments: Vec<(Vec3, Vec3)> },
}

/// Whether curved line `li`'s tangent handles should be drawn (#550): only when the curve or
/// one of its endpoints is selected or hovered, or one of its handles is being manipulated.
/// Keeps handles from cluttering and obscuring every curve in the sketch at rest.
pub(crate) fn bezier_handles_relevant(
    li: crate::model::LineKey,
    selection: &SceneSelection,
    hover: &Option<ViewportHoverHighlight>,
    highlighted_handles: &[(crate::model::LineKey, bool)],
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
    /// Single-handle dial (#1360/#1361): the unit radial marking 0°, and the current signed
    /// turn from it (degrees). When both are present the gizmo draws a line centre→start, a
    /// yellow arc up to the handle, a line centre→handle and a disc at the handle. `None` for
    /// the plain ring gizmos (Free Move's three world rings, a selected text's turn ring).
    pub zero_dir: Option<Vec3>,
    pub angle_deg: Option<f32>,
    /// True while this ring's handle is being dragged (#1420): draw the full thin
    /// circle of rotation, then drop it on release.
    pub dragging: bool,
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

/// Arc length drawn for the revolve gizmo. Multi-turn angles only need the last fractional
/// turn of geometry (or one full turn when the angle is an integer multiple of 360°) —
/// further turns re-trace the same circle, and with a fixed segment count that produced a
/// star of long chords (#1247). The push/pull handle is the arc's far end, so this keeps it
/// at the true planar end angle (angle mod 360°).
fn revolve_arc_display_angle_deg(angle_deg: f32) -> f32 {
    if angle_deg.abs() < 360.0 {
        return angle_deg;
    }
    let sign = if angle_deg < 0.0 { -1.0 } else { 1.0 };
    let rem = angle_deg.abs() % 360.0;
    if rem < 1e-3 {
        sign * 360.0
    } else {
        sign * rem
    }
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
    let segments = segments.max(1);
    let total = angle_deg.to_radians();
    (0..=segments)
        .map(|i| {
            let t = total * i as f32 / segments as f32;
            let rot = glam::Quat::from_axis_angle(n, t);
            center + (rot * zero_dir) * radius
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

/// A tool's live result preview standing in for committed geometry: the bodies it hides
/// and the translucent solids it draws instead. The two lists are independent — a Combine
/// turns any number of picked bodies into any number of result solids.
#[derive(Clone, Debug, Default)]
pub struct PreviewReplacement {
    /// Bodies hidden while the preview is up, because it shows what becomes of them.
    pub bodies: Vec<crate::model::BodyKey>,
    /// The solids drawn in their place, in the shared translucent preview style.
    pub solids: Vec<crate::extrude::SolidMesh>,
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
    pub cut_highlight_bodies: Vec<crate::model::BodyKey>,
    /// Bodies to render dimmed/translucent because they are descendants of the operation being
    /// edited (#260), so the edit's downstream effects are visually de-emphasized.
    pub faded_bodies: Vec<crate::model::BodyKey>,
    /// Dashed ghost world-segments previewing an in-progress in-sketch repeat's duplicates
    /// (#232) — the sketch-plane equivalent of the 3D repeat ghost meshes.
    pub sketch_repeat_ghost: Vec<(Vec3, Vec3)>,
    /// Preview segments for in-progress in-sketch geometry — a mirror's reflection (#542), an
    /// offset's parallel copies (#940). Each `(a, b, dashed)` draws a solid preview-coloured
    /// line (matching the repeat/extrude/revolve preview styling), or a dashed one when the
    /// result will be construction geometry.
    pub sketch_ghost_lines: Vec<(Vec3, Vec3, bool)>,
    /// Live-updated meshes for the faded descendant bodies (#260): while a gizmo edit is being
    /// dragged, `faded_bodies[bi]` renders this preview mesh (recomputed from a scratch document
    /// with the edit applied) instead of its stale committed geometry, so downstream bodies
    /// follow the drag. Keyed by body index; a faded body absent here just fades in place.
    pub edit_preview_meshes: std::collections::HashMap<crate::model::BodyKey, crate::extrude::SolidMesh>,
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
    /// Laser-cut surface previews for the Slice tool (#1142/#1144): ruled strips through the
    /// body, drawn in cut-red (not the cyan solid-preview fill used by `repeat_ghosts`).
    pub cut_surface_ghosts: Vec<crate::extrude::SolidMesh>,
    /// Index of the extrusion currently being edited, if any. Its committed body
    /// is suppressed so only the ghost preview is shown while editing.
    pub editing_extrusion: Option<crate::model::ExtrusionKey>,
    /// Target body when the in-progress extrusion is a **cut** (#142): its committed solid is
    /// suppressed and replaced by a translucent preview of the *cut result* (that body with
    /// `preview_extrusion` subtracted) rather than the additive block, so the preview looks
    /// like the finished cut. `None` for add/new-body extrudes (block preview) or when the
    /// kernel can't build the cut result.
    pub preview_cut_body: Option<crate::model::BodyKey>,
    /// Bodies a tool's live result preview stands in for, and the solids drawn in their
    /// place — what `preview_cut_body` does for one in-progress extrusion, generalized to
    /// the tools whose result isn't one-mesh-per-body: a Sweep cut (one solid per carved
    /// body) and a Combine (#1033, where N picked inputs become M result solids).
    pub preview_replacement: PreviewReplacement,
    /// Bezier tangent handles to draw in the gold pick-highlight color (#472): the
    /// hovered, dragged, and/or selected `(line, near_start)` handles.
    pub highlighted_bezier_handles: Vec<(crate::model::LineKey, bool)>,
    pub plane_preview: Option<ViewportPlanePreview>,
    pub active_sketch_face: Option<FaceId>,
    pub dimension_labels: &'a [ViewportDimLabel],
    pub dim_label_view: Option<PlanarLabelView>,
    pub plane_gizmo: Option<ViewportPlaneGizmo>,
    pub extrude_gizmo: Option<ViewportExtrudeGizmo>,
    /// Push/pull gizmo for the in-progress chamfer/fillet tool; reuses the same offset-gizmo
    /// mesh as [`ViewportSceneInput::extrude_gizmo`] (#37/#38).
    pub vertex_treatment_gizmo: Option<ViewportExtrudeGizmo>,
    /// Extra push/pull arrow handles, each the same offset-arrow mesh as the extrude gizmo:
    /// Free Move's six face-centred translation handles (#215/#1233), and the in-sketch
    /// Offset tool's distance handle (#939).
    pub arrow_gizmos: Vec<ViewportExtrudeGizmo>,
    /// Rotation-ring gizmos: Free Move's three world-axis rings (#1234), Face Snap's spin
    /// ring (#1077), and a selected sketch text's turn ring (#216/#286).
    pub move_rotation_gizmos: Vec<MoveRotationGizmo>,
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
    /// Scene elements to highlight in a colour of their own (#961): what a **destructive**
    /// picker holds — a Slice cutter, say — which reads red rather than the blue selection
    /// style. Bodies go through `cut_highlight_bodies` instead, since a solid takes a fill
    /// rather than an outline.
    pub colored_element_highlights: Vec<(SceneElement, Color32)>,
    /// Bodies whose **fill** a tool is overriding while it previews (#992): the Joint tool's
    /// two sides, green for the one that moves and blue for the one holding it. A fill, not an
    /// aura, because for a solid the fill *is* the visual — and it outranks the selection blue,
    /// which would otherwise paint both sides the same and answer the wrong question.
    pub tinted_bodies: Vec<(crate::model::BodyKey, Color32)>,
    /// Pick targets to highlight in a colour of their own rather than the shared hover colour
    /// (#660): the Move tool marks start point A green and end point A red.
    pub colored_pick_highlights: Vec<(crate::construction::PickTargetKind, Color32)>,
    /// World-space segments to draw in a colour of their own (#668): the Move tool's connector
    /// from start point A to end point A, so the translation reads as a vector.
    pub colored_segments: Vec<(Vec3, Vec3, Color32, bool)>,
    /// Elements using the parameter hovered/focused in the Parameters pane (#620) — its
    /// dimensions, driven geometry, and expression consumers — each drawn in the green
    /// [`PARAMETER_HIGHLIGHT`] color.
    pub parameter_highlight_elements: Vec<SceneElement>,
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
            eye: input.cam.eye(),
            clear_color: color32_to_gpu(input.palette.background),
            ..Default::default()
        };
        // Tracing images (#170): a textured quad per visible image on its host plane.
        for (ii, img) in input.doc.tracing_images.iter() {
            if !input
                .element_visibility
                .effective_visible(input.doc, SceneElement::Image(ii))
            {
                continue;
            }
            let Some((id, w, h, rgba)) = decoded_tracing_image(img) else {
                continue;
            };
            let Some(corners) = tracing_image_corners(input.doc, ii) else {
                continue;
            };
            scene.images.push(ViewportImageQuad {
                id,
                corners,
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

        for (ci, circle) in input.doc.circles.iter() {
            if !circle_alive(input.doc, ci)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Circle(ci))
                // Scaffolding for its own sketch, and noise everywhere else (#994).
                || (circle.construction
                    && !construction_geometry_visible(input.sketch_session, circle.sketch))
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
                shape_fill_depth_bias_laned(ci.index() as usize, 1),
            );
            mesh.set_index_layer(MeshIndexLayer::Base);
        }

        // Closed loops of plain lines (#66) — fill them the same way a rect/circle face is.
        for sketch in input.doc.sketches.keys().collect::<Vec<_>>() {
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
                    shape_fill_depth_bias_laned(lines[0].index() as usize, 2),
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
        let shadow_shown = |bi: crate::model::BodyKey| -> bool {
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
                // Slice targets occupy the same outer envelope as the fragment bodies
                // (#1150): ghosting them on Slice-row hover coplanar-z-fights the pieces.
                // Hovering/selecting the shadow body itself still shows it (above).
                Some(ViewportHoverHighlight::Element(SceneElement::SliceOp(_))) => false,
                _ => false,
            }
        };
        // Bodies an Elements-pane op/component/joint row "lights" while hovered (#977):
        // recolour them in the main pass like body hover (#455/#1150). Stacking a translucent
        // coplanar copy of the same mesh z-fights the solid into a mottled checkerboard.
        let derived_output_bodies: std::collections::HashSet<crate::model::BodyKey> =
            match &input.hover_highlight {
                Some(ViewportHoverHighlight::Element(
                    el @ (SceneElement::BooleanOp(_)
                    | SceneElement::MoveOp(_)
                    | SceneElement::MirrorOp(_)
                    | SceneElement::RepeatOp(_)
                    | SceneElement::SketchRepeatOp(_)
                    | SceneElement::SketchOffsetOp(_)
                    | SceneElement::SketchMirrorOp(_)
                    | SceneElement::SketchVertexTreatmentOp(_)
                    | SceneElement::SketchSliceOp(_)
                    | SceneElement::SliceOp(_)
                    | SceneElement::ShellOp(_)
                    | SceneElement::EdgeTreatmentOp(_)
                    | SceneElement::Revolution(_)
                    | SceneElement::Shape(_)
                    | SceneElement::SweepOp(_)
                    | SceneElement::Joint(_)
                    | SceneElement::Component(_)),
                )) => crate::hierarchy::produced_bodies(input.doc, el)
                    .into_iter()
                    .collect(),
                _ => Default::default(),
            };
        let body_meshes: std::collections::HashMap<crate::model::BodyKey, Option<crate::extrude::SolidMesh>> = input
            .doc
            .bodies
            .iter()
            .map(|(bi, body)| {
                let mut visible = input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Body(bi))
                    && (!body.shadow || shadow_shown(bi));
                // A unit's materialized body follows its instance row's eye toggle (#724).
                if let crate::model::BodySource::UnitInstance(instance) = body.source {
                    visible = visible
                        && input
                            .element_visibility
                            .effective_visible(input.doc, SceneElement::UnitInstance(instance));
                }
                let mesh = if visible {
                    crate::extrude::body_solid_mesh(input.doc, bi)
                } else {
                    None
                };
                (bi, mesh)
            })
            .collect();
        // Smooth per-vertex normals for the shaded modes (#1037), shared out of the same
        // kind of per-document cache the meshes use, so this is a refcount bump per frame
        // rather than a rebuild.
        let shaded = matches!(
            input.cam.shading_mode(),
            crate::camera::ShadingMode::Solid
                | crate::camera::ShadingMode::SolidWireframe
                | crate::camera::ShadingMode::Realistic
        );
        let body_normals: std::collections::HashMap<crate::model::BodyKey, Option<std::rc::Rc<Vec<[Vec3; 3]>>>> =
            body_meshes
                .iter()
                .map(|(bi, mesh)| {
                    let normals = (shaded && mesh.is_some())
                        .then(|| crate::extrude::body_smooth_normals(input.doc, *bi))
                        .flatten();
                    (*bi, normals)
                })
                .collect();

        // Extruded solid bodies (3D, depth-tested, flat-shaded).
        for (bi, body) in input.doc.bodies.iter() {
            if let Some(editing) = input.editing_extrusion {
                if body.source.owns_extrusion(editing)
                    || crate::extrude::body_is_edge_treated_from_extrusion(
                        input.doc, bi, editing,
                    )
                {
                    continue;
                }
            }
            // The cut target is drawn as the translucent cut-result preview instead of its
            // intact committed solid.
            if preview_cut.as_ref().is_some_and(|(cut_bi, _)| *cut_bi == bi)
                || input.preview_replacement.bodies.contains(&bi)
            {
                continue;
            }
            let Some(solid) = body_meshes.get(&bi).and_then(|m| m.as_ref()) else {
                continue;
            };
            let normals = body_normals
                .get(&bi)
                .and_then(|n| n.as_deref())
                .map(|n| n.as_slice());
            // Shadow bodies render as a translucent ghost with a wireframe, whatever the
            // shading mode — visually distinct from every real body.
            if body.shadow {
                mesh.push_solid_translucent(solid, SHADOW_BODY_FILL, SHADOW_BODY_OPACITY);
                let edges = crate::extrude::body_feature_edges(input.doc, bi);
                mesh.push_solid_wireframe(
                    solid,
                    Some(edges.as_ref()),
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
            // A unit's materialized body (#724) reads as the instance row: selection and
            // hover key off the instance, and its base fill is its own hue so it stays
            // visibly "not yours to edit" next to the document's own bodies.
            let unit_instance = match body.source {
                crate::model::BodySource::UnitInstance(i) => Some(i),
                _ => None,
            };
            // A whole body's hover reaches here two ways: as an Elements-pane row
            // (`Element`), or as the pick target a Selection Exploder loupe stands for
            // (#985) — a hovered body loupe (or a group loupe holding the body) recolors
            // the body itself, since `push_hover_highlight` has no marker to draw for a
            // whole solid.
            let hovered = match &input.hover_highlight {
                Some(ViewportHoverHighlight::Element(SceneElement::Body(h))) => *h == bi,
                Some(ViewportHoverHighlight::PickTarget(PickTargetKind::Body(h))) => *h == bi,
                Some(ViewportHoverHighlight::Element(SceneElement::UnitInstance(h))) => {
                    unit_instance == Some(*h)
                }
                _ => false,
            } || input
                .extra_pick_highlights
                .iter()
                .any(|k| matches!(k, PickTargetKind::Body(h) if *h == bi));
            let selected = input.selection.is_selected(SceneElement::Body(bi))
                || unit_instance
                    .is_some_and(|i| input.selection.is_selected(SceneElement::UnitInstance(i)));
            // #1110/#1155: selected/hovered bodies always get **both** solid-body shading
            // (fill + wire recolour) and a screen-space silhouette outline from the mask
            // pass. (`tint` still wins for fill: it's an explicit override, e.g. the
            // joint-mobile green, not a highlight.)
            let tint = input
                .tinted_bodies
                .iter()
                .find(|(t, _)| *t == bi)
                .map(|(_, c)| *c);
            let derived = derived_output_bodies.contains(&bi);
            let fill = if let Some(tint) = tint {
                tint
            } else if selected {
                SOLID_FILL_SELECTED
            } else if derived {
                // #977/#1150: operation-row wash — main-pass recolour, not a coplanar overlay.
                DERIVED_OUTPUT_HIGHLIGHT
            } else if hovered {
                SOLID_FILL_HOVERED
            } else if unit_instance.is_some() {
                UNIT_SOLID_FILL
            } else {
                // The body's material colours it (#834); bodies with no material keep the
                // default look.
                body_material_fill(input.doc, body)
            };
            let line_color = if selected {
                BODY_SILHOUETTE_COLOR
            } else if derived {
                DERIVED_OUTPUT_HIGHLIGHT
            } else if hovered {
                SOLID_FILL_HOVERED
            } else {
                WIREFRAME_LINE_COLOR
            };
            // Outline mask: R for selected, G for hovered (selected wins when both). The
            // mask pass draws them unlit into an offscreen texture; a later fullscreen pass
            // dilates that into the silhouette band over the already-recoloured fill.
            if selected || hovered {
                let mask_color = if selected {
                    Color32::from_rgb(255, 0, 0)
                } else {
                    Color32::from_rgb(0, 255, 0)
                };
                let restore = mesh.index_layer;
                mesh.set_index_layer(MeshIndexLayer::Mask);
                for tri in &solid.triangles {
                    mesh.push_triangle(tri[0], tri[1], tri[2], mask_color);
                }
                mesh.set_index_layer(restore);
            }
            // Sketch mode dims every body (#433): the bright face shading otherwise
            // fights the sketch lines and dimension labels drawn over it.
            let fill = if input.sketch_session.is_some() {
                scale_color(fill, SKETCH_MODE_BODY_DIM)
            } else {
                fill
            };
            // Frames that this body's faces may sit on: every visible construction plane,
            // plus an extrusion's target plane when the top cap lands on one (#29/#1215),
            // plus the build plane itself when solid ground is showing (#1295).
            // Triangles on these frames are re-drawn after plane fills (and after the solid
            // ground fill in the base pass) so coplanar solid/plane pairs don't z-fight —
            // without geometric or frag-depth bias.
            let mut coplanar_planes: Vec<(Vec3, Vec3)> = input
                .doc
                .construction_planes
                .iter()
                .filter(|(pi, _)| {
                    input.element_visibility.effective_visible(
                        input.doc,
                        SceneElement::ConstructionPlane(*pi),
                    )
                })
                .map(|(_, p)| (p.origin, p.normal))
                .collect();
            if let Some(cap) = body
                .source
                .extrusion_indices()
                .first()
                .and_then(|&ei| input.doc.extrusions.get(ei))
                .and_then(|ext| crate::extrude::target_top_plane(input.doc, ext))
            {
                if !coplanar_planes
                    .iter()
                    .any(|&(o, n)| (o - cap.0).length() < 1e-4 && n.dot(cap.1).abs() > 0.999)
                {
                    coplanar_planes.push(cap);
                }
            }
            // Solid ground (#1295): same coplanar-overpaint path as construction planes
            // (#1215), so body bottoms on z = 0 win without a world-space ground bias.
            if input.cam.ground_display() == crate::camera::GroundDisplay::Solid {
                let ground = (Vec3::ZERO, Vec3::Z);
                if !coplanar_planes
                    .iter()
                    .any(|&(o, n)| o.length() < 1e-4 && n.dot(Vec3::Z).abs() > 0.999)
                {
                    coplanar_planes.push(ground);
                }
            }
            // Shading mode (#33) picks how the committed body renders: `Solid` (today's
            // existing look) is opaque fill only; `Wireframe` is edges only, no fill;
            // `TransparentSolid` is translucent fill, no edges; `SolidWireframe` is opaque
            // fill plus an edge overlay that stays visible "through" the body (mirrors how
            // gizmos draw through bodies — depth-test disabled, see `MeshIndexLayer::Wireframe`).
            match input.cam.shading_mode() {
                crate::camera::ShadingMode::Solid => {
                    mesh.push_solid(solid, normals, fill, input.cam, &coplanar_planes);
                }
                crate::camera::ShadingMode::TransparentSolid => {
                    mesh.push_solid_translucent(solid, fill, TRANSPARENT_SOLID_OPACITY);
                }
                crate::camera::ShadingMode::Wireframe => {
                    // Feature edges from the memoized body analysis (#845/#1141) — a body with
                    // circular holes has hundreds of rim segments; recomputing them every frame
                    // from the triangle soup was pure waste while the camera moved.
                    let edges = crate::extrude::body_feature_edges(input.doc, bi);
                    mesh.push_solid_wireframe(
                        solid,
                        Some(edges.as_ref()),
                        line_color,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::SolidWireframe => {
                    mesh.push_solid(solid, normals, fill, input.cam, &coplanar_planes);
                    let edges = crate::extrude::body_feature_edges(input.doc, bi);
                    mesh.push_solid_wireframe(
                        solid,
                        Some(edges.as_ref()),
                        line_color,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::Realistic => {
                    mesh.push_solid_realistic(solid, normals, fill, input.cam, &coplanar_planes);
                    // A contact shadow on the build plane (#1041), Realistic only. Without
                    // one a part resting on the ground and a part hovering 50 mm above it
                    // look identical — there is no cue for where geometry sits at all.
                    mesh.push_ground_shadow(solid, input.cam);
                }
            }
        }
        // Imported unit instances (#722/#724) render through the ordinary body loop above:
        // each instance materializes as a derived body (`BodySource::UnitInstance`), so
        // shading modes, selection, hover, and picking all treat it as a body — keyed to
        // the instance row and tinted UNIT_SOLID_FILL to read as read-only.

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
        for solid in &input.preview_replacement.solids {
            mesh.push_solid_translucent(solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        }
        // Repeat-tool instance ghosts (#223): each would-be copy, translucent like other previews.
        for ghost in &input.repeat_ghosts {
            mesh.push_solid_translucent(ghost, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        }
        // Slice laser cut surfaces (#1144): cut-red so they read as cutters, not as the body.
        for ghost in &input.cut_surface_ghosts {
            mesh.push_solid_translucent(ghost, SOLID_FILL_CUT, SOLID_PREVIEW_OPACITY);
        }
        // Ghost feature edges draw on top of everything (#743): a Move preview that lands
        // flush against — or embedded in — stationary geometry is otherwise swallowed by
        // the depth test, and the visible remainder reads as landing in the wrong place.
        mesh.set_index_layer(MeshIndexLayer::Wireframe);
        for ghost in &input.repeat_ghosts {
            for chain in solid_mesh_edge_chains(ghost) {
                for (a, b) in chain {
                    mesh.push_line_segment(
                        a,
                        b,
                        SOLID_PREVIEW_FILL,
                        2.0,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
            }
        }
        for ghost in &input.cut_surface_ghosts {
            for chain in solid_mesh_edge_chains(ghost) {
                for (a, b) in chain {
                    mesh.push_line_segment(
                        a,
                        b,
                        SOLID_FILL_CUT,
                        2.0,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
            }
        }
        mesh.set_index_layer(MeshIndexLayer::Base);

        let mut plane_draws: Vec<(crate::model::ConstructionPlaneKey, ConstructionPlane, Color32, f32)> = Vec::new();
        for (i, plane) in input.doc.construction_planes.iter() {
            if !input
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
        for (li, line) in input.doc.lines.iter() {
            if !line_alive(input.doc, li)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Line(li))
                || input.selection.is_selected(SceneElement::Line(li))
                // Scaffolding for its own sketch, and noise everywhere else (#994).
                || (line.construction
                    && !construction_geometry_visible(input.sketch_session, line.sketch))
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
                // #1149/#1153/#1167: body-face strokes contrast the face (and brighten in sketch).
                solid_sketch_stroke_color(
                    &input.palette,
                    input.doc,
                    line.sketch,
                    dim,
                    input.sketch_session,
                )
            };
            let color = health_tint_color(base, input.document_health.element_status(element));
            if let Some(points) = line_world_polyline(input.doc, line) {
                // Same solid 2px width on body faces and planes (#1153): the thinner dark stroke
                // from #1149 read as wispy/non-solid under AA.
                const STROKE_WIDTH: f32 = 2.0;
                // While the host sketch is open, every line of that sketch shows through
                // bodies (#1200) — solid, construction, and projected cyan (#1192/#1186)
                // alike — so a body between the camera and the sketch plane cannot hide the
                // profile. Closed sketches keep depth-tested strokes (#1157).
                let show_through = input
                    .sketch_session
                    .is_some_and(|s| s.sketch == line.sketch);
                if show_through {
                    mesh.set_index_layer(MeshIndexLayer::Wireframe);
                }
                if line.construction && line.projection.is_none() {
                    mesh.push_dashed_polyline_segment(
                        &points,
                        color,
                        STROKE_WIDTH,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                } else {
                    mesh.push_polyline_segment(
                        &points,
                        color,
                        STROKE_WIDTH,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                if show_through {
                    mesh.set_index_layer(MeshIndexLayer::Base);
                }
            }
        }

        // Sketch text (#282): draw each baked glyph contour as a closed polyline, transformed by
        // the text's origin/rotation onto its sketch plane. Selected text uses the selection color.
        for (ti, text) in input.doc.sketch_texts.iter() {
            if !input
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
                solid_sketch_stroke_color(
                    &input.palette,
                    input.doc,
                    text.sketch,
                    dim,
                    input.sketch_session,
                )
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
            for (li, line) in input.doc.lines.iter() {
                if line.sketch != session.sketch {
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
        for (ci, circle) in input.doc.circles.iter() {
            if !circle_alive(input.doc, ci)
                || !input
                    .element_visibility
                    .effective_visible(input.doc, SceneElement::Circle(ci))
                // Scaffolding for its own sketch, and noise everywhere else (#994).
                || (circle.construction
                    && !construction_geometry_visible(input.sketch_session, circle.sketch))
            {
                continue;
            }
            let dim = input.sketch_session.is_some_and(|s| {
                !sketch_circle_is_active(input.doc, s, ci, circle.sketch)
            });
            let element = SceneElement::Circle(ci);
            // Committed circle strokes depth-test like body-face lines (#1157 / #1174)
            // when their sketch is closed: screen-space width + STROKE_DEPTH_BIAS so they
            // sit on the face, and the solid occludes them when the host face is behind
            // the body. While the host sketch is open they show through bodies like
            // lines (#1200). Selection/hover still use Wireframe (see push_selection /
            // face hover).
            let show_through = input
                .sketch_session
                .is_some_and(|s| s.sketch == circle.sketch);
            if show_through {
                mesh.set_index_layer(MeshIndexLayer::Wireframe);
            }
            mesh.push_circle_strokes(
                input.doc,
                circle,
                ci,
                input.cam,
                input.viewport,
                &vp,
                health_tint_color(
                    solid_sketch_stroke_color(
                        &input.palette,
                        input.doc,
                        circle.sketch,
                        dim,
                        input.sketch_session,
                    ),
                    input.document_health.element_status(element.clone()),
                ),
                health_tint_color(
                    sketch_color(input.palette.construction, dim),
                    input.document_health.element_status(element),
                ),
            );
            if show_through {
                mesh.set_index_layer(MeshIndexLayer::Overlay);
            }
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

                // The sketch's own axes (#577): the floating origin's two reference axes — X (u) and
                // Y (v) — are drawn through the origin so the sketch frame is always visible and its
                // orientation unambiguous (the camera no longer forces u-right/v-up). They're
                // selectable in the constraint tool: a line constrained parallel to an axis replaces
                // the old Horizontal/Vertical constraints. Faint in their axis colours normally,
                // brighter when hovered, and the selection colour when selected. The half-length
                // tracks the visible viewport (scaled by zoom) so an axis always spans the view.
                use crate::model::{ConstraintLine, SketchAxis};
                let aspect =
                    (input.viewport.width() / input.viewport.height().max(1.0)).max(0.01);
                let (half_w, half_h) = input.cam.viewport_half_extents(aspect);
                let axis_hl = half_w.hypot(half_h) * 2.5;
                for (axis, dir, base) in [
                    (SketchAxis::X, frame.u_axis, input.palette.x_axis),
                    (SketchAxis::Y, frame.v_axis, input.palette.y_axis),
                ] {
                    let el = SceneElement::FaceEdge(ConstraintLine::OriginAxis(axis));
                    let selected = input.selection.is_selected(el.clone());
                    let hovered = matches!(
                        &input.hover_highlight,
                        Some(ViewportHoverHighlight::Element(e)) if *e == el
                    );
                    let (color, width) = if selected {
                        (input.palette.dim_edge_highlight, 3.0)
                    } else if hovered {
                        (input.hover_color, 2.5)
                    } else {
                        // Faint so the axes read as reference lines, not sketch geometry.
                        (base.gamma_multiply(0.55), 1.5)
                    };
                    mesh.push_line_segment(
                        frame.origin - dir * axis_hl,
                        frame.origin + dir * axis_hl,
                        color,
                        width,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
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
            // A preview circle isn't in the document yet, so it has no key of its own; the
            // callee only uses this for a depth lane (#1055).
            mesh.push_circle(
                input.doc,
                circle,
                crate::arena::Key::from_bits(u64::MAX),
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
        // Coloured segments (#668): the Move tool's start-A → end-A connector, plus the
        // dashed end-B guides (#745).
        for &(a, b, color, dashed) in &input.colored_segments {
            if dashed {
                mesh.push_dashed_line_segment(a, b, color, 2.0, input.cam, input.viewport, &vp);
            } else {
                mesh.push_line_segment(a, b, color, 2.0, input.cam, input.viewport, &vp);
            }
        }
        // In-sketch mirror (#542) and offset (#940) previews: solid preview-coloured lines like
        // the repeat/extrude/revolve previews, dashed only when the result is construction.
        for &(a, b, dashed) in &input.sketch_ghost_lines {
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
        for gizmo in &input.arrow_gizmos {
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
        for ring in &input.move_rotation_gizmos {
            let project = |w: Vec3| input.cam.project(w, input.viewport, &vp);
            push_rotation_gizmo(
                &mut mesh,
                ring,
                input.cam,
                input.viewport,
                &vp,
                &project,
            );
        }
        if let Some(arc) = input.revolve_arc_gizmo.as_ref() {
            // The swept arc from 0° to the current angle, plus a push/pull disc handle at its
            // far end (#262). Multi-turn angles draw one full turn so the arc never becomes a
            // star of long chords (#1247); the handle still uses the true end angle.
            let points = revolve_arc_points(
                arc.center,
                arc.axis,
                arc.zero_dir,
                arc.radius,
                revolve_arc_display_angle_deg(arc.angle_deg),
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
        // Individually coloured marks, e.g. the Move tool's green source / red target (#660).
        for (kind, color) in &input.colored_pick_highlights {
            mesh.push_hover_highlight(
                input.doc,
                &ViewportHoverHighlight::PickTarget(kind.clone()),
                *color,
                &body_meshes,
                input.cam,
                input.viewport,
                &vp,
            );
        }
        // What a destructive picker holds, in its own colour (#961) — the elements a Slice
        // cutter set holds are faces and planes, which have no body fill to recolour.
        for (element, color) in &input.colored_element_highlights {
            mesh.push_element_hover(
                input.doc,
                element.clone(),
                *color,
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
        // Everything using the hovered/focused parameter glows green (#620).
        for element in &input.parameter_highlight_elements {
            match element {
                // The sub-body recolor tint washes out against a solid fill; outline the
                // driven extrusion's own mesh in green so it always reads.
                SceneElement::Extrusion(ei) => {
                    if let Some(m) = input
                        .doc
                        .extrusions
                        .get(*ei)
                        .and_then(|e| crate::extrude::extrusion_mesh(input.doc, e))
                    {
                        mesh.set_index_layer(MeshIndexLayer::Wireframe);
                        mesh.push_solid_wireframe(
                            &m,
                            None,
                            PARAMETER_HIGHLIGHT,
                            input.cam,
                            input.viewport,
                            &vp,
                        );
                        mesh.set_index_layer(MeshIndexLayer::Overlay);
                    }
                }
                _ => mesh.push_hover_highlight(
                    input.doc,
                    &ViewportHoverHighlight::Element(element.clone()),
                    PARAMETER_HIGHLIGHT,
                    &body_meshes,
                    input.cam,
                    input.viewport,
                    &vp,
                ),
            }
        }

        if input.dim_label_view.is_some() {
            // While a sketch is open, dimension lines/arrows show through bodies
            // (#1280) — same depth-disabled wireframe path as open-sketch lines
            // (#1200). Committed dims only draw in sketch mode, so this always
            // applies when labels are present.
            let show_through = input.sketch_session.is_some();
            let restore = mesh.index_layer;
            if show_through {
                mesh.set_index_layer(MeshIndexLayer::Wireframe);
            }
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
            if show_through {
                mesh.set_index_layer(restore);
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
    /// Contact shadows on the build plane (#1041).
    GroundShadow,

    PlaneFill,
    /// Solid faces coplanar with a construction/target plane, drawn after plane fills
    /// so they win coplanar depth ties without bias (#1215).
    BodyOverPlane,
    Overlay,
    /// Manipulation gizmos, drawn last with the depth test disabled (#36).
    Gizmo,
    /// Body edge-wireframe overlay, drawn depth-test-disabled like [`Self::Gizmo`] (#33).
    Wireframe,
    /// Selection/hover outline mask (#1110): selected/hovered bodies' triangles, drawn
    /// flat (unlit) into the offscreen mask texture, never the main color target.
    Mask,
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
            MeshIndexLayer::GroundShadow => &mut self.scene.shadow_indices,

            MeshIndexLayer::PlaneFill => &mut self.scene.plane_fill_indices,
            MeshIndexLayer::BodyOverPlane => &mut self.scene.body_over_plane_indices,
            MeshIndexLayer::Overlay => &mut self.scene.overlay_indices,
            MeshIndexLayer::Gizmo => &mut self.scene.gizmo_indices,
            MeshIndexLayer::Wireframe => &mut self.scene.wireframe_indices,
            MeshIndexLayer::Mask => &mut self.scene.mask_indices,
        }
    }

    fn push_vertex(&mut self, position: Vec3, color: Color32) {
        self.push_lit_vertex(position, color, Vec3::ZERO, ShadingModel::Unlit);
    }

    /// A vertex the fragment shader lights itself (#1037): `normal` is world-space, and
    /// `model` picks which lighting it gets.
    fn push_lit_vertex(
        &mut self,
        position: Vec3,
        color: Color32,
        normal: Vec3,
        model: ShadingModel,
    ) {
        self.scene.vertices.push(GpuVertex {
            position: position.to_array(),
            color: color32_to_gpu(color),
            normal: [normal.x, normal.y, normal.z, model.as_w()],
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

    /// A triangle the shader lights per pixel, with its own normal per corner (#1037).
    fn push_lit_triangle(
        &mut self,
        verts: [Vec3; 3],
        normals: [Vec3; 3],
        color: Color32,
        model: ShadingModel,
    ) {
        let base = self.scene.vertices.len() as u32;
        for (v, n) in verts.iter().zip(normals.iter()) {
            self.push_lit_vertex(*v, color, *n, model);
        }
        self.indices_mut()
            .extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn push_quad_fill(&mut self, fill_corners: [Vec3; 4], fill: Color32) {
        self.push_triangle(fill_corners[0], fill_corners[1], fill_corners[2], fill);
        self.push_triangle(fill_corners[0], fill_corners[2], fill_corners[3], fill);
    }

    /// Push a solid mesh with flat (per-triangle) two-sided shading.
    ///
    /// `coplanar_planes` are construction-plane (and optional extrusion-target) frames:
    /// triangles that lie on any of them are also indexed into
    /// [`ViewportScene::body_over_plane_indices`] so the renderer can re-draw them after
    /// translucent plane fills and break coplanar z-fighting without depth bias (#1215/#29).
    fn push_solid(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        normals: Option<&[[Vec3; 3]]>,
        base: Color32,
        cam: &Camera,
        coplanar_planes: &[(Vec3, Vec3)],
    ) {
        self.push_shaded_solid(solid, normals, base, cam, coplanar_planes, ShadingModel::Lambert);
    }

    /// A body's contact shadow on the build plane (#1041): its triangles projected onto
    /// z = 0 along the scene's fixed light direction, drawn dark and translucent.
    ///
    /// The receiver is one known plane, so this needs no shadow map and no second depth pass
    /// — the projection is a line-plane intersection per vertex. Triangles that dip below the
    /// plane are dropped rather than projected backwards through the light, so a half-buried
    /// part shadows only the half that is above it.
    ///
    /// A silhouette overlaps itself, and overlapping translucent triangles blend twice into
    /// blotches; the stencil pass this layer draws through paints each pixel once, exactly as
    /// coplanar sketch fills do (#3).
    fn push_ground_shadow(&mut self, solid: &crate::extrude::SolidMesh, cam: &Camera) {
        let light = SCENE_LIGHT_DIR.normalize_or_zero();
        // A light parallel to the plane casts no shadow onto it.
        if light.z.abs() < 1e-3 {
            return;
        }
        let prev = self.index_layer;
        self.set_index_layer(MeshIndexLayer::GroundShadow);
        let eye = cam.eye();
        for tri in &solid.triangles {
            if tri.iter().any(|p| p.z < 0.0) {
                continue;
            }
            let flat: [Vec3; 3] = std::array::from_fn(|i| {
                let p = tri[i];
                p - light * (p.z / light.z)
            });
            // Lifted off the plane the same way every other decal is, or it z-fights the
            // ground fill it sits on.
            let lifted = offset_corners_toward_camera(
                [flat[0], flat[1], flat[2], flat[0]],
                Vec3::Z,
                eye,
                GROUND_SHADOW_DEPTH_BIAS,
            );
            self.push_triangle(lifted[0], lifted[1], lifted[2], GROUND_SHADOW_FILL);
        }
        self.set_index_layer(prev);
    }

    /// Push a solid mesh lit as `ShadingMode::Realistic` (#83): ambient + diffuse plus a
    /// camera-dependent Blinn-Phong specular. No materials/textures yet — every body uses
    /// the same fixed gloss.
    fn push_solid_realistic(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        normals: Option<&[[Vec3; 3]]>,
        base: Color32,
        cam: &Camera,
        coplanar_planes: &[(Vec3, Vec3)],
    ) {
        self.push_shaded_solid(
            solid,
            normals,
            base,
            cam,
            coplanar_planes,
            ShadingModel::Realistic,
        );
    }

    /// Push a solid mesh for the fragment shader to light per pixel (#1037).
    ///
    /// `normals` are the body's smooth per-vertex normals; without them each corner falls
    /// back to its triangle's own geometric normal, which is the pre-#1037 faceted look.
    /// Triangles that lie on any frame in `coplanar_planes` are also recorded in
    /// [`ViewportScene::body_over_plane_indices`] so a later pass can repaint them over
    /// translucent plane fills without geometric or frag-depth bias (#1215/#29).
    fn push_shaded_solid(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        normals: Option<&[[Vec3; 3]]>,
        base: Color32,
        _cam: &Camera,
        coplanar_planes: &[(Vec3, Vec3)],
        model: ShadingModel,
    ) {
        // Stale normals would shade the wrong geometry, so a length mismatch drops back to
        // flat rather than indexing into whatever is there.
        let normals = normals.filter(|n| n.len() == solid.triangles.len());
        for (ti, tri) in solid.triangles.iter().enumerate() {
            let flat = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            let corner_normals = normals.map(|n| n[ti]).unwrap_or([flat; 3]);
            let base_idx = self.scene.vertices.len() as u32;
            self.push_lit_triangle(*tri, corner_normals, base, model);
            // Re-index (same vertices) for the post-plane solid pass when this face sits on
            // a datum/target plane — no position nudge (#1215).
            if coplanar_planes
                .iter()
                .any(|&(origin, normal)| triangle_on_plane(tri, origin, normal))
            {
                let restore = self.index_layer;
                self.set_index_layer(MeshIndexLayer::BodyOverPlane);
                self.indices_mut()
                    .extend_from_slice(&[base_idx, base_idx + 1, base_idx + 2]);
                self.set_index_layer(restore);
            }
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
    ///
    /// `feature_edges`, when provided, is the precomputed crease/boundary set (from
    /// [`crate::extrude::body_feature_edges`]); otherwise edges are derived from `solid`.
    fn push_solid_wireframe(
        &mut self,
        solid: &crate::extrude::SolidMesh,
        feature_edges: Option<&[(Vec3, Vec3)]>,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let prev = self.index_layer;
        self.set_index_layer(MeshIndexLayer::Wireframe);
        let owned;
        let edges: &[(Vec3, Vec3)] = match feature_edges {
            Some(e) => e,
            None => {
                owned = solid_mesh_unique_edges(solid);
                owned.as_slice()
            }
        };
        for &(a, b) in edges {
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
        // Depth-tested layers: widen in screen space (#1157 / #1072). Wireframe/gizmo use
        // depth-disabled world ribbons so they still show through bodies (#33/#36).
        match self.index_layer {
            MeshIndexLayer::Gizmo | MeshIndexLayer::Wireframe => {
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
                        normal: [0.0, 0.0, 0.0, ShadingModel::Unlit.as_w()],
                    });
                }
                self.indices_mut()
                    .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            MeshIndexLayer::Base
            | MeshIndexLayer::Overlay
            | MeshIndexLayer::SketchFill
            | MeshIndexLayer::PlaneFill
            | MeshIndexLayer::BodyOverPlane
            | MeshIndexLayer::GroundShadow
            | MeshIndexLayer::Mask => {
                let _ = (viewport, view_proj);
                self.push_screen_width_stroke(a, b, color, width_px, cam.eye(), depth_bias);
            }
        }
    }

    /// Screen-space-widened stroke into [`ViewportScene::stroke_vertices`] (#1157).
    fn push_screen_width_stroke(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        width_px: f32,
        eye: Vec3,
        depth_bias: f32,
    ) {
        let (a, b) = offset_segment_toward_camera(a, b, eye, depth_bias);
        if (b - a).length_squared() < 1e-12 {
            return;
        }
        let gpu = color32_to_gpu(color);
        let half = width_px * 0.5;
        let base = self.scene.stroke_vertices.len() as u32;
        // Same corner packing as `push_screen_width_segment` / origin axes (#1072).
        for (own, other, side) in [(a, b, half), (a, b, -half), (b, a, half), (b, a, -half)] {
            self.scene.stroke_vertices.push(GpuVertex {
                position: own.to_array(),
                color: gpu,
                normal: [other.x, other.y, other.z, side],
            });
        }
        self.scene
            .stroke_indices
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
        // Reach past the distance-fade end so the hard footprint edge sits outside the
        // soft ramp (#1123) — orbiting no longer snaps lattice sections on and off.
        let cam_dist = (eye - cam.target).length().max(1.0);
        let fade_start_mm = (cam_dist * 2.5).max(GRID_EXTENT * 0.5);
        let fade_end_mm = (cam_dist * 7.0).max(GRID_EXTENT * 2.0);
        let reach = fade_end_mm.max(GRID_EXTENT);
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
        // Always cover at least a disc of radius `reach` around the target so horizon
        // views (few ground hits) still have a full soft-faded lattice, not a thin wedge.
        lo = lo.min(anchor - glam::Vec2::splat(reach));
        hi = hi.max(anchor + glam::Vec2::splat(reach));
        lo -= glam::Vec2::splat(coarse_step);
        hi += glam::Vec2::splat(coarse_step);
        // `None` hides the ground entirely (#579).
        //
        // Solid ground (#159/#1295/#1301): one filled plane in a dark grey-blue at exact z = 0
        // on a dedicated no-depth-write shader pass (same pattern as the grid). Putting it in
        // the opaque base mesh wrote depth and z-fought coplanar construction planes / body
        // bottoms; a world-space bias mis-places coplanar geometry (#1088/#1121). Body faces
        // resting on the ground still re-draw after plane fills via `body_over_plane` (#1215).
        //
        // The *solid* ground is only drawn when the camera is above z = 0 (#1300): looking up
        // from underneath must not paint a floor through the scene. The *grid*, by contrast,
        // is independent of the camera side (#1370) — a subdivision lattice reads the same
        // viewed from under the plane, and its axes still orient the view, so only the solid
        // fill is suppressed from underneath.
        let above_ground = eye.z > 0.0;
        if cam.ground_display() == crate::camera::GroundDisplay::None {
            // nothing
        } else if cam.ground_display() == crate::camera::GroundDisplay::Solid && above_ground {
            let fill = sketch_ground_color(SOLID_GROUND_COLOR, dim);
            let corners = [
                Vec3::new(lo.x, lo.y, 0.0),
                Vec3::new(hi.x, lo.y, 0.0),
                Vec3::new(hi.x, hi.y, 0.0),
                Vec3::new(lo.x, hi.y, 0.0),
            ];
            self.scene.solid_ground = Some(ViewportSolidGround {
                corners,
                color: color32_to_gpu(fill),
            });
        } else if cam.ground_display() == crate::camera::GroundDisplay::Grid {
            // One footprint quad; the lattice is computed per fragment (#1073). The old
            // per-line world-space quads could not hold a constant on-screen width — at a
            // grazing angle each quad foreshortened into a wedge, and up close it swelled —
            // so the widths below are pixels, measured against the screen-space derivative
            // of world position in the shader.
            let corners = [
                Vec3::new(lo.x, lo.y, 0.0),
                Vec3::new(hi.x, lo.y, 0.0),
                Vec3::new(hi.x, hi.y, 0.0),
                Vec3::new(lo.x, hi.y, 0.0),
            ];
            // Fine subdivisions fade in with zoom (#464), a continuous ramp so nothing pops.
            let fade = ((fine_step * ppw - FINE_MIN_PX) / (FINE_FULL_PX - FINE_MIN_PX))
                .clamp(0.0, 1.0);
            self.scene.grid = Some(ViewportGrid {
                corners: offset_corners_toward_camera(corners, Vec3::Z, eye, GRID_DEPTH_BIAS),
                fine_step,
                coarse_step,
                fine_fade: fade,
                fade_start_mm,
                fade_end_mm,
                fine_width_px: 1.0,
                coarse_width_px: 1.0,
                axis_width_px: 1.0,
                fine_color: color32_to_gpu(mix_color(
                    palette.background,
                    sketch_ground_color(palette.grid, dim),
                    0.7,
                )),
                coarse_color: color32_to_gpu(sketch_ground_color(palette.grid, dim)),
                axis_color: color32_to_gpu(sketch_ground_color(palette.grid_axis, dim)),
            });
        }
        // The origin triad, widened on screen rather than in the world (#1072): a
        // fixed-world-width quad is only ever the right thickness at one depth, so under
        // perspective the near end of an axis swelled while the far end thinned away.
        for (end, color) in [
            (Vec3::new(axis_len, 0.0, 0.0), palette.x_axis),
            (Vec3::new(0.0, axis_len, 0.0), palette.y_axis),
            (Vec3::new(0.0, 0.0, axis_len), palette.z_axis),
        ] {
            self.push_screen_width_segment(
                Vec3::ZERO,
                end,
                sketch_ground_color(color, dim),
                ORIGIN_AXIS_WIDTH_PX,
                cam.eye(),
                GRID_DEPTH_BIAS,
            );
        }
    }

    /// A line whose width is measured in **pixels by the vertex shader** (#1072), not in
    /// world units here. Each corner carries its own endpoint, the segment's other endpoint,
    /// and a signed half-width; `vs_axis` projects both and steps sideways on screen.
    fn push_screen_width_segment(
        &mut self,
        a: Vec3,
        b: Vec3,
        color: Color32,
        width_px: f32,
        eye: Vec3,
        depth_bias: f32,
    ) {
        let (a, b) = offset_segment_toward_camera(a, b, eye, depth_bias);
        if (b - a).length_squared() < 1e-12 {
            return;
        }
        let gpu = color32_to_gpu(color);
        let half = width_px * 0.5;
        let base = self.scene.axis_vertices.len() as u32;
        // Corner order matches the quad the old world-space path emitted: a+, a-, b-, b+.
        for (own, other, side) in [(a, b, half), (a, b, -half), (b, a, half), (b, a, -half)] {
            self.scene.axis_vertices.push(GpuVertex {
                position: own.to_array(),
                color: gpu,
                // `vs_axis` takes its screen direction as (other - own), which points the
                // opposite way at the far end — so the same signed half-width lands the far
                // corners on the opposite sides, giving the cyclic order a+, a-, b-, b+ the
                // indices below assume.
                normal: [other.x, other.y, other.z, side],
            });
        }
        self.scene
            .axis_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
        _index: crate::model::CircleKey,
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
        _index: crate::model::CircleKey,
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
        index: crate::model::CircleKey,
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
        let corners = plane_corners(plane);
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
        index: crate::model::ConstructionPlaneKey,
        color: Color32,
        opacity: f32,
        cam: &Camera,
    ) {
        let corners = plane_corners(plane);
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
        _body_meshes: &std::collections::HashMap<crate::model::BodyKey, Option<crate::extrude::SolidMesh>>,
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
                            // Depth-disabled (#1409): a selected sketch line on a body face
                            // must never be occluded by extrusion geometry. Always use
                            // Wireframe like selected circles already do.
                            let restore = self.index_layer;
                            self.set_index_layer(MeshIndexLayer::Wireframe);
                            if dashed {
                                self.push_dashed_polyline_segment(
                                    &points, color, 3.0, cam, viewport, view_proj,
                                );
                            } else {
                                self.push_polyline_segment(&points, color, 3.0, cam, viewport, view_proj);
                            }
                            self.set_index_layer(restore);
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
                            // Depth-disabled (#1140): a selected circle on a body face is
                            // coplanar with the solid — same mottled z-fight as body-face
                            // selection before #555. Always use Wireframe (even on a
                            // construction plane the ring is thin and Always is fine).
                            let restore = self.index_layer;
                            self.set_index_layer(MeshIndexLayer::Wireframe);
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
                            self.set_index_layer(restore);
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
                        let restore = self.index_layer;
                        self.set_index_layer(MeshIndexLayer::Wireframe);
                        self.push_point_marker(world, color, 6.0, cam, viewport, view_proj);
                        self.set_index_layer(restore);
                    }
                }
                // Selected 3D body sub-elements (#156): drawn depth-test-disabled like their
                // hover highlights (#153), so a concave joint can't bury the selection mark.
                // A selected edge draws its whole tangent-continuous curve (#626).
                SceneElement::BodyEdge { body, a, b } => {
                    let wa = crate::hierarchy::dequantize_body_point(a);
                    let wb = crate::hierarchy::dequantize_body_point(b);
                    let chain = crate::extrude::body_solid_mesh(doc, body)
                        .map(|s| body_edge_curve_chain(&s, wa, wb))
                        .unwrap_or_else(|| vec![(wa, wb)]);
                    let restore = self.index_layer;
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                    for (sa, sb) in chain {
                        self.push_line_segment(sa, sb, color, 4.0, cam, viewport, view_proj);
                    }
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
                // A selected cylinder or centre line (#1013): the same marks the hover draws.
                SceneElement::BodyCylinder { .. } | SceneElement::BodyAxis { .. } => {
                    self.push_element_hover(
                        doc,
                        element.clone(),
                        color,
                        &std::collections::HashMap::new(),
                        cam,
                        viewport,
                        view_proj,
                    );
                }
                // A selected body face (#555/#557): re-find the coplanar-triangle group whose
                // quantized centroid+normal matches the stored key, then fill + stroke it in the
                // selection color, depth-test-disabled like the edge/vertex marks above.
                SceneElement::BodyFace { body, centroid, normal } => {
                    if let Some(tris) =
                        crate::extrude::body_face_triangles(doc, body, centroid, normal)
                    {
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
                // Selected world origin axis (#1124): 2× hover thickness, depth-test
                // disabled so it bleeds through every body (same always-on-top path as
                // selected body edges). Normal axes stay behind bodies; selected ones don't.
                SceneElement::GlobalAxis(axis) => {
                    let (a, b) = global_axis_segment(axis);
                    let restore = self.index_layer;
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                    self.push_line_segment(
                        a,
                        b,
                        color,
                        ORIGIN_AXIS_SELECTED_WIDTH_PX,
                        cam,
                        viewport,
                        view_proj,
                    );
                    self.set_index_layer(restore);
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
        extrusion: crate::model::ExtrusionKey,
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let Some(ext) = doc.extrusions.get(extrusion) else {
            return;
        };
        let Some(mesh) = crate::extrude::extrusion_mesh(doc, ext) else {
            return;
        };
        if matches!(cam.shading_mode(), crate::camera::ShadingMode::Wireframe) {
            self.push_solid_wireframe(&mesh, None, color, cam, viewport, view_proj);
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
        index: crate::model::ConstructionPlaneKey,
        color: Color32,
        fill_multiplier: f32,
        cam: &Camera,
    ) {
        let corners = plane_corners(plane);
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
        // Datum planes stay on the caller's depth-tested layer (#1090): a large GPU/world
        // lift would make the hover show through bodies sitting on the plane. Every other
        // face is coplanar with a solid (or its own sketch fill on one), and the overlay
        // bias alone is not enough — the body and the gold hover fill z-fight into a
        // mottled checkerboard (#1139). Body-face selection already uses the depth-disabled
        // wireframe layer; body-coplanar face hover fills do the same.
        let body_coplanar = !matches!(face, FaceId::ConstructionPlane(_));
        let restore = self.index_layer;
        if body_coplanar {
            self.set_index_layer(MeshIndexLayer::Wireframe);
        }
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
            FaceId::BodyMeshFace {
                body,
                centroid,
                normal,
            } => {
                // Fill from the mesh triangles themselves (#1219/#1220): a fan of the
                // outline loop mis-fills concave cut faces, and the old visit-order loop
                // painted diagonals. The border is drawn separately from the true outline.
                if let Some(tris) =
                    crate::extrude::body_face_triangles(doc, body, centroid, normal)
                {
                    let n = (tris[0][1] - tris[0][0])
                        .cross(tris[0][2] - tris[0][0])
                        .normalize_or_zero();
                    let lift =
                        |p: Vec3| offset_toward_camera(p, n, eye, HOVER_FILL_DEPTH_BIAS);
                    for tri in &tris {
                        self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                    }
                }
            }
            FaceId::RevolveCap { .. }
            | FaceId::RevolveSide { .. }
            | FaceId::UnitFace { .. }
            | FaceId::PrimitiveFace { .. }
            | FaceId::RepeatedFace { .. } => {
                let poly = match face {
                    FaceId::RevolveCap {
                        revolution,
                        ref profile,
                        end,
                    } => crate::extrude::revolve_cap_polygon_world(doc, revolution, profile, end)
                        .map(|(poly, _)| poly),
                    FaceId::RevolveSide {
                        revolution,
                        ref profile,
                        edge,
                    } => crate::extrude::revolve_side_geom(doc, revolution, profile, edge as usize)
                        .map(|(poly, _, _)| poly),
                    // A unit's flat face (#725): its placed boundary polygon.
                    FaceId::UnitFace { instance, ref face } => {
                        crate::units::unit_face_world_polygon(doc, instance, face)
                    }
                    FaceId::PrimitiveFace { primitive, face } => {
                        doc.primitives
                            .get(primitive)
                            .and_then(|shape| crate::primitives::face_polygon(doc, shape, face))
                    }
                    FaceId::RepeatedFace { .. } => {
                        crate::extrude::face_boundary_loop_world(doc, &face)
                    }
                    _ => None,
                };
                if let Some(poly) = poly {
                    if poly.len() >= 3 {
                        // Newell normal: consecutive arc points are nearly collinear, so a
                        // three-point cross product would be degenerate here.
                        let mut area = Vec3::ZERO;
                        for i in 0..poly.len() {
                            area += poly[i].cross(poly[(i + 1) % poly.len()]);
                        }
                        let normal = area.normalize_or_zero();
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
            // A datum plane is a face like any other here (#974). This arm used to be empty,
            // with the plane's fill written out at each *call site* instead — so whichever
            // caller hadn't been given the special case drew nothing, and a plane silently
            // failed to highlight while every other face worked.
            FaceId::ConstructionPlane(index) => {
                if let Some(plane) = doc.construction_planes.get(index) {
                    self.push_construction_plane_hover_fill(
                        plane,
                        index,
                        color,
                        fill_multiplier,
                        cam,
                    );
                }
            }
        }
        if body_coplanar {
            self.set_index_layer(restore);
        }
    }

    fn push_hover_highlight(
        &mut self,
        doc: &Document,
        hover: &ViewportHoverHighlight,
        color: Color32,
        body_meshes: &std::collections::HashMap<crate::model::BodyKey, Option<crate::extrude::SolidMesh>>,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let project = |w: Vec3| cam.project(w, viewport, view_proj);
        match hover {
            ViewportHoverHighlight::SketchFace(face) => {
                // Fill + border both depth-disabled for body-coplanar faces (#1139); the
                // fill helper switches the layer itself for non-plane faces, and the border
                // must follow or the outline would z-fight the solid independently.
                let body_coplanar = !matches!(face, FaceId::ConstructionPlane(_));
                let restore = self.index_layer;
                if body_coplanar {
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                }
                self.push_sketch_face_hover(
                    doc,
                    face.clone(),
                    color,
                    FACE_HOVER_FILL_MULTIPLIER,
                    cam,
                );
                self.push_sketch_face_hover_border(
                    doc,
                    face.clone(),
                    color,
                    2.0,
                    cam,
                    viewport,
                    view_proj,
                );
                if body_coplanar {
                    self.set_index_layer(restore);
                }
            }
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
            ViewportHoverHighlight::Curve { segments } => {
                // Depth-test-disabled like every other pick highlight (#153): a rim sunk
                // into a hole would otherwise be half-buried in the wall beside it.
                let restore_layer = self.index_layer;
                self.set_index_layer(MeshIndexLayer::Wireframe);
                for (a, b) in segments {
                    self.push_segment_hover(*a, *b, color, cam, viewport, view_proj, &project);
                }
                self.set_index_layer(restore_layer);
            }
            ViewportHoverHighlight::ClosedLoop { world_loop, holes } => {
                if world_loop.len() >= 3 {
                    let eye = cam.eye();
                    let normal = (world_loop[1] - world_loop[0])
                        .cross(world_loop[2] - world_loop[0])
                        .normalize_or_zero();
                    let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                    let lift = |p: Vec3| offset_toward_camera(p, normal, eye, HOVER_FILL_DEPTH_BIAS);
                    // Depth-disabled like body-coplanar face fills (#1139): a region on a
                    // solid's face is coplanar with the body and would z-fight otherwise.
                    let restore = self.index_layer;
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                    // Hole-aware fill (#942): the wall of a ring highlights as the wall, not as
                    // the whole outline with its interior filled in.
                    for tri in crate::polygon::triangulate_planar_with_holes(
                        world_loop, holes, normal,
                    ) {
                        self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                    }
                    for loop_ in std::iter::once(world_loop).chain(holes.iter()) {
                        let n = loop_.len();
                        for i in 0..n {
                            let j = (i + 1) % n;
                            self.push_line_segment(
                                loop_[i],
                                loop_[j],
                                color,
                                2.0,
                                cam,
                                viewport,
                                view_proj,
                            );
                        }
                    }
                    self.set_index_layer(restore);
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
            FaceId::RevolveCap { .. }
            | FaceId::RevolveSide { .. }
            | FaceId::UnitFace { .. }
            | FaceId::PrimitiveFace { .. }
            | FaceId::RepeatedFace { .. }
            | FaceId::BodyMeshFace { .. } => {
                let poly = match face {
                    FaceId::RevolveCap {
                        revolution,
                        ref profile,
                        end,
                    } => crate::extrude::revolve_cap_polygon_world(doc, revolution, profile, end)
                        .map(|(poly, _)| poly),
                    FaceId::RevolveSide {
                        revolution,
                        ref profile,
                        edge,
                    } => crate::extrude::revolve_side_geom(doc, revolution, profile, edge as usize)
                        .map(|(poly, _, _)| poly),
                    // A unit's flat face (#725): its placed boundary polygon.
                    FaceId::UnitFace { instance, ref face } => {
                        crate::units::unit_face_world_polygon(doc, instance, face)
                    }
                    FaceId::PrimitiveFace { primitive, face } => {
                        doc.primitives
                            .get(primitive)
                            .and_then(|shape| crate::primitives::face_polygon(doc, shape, face))
                    }
                    FaceId::RepeatedFace { .. } | FaceId::BodyMeshFace { .. } => {
                        crate::extrude::face_boundary_loop_world(doc, &face)
                    }
                    _ => None,
                };
                if let Some(poly) = poly {
                    let n = poly.len();
                    for i in 0..n {
                        let j = (i + 1) % n;
                        self.push_line_segment(
                            poly[i], poly[j], color, width, cam, viewport, view_proj,
                        );
                    }
                }
            }
            // The plane's own rectangle, lifted with its fill so the two don't z-fight (#974).
            FaceId::ConstructionPlane(index) => {
                if let Some(plane) = doc.construction_planes.get(index) {
                    let bias = plane_fill_depth_bias(index) + HOVER_PLANE_DEPTH_LIFT;
                    let corners = offset_corners_toward_camera(
                        plane_corners(plane),
                        plane.normal,
                        cam.eye(),
                        bias,
                    );
                    for i in 0..corners.len() {
                        self.push_line_segment(
                            corners[i],
                            corners[(i + 1) % corners.len()],
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
    }

    /// Viewport highlight for an elements-pane hover (#161): per element kind, in `color`.
    /// Drawn depth-test-disabled like other pick highlights (#153).
    fn push_element_hover(
        &mut self,
        doc: &Document,
        element: crate::hierarchy::SceneElement,
        color: Color32,
        _body_meshes: &std::collections::HashMap<crate::model::BodyKey, Option<crate::extrude::SolidMesh>>,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        use crate::hierarchy::SceneElement;
        let project = |w: Vec3| cam.project(w, viewport, view_proj);
        let restore = self.index_layer;
        self.set_index_layer(MeshIndexLayer::Wireframe);
        match element {
            // Nothing on a drawing page draws in the 3D viewport (#967).
            SceneElement::DrawingElement { .. } => {}
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
            // A selected/hovered world axis lights up its origin triad leg (#952), the same
            // stroke its pick-target hover already draws.
            SceneElement::GlobalAxis(axis) => {
                self.push_pick_target_highlight(
                    doc,
                    &PickTargetKind::GlobalAxis(axis),
                    color,
                    cam,
                    viewport,
                    view_proj,
                    &project,
                );
            }
            // An extrusion's analytic edge (#952) lights up **whole**: a hole's rim is one
            // `Cap` reference but many mesh chords, so drawing one chord would make a circle
            // read as a row of facets (#807).
            SceneElement::ExtrusionEdge { extrusion, edge } => {
                let solid = crate::model::TreatableSolid::Extrusion(extrusion);
                for (a, b) in crate::extrude::treatable_edges(doc)
                    .into_iter()
                    .filter(|(s, r, _, _)| *s == solid && *r == edge)
                    .map(|(_, _, a, b)| (a, b))
                {
                    self.push_polyline_segment(&[a, b], color, 3.0, cam, viewport, view_proj);
                }
            }
            SceneElement::PrimitiveEdge { primitive, edge } => {
                let solid = crate::model::TreatableSolid::Primitive(primitive);
                for (a, b) in crate::extrude::treatable_edges(doc)
                    .into_iter()
                    .filter(|(s, r, _, _)| *s == solid && *r == edge)
                    .map(|(_, _, a, b)| (a, b))
                {
                    self.push_polyline_segment(&[a, b], color, 3.0, cam, viewport, view_proj);
                }
            }
            // A Move/Joint snap point (#952) lights up as the point it resolves to.
            SceneElement::MovePoint(point) => {
                if let Some(world) = crate::extrude::move_point_world(doc, &point) {
                    self.push_pick_target_highlight(
                        doc,
                        &PickTargetKind::BodyVertex {
                            // The origin belongs to no body; a highlight on it needs a
                            // key all the same, and a vacant slot resolves to nothing.
                            body: point
                                .body()
                                .unwrap_or_else(|| crate::arena::Key::from_bits(u64::MAX)),
                            position: world,
                        },
                        color,
                        cam,
                        viewport,
                        view_proj,
                        &project,
                    );
                }
            }
            // A repeat instance's face (#955): the source face's boundary run through that
            // instance's transform — the plane the user snapped to, not the original.
            SceneElement::RepeatedFace { face, op, instance } => {
                if let (Some(loop_), Some(xf)) = (
                    crate::construction::sketch_face_boundary_world(doc, &face),
                    doc.repeat_ops
                        .get(op)
                        .and_then(|o| crate::extrude::repeat_instance_transform(doc, o, instance)),
                ) {
                    let moved: Vec<Vec3> =
                        loop_.iter().map(|p| xf.transform_point3(*p)).collect();
                    for i in 0..moved.len() {
                        self.push_polyline_segment(
                            &[moved[i], moved[(i + 1) % moved.len()]],
                            color,
                            3.0,
                            cam,
                            viewport,
                            view_proj,
                        );
                    }
                }
            }
            // An analytic face (#952/#958) hovers as a **face**: a translucent fill with a
            // bright border, the same treatment `ViewportHoverHighlight::SketchFace` gives it.
            // It used to light only its boundary loop, which read as "these edges" rather than
            // "this surface" — the difference mattered once the face-picking tools' hover
            // started coming from their pickers rather than a hand-written arm per tool.
            // Body-coplanar fills + borders are depth-disabled (#1139).
            SceneElement::SketchFace(face) => {
                let body_coplanar = !matches!(face, FaceId::ConstructionPlane(_));
                let restore = self.index_layer;
                if body_coplanar {
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                }
                self.push_sketch_face_hover(
                    doc,
                    face.clone(),
                    color,
                    FACE_HOVER_FILL_MULTIPLIER,
                    cam,
                );
                self.push_sketch_face_hover_border(
                    doc,
                    face,
                    color,
                    2.0,
                    cam,
                    viewport,
                    view_proj,
                );
                if body_coplanar {
                    self.set_index_layer(restore);
                }
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
                for (li, line) in doc.lines.iter() {
                    if line.sketch == sketch {
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
                for (ci, circle) in doc.circles.iter() {
                    if circle.sketch == sketch {
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
            SceneElement::BodyEdge { body, a, b } => {
                let wa = crate::hierarchy::dequantize_body_point(a);
                let wb = crate::hierarchy::dequantize_body_point(b);
                // The whole tangent-continuous curve, not just the identity facet (#626).
                let chain = crate::extrude::body_solid_mesh(doc, body)
                    .map(|s| body_edge_curve_chain(&s, wa, wb))
                    .unwrap_or_else(|| vec![(wa, wb)]);
                for (sa, sb) in chain {
                    self.push_segment_hover(sa, sb, color, cam, viewport, view_proj, &project);
                }
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
                if let Some(tris) = crate::extrude::body_face_triangles(doc, body, centroid, normal)
                {
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
            // A hovered cylinder or its centre line (#1013): the round wall's own facets, and
            // the axis drawn as a line through them.
            SceneElement::BodyCylinder { body, origin, dir, radius } => {
                if let Some(cyl) =
                    crate::extrude::body_cylinder_matching(doc, body, origin, dir, radius)
                {
                    self.push_pick_target_highlight(
                        doc,
                        &PickTargetKind::BodyCylinder { body, cylinder: Box::new(cyl) },
                        color,
                        cam,
                        viewport,
                        view_proj,
                        &project,
                    );
                }
            }
            SceneElement::BodyAxis { body, origin, dir } => {
                if let Some((a, b)) = crate::extrude::body_axis_segment(doc, body, origin, dir) {
                    self.push_pick_target_highlight(
                        doc,
                        &PickTargetKind::BodyAxis { body, a, b },
                        color,
                        cam,
                        viewport,
                        view_proj,
                        &project,
                    );
                }
            }
            // A hovered sketch text (#307): trace its glyph outlines in the hover color, so
            // the Extrude tool's "click picks the whole string" affordance is visible.
            SceneElement::SketchText(ti) => {
                let Some(text) = doc.sketch_texts.get(ti) else {
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
            // A sketched-on face's own boundary edge (#199/#974): the same segment the
            // *selection* highlight draws. It used to fall in the silent group below, so
            // hovering one lit nothing while selecting it lit up — the same shape of gap the
            // construction plane had.
            SceneElement::FaceEdge(crate::model::ConstraintLine::FaceEdge { face, index }) => {
                if let Ok((a, b)) =
                    crate::geometric_constraints::face_edge_world(doc, &face, index)
                {
                    self.push_line_segment(a, b, color, 4.0, cam, viewport, view_proj);
                }
            }
            // A tracing image (#170/#977): its quad's outline, so pointing at its row says
            // where on the plane it sits.
            SceneElement::Image(index) => {
                if let Some(corners) = tracing_image_corners(doc, index) {
                    for i in 0..corners.len() {
                        self.push_line_segment(
                            corners[i],
                            corners[(i + 1) % corners.len()],
                            color,
                            3.0,
                            cam,
                            viewport,
                            view_proj,
                        );
                    }
                }
            }
            // Everything whose own shape isn't in the 3D view lights what it **made** instead
            // (#977): an operation its output bodies, a component every body under it, a joint
            // the parts it joins. In a colour of their own — the plain hover colour would claim
            // those bodies are what the cursor is on, and it's on the row.
            //
            // Fill (and wireframe line colour) recolour in the main body pass (#1150), like
            // body hover (#455). A translucent coplanar overlay of the same mesh used to
            // z-fight the solid into a mottled checkerboard — especially bad for Slice, where
            // the cut faces and outer walls sit on top of themselves.
            SceneElement::BooleanOp(_)
            | SceneElement::MoveOp(_)
            | SceneElement::MirrorOp(_)
            | SceneElement::RepeatOp(_)
            | SceneElement::SketchRepeatOp(_)
            | SceneElement::SketchOffsetOp(_)
            | SceneElement::SketchMirrorOp(_)
            | SceneElement::SketchVertexTreatmentOp(_)
            | SceneElement::SketchSliceOp(_)
            | SceneElement::SliceOp(_)
            | SceneElement::ShellOp(_)
            | SceneElement::EdgeTreatmentOp(_)
            | SceneElement::Revolution(_)
            | SceneElement::Shape(_)
            | SceneElement::SweepOp(_)
            | SceneElement::Joint(_)
            | SceneElement::Component(_) => {}
            // The origin and the sketch axes draw their own hover, where the sketch frame that
            // places them is in hand (see the origin marker in `build`).
            SceneElement::FaceEdge(_) | SceneElement::Origin => {}
            // A selected/hovered unit instance outlines its placed meshes (#723).
            SceneElement::UnitInstance(index) => {
                for solid in crate::units::placed_instance_meshes(doc, index) {
                    self.push_solid_wireframe(&solid, None, color, cam, viewport, view_proj);
                }
            }
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
            PickTargetKind::BodyEdge { body, a, b } => {
                // The whole tangent-continuous curve, not just the picked facet (#626).
                let chain = crate::extrude::body_solid_mesh(doc, *body)
                    .map(|s| body_edge_curve_chain(&s, *a, *b))
                    .unwrap_or_else(|| vec![(*a, *b)]);
                for (sa, sb) in chain {
                    self.push_segment_hover(sa, sb, color, cam, viewport, view_proj, project);
                }
            }
            PickTargetKind::BodyFace {
                triangles, normal, ..
            } => {
                let eye = cam.eye();
                let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                let lift =
                    |p: Vec3| offset_toward_camera(p, *normal, eye, HOVER_FILL_DEPTH_BIAS);
                // Depth-disabled (#555/#1139): coplanar with the solid, so a depth-tested
                // fill z-fights the body into a mottled checkerboard. Callers that already
                // switched to Wireframe (PickTarget hover) keep that; Element hover did not.
                let restore = self.index_layer;
                self.set_index_layer(MeshIndexLayer::Wireframe);
                for tri in triangles {
                    self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                }
                for (a, b) in crate::construction::coplanar_face_boundary(triangles) {
                    self.push_line_segment(a, b, color, 3.0, cam, viewport, view_proj);
                }
                self.set_index_layer(restore);
            }
            // A round wall (#1013): its own facets, lifted toward the camera like a face's.
            PickTargetKind::BodyCylinder { cylinder, .. } => {
                let eye = cam.eye();
                let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
                // Same coplanar-with-body problem as BodyFace (#1139).
                let restore = self.index_layer;
                self.set_index_layer(MeshIndexLayer::Wireframe);
                for tri in &cylinder.triangles {
                    let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                    let lift = |p: Vec3| offset_toward_camera(p, n, eye, HOVER_FILL_DEPTH_BIAS);
                    self.push_triangle(lift(tri[0]), lift(tri[1]), lift(tri[2]), fill);
                }
                self.set_index_layer(restore);
            }
            // Its centre line: the segment the surface spans.
            PickTargetKind::BodyAxis { a, b, .. } => {
                self.push_segment_hover(*a, *b, color, cam, viewport, view_proj, project);
            }
            PickTargetKind::BodyVertex { position, .. } => {
                push_screen_disc(self, *position, 5.0, color, cam, viewport, view_proj, project);
            }
            PickTargetKind::GlobalAxis(axis) => {
                let (a, b) = global_axis_segment(*axis);
                let axis_color = axis.color().gamma_multiply(1.25);
                // Hover width only — selection uses a thicker bleed-through stroke (#1124).
                self.push_line_segment(
                    a,
                    b,
                    axis_color,
                    ORIGIN_AXIS_HOVER_WIDTH_PX,
                    cam,
                    viewport,
                    view_proj,
                );
            }
            // A plane reaches the crowd both as itself and as an analytic face over the same
            // surface, and the dedupe keeps whichever is nearer — so the two must draw the
            // same thing, through the same helpers (#974).
            PickTargetKind::ConstructionPlane(index) => {
                let face = FaceId::ConstructionPlane(*index);
                self.push_sketch_face_hover(doc, face.clone(), color, FACE_HOVER_FILL_MULTIPLIER, cam);
                self.push_sketch_face_hover_border(
                    doc,
                    face,
                    color,
                    2.0,
                    cam,
                    viewport,
                    view_proj,
                );
            }
            PickTargetKind::Ground(p) => {
                push_ground_hover_marker(self, *p, color, cam, viewport, view_proj, project);
            }
            // A constraint's hover highlight is its badge glowing in the 2D annotation overlay
            // (#568), not a world-geometry marker — nothing to push into the 3D scene here.
            PickTargetKind::Constraint(_) => {}
            // A whole body (#902) recolors in the main pass, like a hovered body row.
            PickTargetKind::Body(_) => {}
            // An analytic sketchable face (#625): same fill + border a face-picking tool's own
            // hover uses. Body-coplanar fills + borders are depth-disabled (#1139); the
            // PickTarget arm already sets Wireframe for all kinds, but ConstructionPlane
            // stays depth-tested when drawn through the face helper alone.
            PickTargetKind::SketchFace(face) => {
                let body_coplanar = !matches!(face, FaceId::ConstructionPlane(_));
                let restore = self.index_layer;
                if body_coplanar {
                    self.set_index_layer(MeshIndexLayer::Wireframe);
                }
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
                if body_coplanar {
                    self.set_index_layer(restore);
                }
            }
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

pub fn plane_fill_depth_bias(index: crate::model::ConstructionPlaneKey) -> f32 {
    PLANE_FILL_DEPTH_BIAS - index.index() as f32 * SHAPE_FILL_DEPTH_BIAS_STEP * 0.25
}

fn plane_camera_depth(plane: &ConstructionPlane, cam: &Camera) -> f32 {
    let corners = plane_corners(plane);
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
/// Quantize a world point for mesh-edge/vertex identity (0.001 world-unit bins).
/// Shared with body-vertex picking so "feature corner" agrees with feature edges (#1118).
pub fn quantize_vertex(v: Vec3) -> (i64, i64, i64) {
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

/// Turn threshold for chaining feature segments into one smooth "curve edge" (#626):
/// consecutive segments whose directions differ by less than this continue the same curve
/// (a tessellated circle at `CIRCLE_SEGMENTS` = 48 turns 7.5° per facet); sharper turns are
/// real corners and break the chain.
const CURVE_CHAIN_COS_THRESHOLD: f32 = 0.866_025; // cos(30°)

/// Group items into maximal tangent-continuous chains by their ends (#626/#984): at a vertex
/// where **exactly two** item-ends meet and their away-from-the-vertex directions are nearly
/// opposite (within [`CURVE_CHAIN_COS_THRESHOLD`]), the two items join one chain; corners,
/// junctions of 3+ ends, and free ends stay boundaries. Each item is its two
/// `(quantized vertex key, direction pointing away from that vertex into the item)` ends.
///
/// This is the one chaining rule, shared by the solid-mesh feature-edge chains and the
/// sketch-line chains — the two differ only in how a vertex is keyed and a tangent read.
pub fn chain_by_tangency(ends: &[[((i64, i64, i64), Vec3); 2]]) -> Vec<Vec<usize>> {
    let n = ends.len();
    let mut adj: std::collections::HashMap<(i64, i64, i64), Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    for (i, item) in ends.iter().enumerate() {
        for (which, (key, _)) in item.iter().enumerate() {
            adj.entry(*key).or_default().push((i, which));
        }
    }
    // Union-find with path halving; union at every smooth 2-end vertex.
    let mut parent: Vec<usize> = (0..n).collect();
    let find = |parent: &mut Vec<usize>, mut i: usize| -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    };
    for incident in adj.values() {
        let [(i, i_end), (j, j_end)] = incident[..] else {
            continue;
        };
        // A smooth continuation means the two away-directions are nearly opposite.
        if ends[i][i_end].1.dot(ends[j][j_end].1) <= -CURVE_CHAIN_COS_THRESHOLD {
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut by_root: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        by_root.entry(r).or_default().push(i);
    }
    by_root.into_values().collect()
}

/// Partition a solid's feature edges into maximal tangent-continuous chains (#626): at every
/// vertex where **exactly two** feature segments meet at a shallow angle — the tessellation
/// of a smooth curve, like a revolve's circular rim — the segments join one chain; corners,
/// junctions of 3+ edges, and chain ends stay boundaries. A straight or curved edge thus
/// becomes **one** chain of segments, so picking any facet can select the whole curve.
pub fn solid_mesh_edge_chains(solid: &crate::extrude::SolidMesh) -> Vec<Vec<(Vec3, Vec3)>> {
    let edges = solid_mesh_unique_edges(solid);
    let ends: Vec<[((i64, i64, i64), Vec3); 2]> = edges
        .iter()
        .map(|(a, b)| {
            let d = (*b - *a).normalize_or_zero();
            [(quantize_vertex(*a), d), (quantize_vertex(*b), -d)]
        })
        .collect();
    chain_by_tangency(&ends)
        .into_iter()
        .map(|chain| chain.into_iter().map(|i| edges[i]).collect())
        .collect()
}

/// The canonical identity segment of a chain (#626): the lexicographically smallest
/// quantized segment, so every facet of one curve maps to the same selection element.
pub fn chain_canonical_segment(chain: &[(Vec3, Vec3)]) -> (Vec3, Vec3) {
    let key = |&(a, b): &(Vec3, Vec3)| {
        let (qa, qb) = (quantize_vertex(a), quantize_vertex(b));
        if qa <= qb { (qa, qb) } else { (qb, qa) }
    };
    *chain
        .iter()
        .min_by_key(|seg| key(seg))
        .expect("chains are never empty")
}

/// The full tangent-continuous chain through segment `(a, b)` (#626) — the segments a
/// highlight should draw when that curve is hovered or selected. Endpoints are matched by
/// proximity, not exact keys: a selected edge's geometry round-trips through
/// `hierarchy::quantize_body_point` (0.01 world units), so the dequantized points can sit
/// up to half that step off the mesh's true vertices. Falls back to the lone segment when
/// nothing matches (e.g. stale selection geometry after a rebuild).
pub fn body_edge_curve_chain(
    solid: &crate::extrude::SolidMesh,
    a: Vec3,
    b: Vec3,
) -> Vec<(Vec3, Vec3)> {
    const EPS_SQ: f32 = 0.006 * 0.006;
    let near = |p: Vec3, q: Vec3| (p - q).length_squared() <= EPS_SQ;
    for chain in solid_mesh_edge_chains(solid) {
        let hit = chain
            .iter()
            .any(|&(x, y)| (near(x, a) && near(y, b)) || (near(x, b) && near(y, a)));
        if hit {
            return chain;
        }
    }
    vec![(a, b)]
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
        let angle_color = if angle_hovered {
            GIZMO_HANDLE_HOVER_RGBA
        } else {
            color
        };
        // Unify with the Face Snap / Free Move rotation dial (#1384): a radial line from
        // the centre to the 0° reference, the yellow arc up to the current angle, a radial
        // line centre→handle and a single disc at the handle.
        let start_dir = perp.normalize_or_zero();
        if axis != Vec3::ZERO && start_dir != Vec3::ZERO {
            self.push_line_segment(
                origin,
                origin + start_dir * AXIS_ANGLE_GIZMO_RADIUS_MM,
                angle_color,
                1.5,
                cam,
                viewport,
                view_proj,
            );
            if angle_deg.abs() > 1e-3 {
                let arc = revolve_arc_points(
                    origin,
                    axis,
                    start_dir,
                    AXIS_ANGLE_GIZMO_RADIUS_MM,
                    angle_deg,
                    GIZMO_ANGLE_CIRCLE_SEGMENTS,
                );
                self.push_polyline_segment(
                    &arc,
                    MOVE_ROTATION_ARC,
                    2.5,
                    cam,
                    viewport,
                    view_proj,
                );
            }
            if handle_dir != Vec3::ZERO {
                self.push_line_segment(
                    origin,
                    handle,
                    angle_color,
                    2.0,
                    cam,
                    viewport,
                    view_proj,
                );
            }
        }
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
/// The deterministic ring-plane reference for a rotation gizmo with no pinned zero direction:
/// the ring's own `u` basis vector used to sample a full circle (#1405).
fn ring_reference(axis: Vec3) -> Vec3 {
    let n = axis.normalize_or_zero();
    if n == Vec3::ZERO {
        return Vec3::ZERO;
    }
    let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    n.cross(reference).normalize_or_zero()
}

/// The rotation gizmo dial (#1405): instead of a full circle, two fading arcs extend 30° in
/// each direction from the handle, the handle floats at its (rotated) reference with a
/// direction arrow on each side pointing along an arc. When a live turn is set, the fade arcs
/// follow the handle but are painted underneath the sweep arc (drawn first, so it reads on
/// top). Replaces [`ring_points`] for the Move-tool rotation gizmos.
#[allow(clippy::too_many_arguments)]
fn push_rotation_gizmo(
    mesh: &mut SceneMesh<'_>,
    gizmo: &MoveRotationGizmo,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let n = gizmo.axis.normalize_or_zero();
    let start_dir = gizmo
        .zero_dir
        .map(|z| z.normalize_or_zero())
        .filter(|z| *z != Vec3::ZERO)
        .unwrap_or_else(|| ring_reference(n));
    if n == Vec3::ZERO || start_dir == Vec3::ZERO {
        return;
    }
    let angle_deg = gizmo.angle_deg.unwrap_or(0.0);
    let handle_dir = glam::Quat::from_axis_angle(n, angle_deg.to_radians()) * start_dir;
    let handle_pos = gizmo.center + handle_dir * gizmo.radius;
    let stroke = if gizmo.hovered { 3.0 } else { 1.5 };

    // While the handle is held, the full thin circle of rotation (#1420).
    if gizmo.dragging {
        push_rotation_full_circle(
            mesh,
            gizmo.center,
            n,
            start_dir,
            gizmo.radius,
            gizmo.color.gamma_multiply(0.7),
            1.0,
            cam,
            viewport,
            view_proj,
        );
    }

    // The fading arcs either side of the handle, drawn first so a live sweep stays on top.
    // Pulling off 0° drops the fade on the unused side (#1420).
    push_rotation_fade_arcs(
        mesh,
        gizmo.center,
        n,
        handle_dir,
        gizmo.radius,
        gizmo.color,
        stroke,
        crate::extrude::rotation_fade_arc_signs(angle_deg),
        cam,
        viewport,
        view_proj,
    );

    // The 0° reference radial (dashed) and, once turned off 0, the yellow sweep plus the
    // solid handle radial — both painted on top of the fading arcs (#1419).
    if gizmo.zero_dir.is_some() && gizmo.angle_deg.is_some() && angle_deg.abs() > 1e-3 {
        let original = gizmo.center + start_dir * gizmo.radius;
        mesh.push_dashed_line_segment(
            gizmo.center,
            original,
            gizmo.color,
            GIZMO_ROTATION_RADIAL_STROKE_PX,
            cam,
            viewport,
            view_proj,
        );
        let arc = revolve_arc_points(
            gizmo.center,
            n,
            start_dir,
            gizmo.radius,
            angle_deg,
            64,
        );
        mesh.push_polyline_segment(&arc, MOVE_ROTATION_ARC, 2.5, cam, viewport, view_proj);
        mesh.push_line_segment(
            gizmo.center,
            handle_pos,
            gizmo.color,
            GIZMO_ROTATION_RADIAL_STROKE_PX,
            cam,
            viewport,
            view_proj,
        );
    }

    // One direction arrow on each side of the handle, pointing along an arc.
    let tangent = n.cross(handle_dir).normalize_or_zero();
    if tangent != Vec3::ZERO {
        for sign in [1.0f32, -1.0] {
            let dir = tangent * sign;
            let offset =
                crate::dimensions::pixels_to_world_distance(project, handle_pos, dir, GIZMO_ROTATION_ARROW_OFFSET_PX);
            let tip = handle_pos + dir * offset;
            push_gizmo_arrowhead(
                mesh,
                tip,
                dir,
                GIZMO_ARROW_HEAD_PX,
                GIZMO_ARROW_WING_PX,
                stroke,
                gizmo.color,
                cam,
                viewport,
                view_proj,
                project,
            );
        }
    }

    if gizmo.hovered {
        push_gizmo_handle_hover(
            mesh,
            handle_pos,
            GIZMO_HANDLE_HOVER_RGBA,
            cam,
            viewport,
            view_proj,
            project,
        );
    }
    push_gizmo_handle(mesh, handle_pos, gizmo.color, cam, viewport, view_proj, project);
}

/// Full thin circle of rotation, shown only while a handle is held (#1420).
#[allow(clippy::too_many_arguments)]
fn push_rotation_full_circle(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    axis: Vec3,
    start_dir: Vec3,
    radius: f32,
    color: Color32,
    stroke_px: f32,
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
) {
    let n = axis.normalize_or_zero();
    let dir0 = start_dir.normalize_or_zero();
    if n == Vec3::ZERO || dir0 == Vec3::ZERO || radius <= 0.0 {
        return;
    }
    let mut prev: Option<Vec3> = None;
    for i in 0..=GIZMO_ANGLE_CIRCLE_SEGMENTS {
        let a = i as f32 / GIZMO_ANGLE_CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
        let pt = center + (glam::Quat::from_axis_angle(n, a) * dir0) * radius;
        if let Some(p0) = prev {
            mesh.push_line_segment(p0, pt, color, stroke_px, cam, viewport, view_proj);
        }
        prev = Some(pt);
    }
}

/// Arcs through `±GIZMO_ROTATION_FADE_ARC_DEG` from `handle_dir` along `signs`, fading
/// from `color` at the handle out to transparent at their tips (#1405/#1420).
#[allow(clippy::too_many_arguments)]
fn push_rotation_fade_arcs(
    mesh: &mut SceneMesh<'_>,
    center: Vec3,
    axis: Vec3,
    handle_dir: Vec3,
    radius: f32,
    color: Color32,
    stroke_px: f32,
    signs: &[f32],
    cam: &Camera,
    viewport: UiRect,
    view_proj: &Mat4,
) {
    let n = axis.normalize_or_zero();
    let hdir = handle_dir.normalize_or_zero();
    if n == Vec3::ZERO || hdir == Vec3::ZERO || radius <= 0.0 {
        return;
    }
    let half = GIZMO_ROTATION_FADE_ARC_DEG.to_radians();
    let segments = GIZMO_ROTATION_FADE_ARC_SEGMENTS.max(1);
    for &sign in signs {
        let mut prev: Option<Vec3> = None;
        for i in 0..=segments {
            let frac = i as f32 / segments as f32;
            let dt = sign * half * frac;
            let dir = glam::Quat::from_axis_angle(n, dt) * hdir;
            let pt = center + dir * radius;
            if let Some(p0) = prev {
                let alpha = (color.a() as f32 * (1.0 - frac)).round() as u8;
                if alpha > 0 {
                    let c =
                        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
                    mesh.push_line_segment(p0, pt, c, stroke_px, cam, viewport, view_proj);
                }
            }
            prev = Some(pt);
        }
    }
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
        mesh.indices_mut()
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


/// Ambient/diffuse/specular weights for `ShadingMode::Realistic` (#83) — a fixed matte/satin
/// "painted object" look; no per-material tuning yet.
///
/// Since #1037 the lighting itself runs per pixel in `shader.wgsl`, which is the single
/// source of truth. These, and [`realistic_shade`] below, are the CPU mirror the tests
/// exercise — `realistic_terms_match_the_shader` asserts the two stay in step.
/// The scene's one fixed light direction. The fragment shader lights solids against this
/// (passed through the uniforms), and [`realistic_shade`] uses it on the CPU for tests.
pub const SCENE_LIGHT_DIR: Vec3 = Vec3::new(0.35, 0.45, 0.82);

#[cfg(test)]
const REALISTIC_AMBIENT: f32 = 0.0707;
#[cfg(test)]
const REALISTIC_DIFFUSE: f32 = 0.6287;
#[cfg(test)]
const REALISTIC_SPECULAR: f32 = 0.35;
#[cfg(test)]
const REALISTIC_SHININESS: f32 = 24.0;
/// Weight of the camera-attached "headlight" diffuse term (#102). The fixed light sits
/// above-ish the scene, so from horizontal views a camera-facing wall got no diffuse at all
/// and rendered at the ambient floor — nearly black, *less* readable than `Solid` mode. The
/// headlight guarantees a face square to the camera reaches ambient + 0.7·diffuse ≈ 0.69
/// total, on par with `Solid`'s ~0.67 for the same face. Combined with `max()` (not summed)
/// so the fixed light still dominates wherever it lands, preserving per-face contrast/shape.
#[cfg(test)]
const REALISTIC_HEADLIGHT: f32 = 0.70;

/// Blinn-Phong-ish flat shading for one triangle face, two-sided (the normal is flipped to
/// face the camera first, matching `push_solid`'s two-sided convention): ambient + diffuse +
/// a camera-dependent specular highlight, instead of `push_solid`'s single Lambert-ish term.
/// The diffuse term takes the stronger of the fixed scene light and a camera headlight
/// (#102), so surfaces the user is looking at are always reasonably lit.
#[cfg(test)]
fn realistic_shade(base: Color32, normal: Vec3, light: Vec3, view: Vec3) -> Color32 {
    let n = if normal.dot(view) < 0.0 { -normal } else { normal };
    let fixed_diffuse = n.dot(light).max(0.0);
    let headlight_diffuse = REALISTIC_HEADLIGHT * n.dot(view).max(0.0);
    let diffuse = fixed_diffuse.max(headlight_diffuse);
    let half = (light + view).normalize_or_zero();
    let specular = n.dot(half).max(0.0).powf(REALISTIC_SHININESS);
    let lit = |channel: u8| -> u8 {
        let linear = srgb_to_linear(channel as f32 / 255.0);
        let shaded = linear * (REALISTIC_AMBIENT + REALISTIC_DIFFUSE * diffuse)
            + REALISTIC_SPECULAR * specular;
        (linear_to_srgb(tonemap(shaded)) * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_unmultiplied(lit(base.r()), lit(base.g()), lit(base.b()), base.a())
}

/// sRGB transfer function and its inverse, and the tonemap — the CPU mirror of the same
/// three functions in `shader.wgsl` (#1038).
#[cfg(test)]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

/// Narkowicz's ACES fit, normalized so full white maps back to full white.
#[cfg(test)]
fn tonemap(x: f32) -> f32 {
    let aces = |v: f32| (v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14);
    (aces(x.max(0.0)) / ACES_WHITE).clamp(0.0, 1.0)
}

/// `aces(1.0)`; see the shader constant of the same name.
#[cfg(test)]
const ACES_WHITE: f32 = 0.8037036;

pub fn sketch_ground_color(color: Color32, in_sketch: bool) -> Color32 {
    if in_sketch {
        color.gamma_multiply(SKETCH_GROUND_DIMMED)
    } else {
        color
    }
}

/// Whether **construction** geometry belonging to `sketch` should be drawn at all (#994).
///
/// Construction lines and circles are scaffolding for the sketch that owns them — guides to
/// dimension and constrain against, never model geometry. Outside that sketch they are noise
/// standing on a face, indistinguishable from real edges except by being dashed, and they
/// clutter every view of the finished part. So they draw **only while their own sketch is
/// open**; solid geometry is unaffected and still dims when another sketch is active.
fn construction_geometry_visible(
    session: Option<SketchSession>,
    sketch: crate::model::SketchId,
) -> bool {
    session.is_some_and(|s| s.sketch == sketch)
}

/// Whether a sketch sits on a solid's face (extrude cap/side, etc.) rather than a datum
/// plane. Used for stroke colour contrast (#1149/#1167). Committed strokes on those faces
/// depth-test like plane sketches (#1174); hover/selection fills still use the depth-
/// disabled wireframe layer (#1139/#1140).
fn sketch_is_body_coplanar(doc: &Document, sketch: crate::model::SketchId) -> bool {
    match doc.sketch_face(sketch) {
        Some(FaceId::ConstructionPlane(_)) | None => false,
        Some(_) => true,
    }
}

/// Stroke colour for unconstrained solid sketch geometry (#1149/#1153/#1167):
/// - construction-plane sketches: plane blue ([`ViewportPalette::rect_line`])
/// - body-face sketches **while a sketch is open**: bright blue-grey — sketch mode dims
///   bodies (#433), so the dark stroke vanishes on the dimmed face
/// - body-face sketches **outside** sketch mode: dark or bright so the stroke contrasts
///   the undimmed face material (dark on light fills, light on dark fills)
fn solid_sketch_stroke_color(
    palette: &ViewportPalette,
    doc: &Document,
    sketch: crate::model::SketchId,
    dim: bool,
    sketch_session: Option<SketchSession>,
) -> Color32 {
    let base = if sketch_is_body_coplanar(doc, sketch) {
        on_body_sketch_stroke(palette, doc, sketch, sketch_session.is_some())
    } else {
        palette.rect_line
    };
    sketch_color(base, dim)
}

/// Body-face fill colour for stroke contrast (#1167): the owning body's material, or the
/// default solid fill when the sketch's face has no body.
fn sketch_body_fill(doc: &Document, sketch: crate::model::SketchId) -> Color32 {
    doc.sketch_face(sketch)
        .and_then(|face| crate::model::body_index_for_face(doc, &face))
        .and_then(|bi| doc.bodies.get(bi))
        .map(|body| body_material_fill(doc, body))
        .unwrap_or(SOLID_FILL)
}

/// Rec. 709 relative luminance of an sRGB colour (channels as authored, 0–1).
fn srgb_relative_luminance(color: Color32) -> f32 {
    let r = color.r() as f32 / 255.0;
    let g = color.g() as f32 / 255.0;
    let b = color.b() as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Pick the body-face stroke that stays readable (#1167).
///
/// - Sketch open: always the bright stroke (bodies are dimmed).
/// - Sketch closed: dark on light materials, bright on dark materials.
fn on_body_sketch_stroke(
    palette: &ViewportPalette,
    doc: &Document,
    sketch: crate::model::SketchId,
    sketch_open: bool,
) -> Color32 {
    if sketch_open {
        return palette.rect_line_on_body_in_sketch;
    }
    let face = sketch_body_fill(doc, sketch);
    // Match the seeded-material "light enough to shade" threshold in model materials.
    if srgb_relative_luminance(face) > 0.35 {
        palette.rect_line_on_body
    } else {
        palette.rect_line_on_body_in_sketch
    }
}

fn sketch_circle_is_active(
    doc: &Document,
    session: SketchSession,
    circle_index: crate::model::CircleKey,
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

/// Distance from `p` to the segment `a`→`b` (mirrors `dist_to_segment` in `shader.wgsl`).
/// Used to pin the round-cap capsule math that `fs_axis` applies to sketch strokes (#1202).
#[cfg(test)]
fn dist_to_segment_2d(p: egui::Vec2, a: egui::Vec2, b: egui::Vec2) -> f32 {
    let ab = b - a;
    let denom = ab.dot(ab);
    let t = if denom > 1e-12 {
        ((p - a).dot(ab) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + ab * t)).length()
}

/// Whether a screen-space point lies inside a round-capped stroke of half-width `half_px`
/// along segment `a`→`b` — the coverage test `fs_axis` uses (#1202).
#[cfg(test)]
fn point_in_stroke_capsule(
    p: egui::Vec2,
    a: egui::Vec2,
    b: egui::Vec2,
    half_px: f32,
) -> bool {
    dist_to_segment_2d(p, a, b) <= half_px
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
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::constraint_key_for_slot as nkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::joint_key_for_slot as jkey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::component_key_for_slot as ckey;
    use super::*;
    use crate::actions::AppState;
    use crate::model::FaceId;
    use egui::Rect as UiRect;

    fn test_viewport() -> UiRect {
        UiRect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0))
    }

    /// #1421: rotation-handle arrows stand off as far as the Move translation arrows.
    #[test]
    fn rotation_arrow_offset_matches_translation_gizmo() {
        assert_eq!(
            GIZMO_ROTATION_ARROW_OFFSET_PX,
            GIZMO_ARROW_GAP_PX + GIZMO_ARROW_HEAD_PX
        );
    }

    /// #1419: the original radial is dashed and both radials are thinner than the fade stroke.
    #[test]
    fn rotation_radials_are_thin_and_original_is_dashed() {
        assert!(rotation_radial_is_dashed(true));
        assert!(!rotation_radial_is_dashed(false));
        assert!(GIZMO_ROTATION_RADIAL_STROKE_PX < 1.5);
    }

    /// #1247: multi-turn revolve angles must not feed the full multi-turn into a fixed
    /// segment polyline (that drew a star of long chords). Display collapses to the last
    /// fractional turn (or one full turn for integer revolutions).
    #[test]
    fn revolve_arc_display_collapses_multi_turn() {
        assert!((revolve_arc_display_angle_deg(90.0) - 90.0).abs() < 1e-4);
        assert!((revolve_arc_display_angle_deg(360.0) - 360.0).abs() < 1e-4);
        assert!((revolve_arc_display_angle_deg(7200.0) - 360.0).abs() < 1e-3); // 20 revs
        assert!((revolve_arc_display_angle_deg(7380.0) - 180.0).abs() < 1e-3); // 20.5 revs
        assert!((revolve_arc_display_angle_deg(-7200.0) + 360.0).abs() < 1e-3);
        assert!((revolve_arc_display_angle_deg(-370.0) + 10.0).abs() < 1e-3);
    }

    /// #1247: with the collapsed display angle, consecutive arc samples stay on a smooth
    /// circle (chord span ≲ 360°/64), never the ~100° jumps that made the star.
    #[test]
    fn revolve_arc_multi_turn_samples_stay_smooth() {
        let center = Vec3::ZERO;
        let axis = Vec3::Y;
        let zero_dir = Vec3::X;
        let radius = 10.0;
        let display = revolve_arc_display_angle_deg(20.0 * 360.0);
        let pts = revolve_arc_points(center, axis, zero_dir, radius, display, 64);
        assert_eq!(pts.len(), 65);
        let max_step_deg = pts
            .windows(2)
            .map(|w| {
                let a = (w[0] - center).normalize_or_zero();
                let b = (w[1] - center).normalize_or_zero();
                a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(0.0_f32, f32::max);
        assert!(
            max_step_deg < 10.0,
            "multi-turn arc step {max_step_deg}° is a star chord, not a smooth circle"
        );
    }

    fn build_scene_with_shading(
        state: &AppState,
        mode: crate::camera::ShadingMode,
    ) -> ViewportScene {
        build_scene_with_ghosts(state, mode, Vec::new())
    }

    fn build_scene_with_ghosts(
        state: &AppState,
        mode: crate::camera::ShadingMode,
        repeat_ghosts: Vec<crate::extrude::SolidMesh>,
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts,
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
        
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,

        });
        assert_eq!(state.doc.bodies.len(), 1);
        state
    }

    /// #743: a preview ghost's feature edges draw into the always-on-top wireframe
    /// layer, so a Move preview landing flush against — or inside — stationary geometry
    /// stays readable instead of being swallowed by the depth test.
    #[test]
    fn preview_ghost_edges_land_on_the_wireframe_overlay() {
        let state = state_with_one_body();
        let ghost = crate::extrude::SolidMesh {
            triangles: vec![
                [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.0)],
                [Vec3::ZERO, Vec3::new(10.0, 10.0, 0.0), Vec3::new(0.0, 10.0, 0.0)],
            ],
        };
        let scene =
            build_scene_with_ghosts(&state, crate::camera::ShadingMode::Solid, vec![ghost]);
        assert!(
            !scene.wireframe_indices.is_empty(),
            "ghost outline draws on the depth-test-free overlay layer"
        );
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

    /// #1037: lighting moved into `shader.wgsl`, so the CPU `realistic_shade` above is a
    /// mirror rather than the implementation. The two can drift silently — nothing compiles
    /// the WGSL against the Rust — so pin every shared constant to the shader source.
    /// Nothing in the build compiles the WGSL — a GPU device does, at startup, on a machine
    /// that has one. So parse and validate it here (#1073), with wgpu's own front end, and a
    /// typo fails the test suite instead of the app.
    #[test]
    fn the_shader_parses_and_validates() {
        let wgsl = include_str!("shader.wgsl");
        let module = naga::front::wgsl::parse_str(wgsl)
            .unwrap_or_else(|e| panic!("shader.wgsl does not parse: {}", e.emit_to_string(wgsl)));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("shader.wgsl does not validate: {e:?}"));
        // Every entry point the renderer names must exist.
        let entries: Vec<&str> = module.entry_points.iter().map(|e| e.name.as_str()).collect();
        for name in [
            "vs_main", "fs_main", "vs_axis", "fs_axis", "vs_grid", "fs_grid",
            "fs_solid_ground", "vs_blit", "fs_blit", "fs_outline", "vs_text",
            "fs_text", "fs_image",
        ] {
            assert!(entries.contains(&name), "shader.wgsl has no `{name}`: {entries:?}");
        }
    }

    #[test]
    fn realistic_terms_match_the_shader() {
        let wgsl = include_str!("shader.wgsl");
        let shader_const = |name: &str| -> f32 {
            let line = wgsl
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("const {name}:")))
                .unwrap_or_else(|| panic!("shader.wgsl declares no `{name}`"));
            line.split('=')
                .nth(1)
                .and_then(|v| v.trim().trim_end_matches(';').parse::<f32>().ok())
                .unwrap_or_else(|| panic!("`{name}` in shader.wgsl is not a plain number"))
        };
        for (name, rust) in [
            ("ACES_WHITE", ACES_WHITE),
            ("REALISTIC_AMBIENT", REALISTIC_AMBIENT),
            ("REALISTIC_DIFFUSE", REALISTIC_DIFFUSE),
            ("REALISTIC_SPECULAR", REALISTIC_SPECULAR),
            ("REALISTIC_SHININESS", REALISTIC_SHININESS),
            ("REALISTIC_HEADLIGHT", REALISTIC_HEADLIGHT),
        ] {
            assert_eq!(shader_const(name), rust, "{name} drifted from the shader");
        }
        // The lighting-model discriminants the shader branches on ride in normal.w.
        for (name, model) in [
            ("MODE_UNLIT", ShadingModel::Unlit),
            ("MODE_LAMBERT", ShadingModel::Lambert),
            ("MODE_REALISTIC", ShadingModel::Realistic),
        ] {
            assert_eq!(shader_const(name), model.as_w(), "{name} drifted");
        }
    }

    /// #1037: geometry that isn't a body — lines, fills, text, gizmos, the grid — must stay
    /// unlit, or the shader would light 2D chrome against a 3D normal it doesn't have.
    #[test]
    fn only_body_solids_are_lit() {
        let state = state_with_one_body();
        let scene = build_scene_with_shading(&state, crate::camera::ShadingMode::Solid);
        let lit = scene
            .vertices
            .iter()
            .filter(|v| v.normal[3] != ShadingModel::Unlit.as_w())
            .count();
        assert!(lit > 0, "the body should contribute lit vertices");
        // Every lit vertex carries a usable normal; an unlit one is never lit by accident.
        for v in &scene.vertices {
            if v.normal[3] != ShadingModel::Unlit.as_w() {
                let n = Vec3::from_slice(&v.normal[..3]);
                assert!(n.length() > 0.5, "a lit vertex needs a unit normal, got {n}");
            }
        }
        // The ground grid alone is far more geometry than one box, so plenty stays unlit.
        let unlit = scene.vertices.len() - lit;
        assert!(unlit > 0, "chrome and the grid should stay unlit");
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
        // Facing directly away from both light and camera: diffuse and specular are both zero,
        // so the surface sits on the ambient floor.
        let shaded = realistic_shade(base, Vec3::new(0.0, 1.0, 0.0), Vec3::Z, Vec3::Z);
        let lit = realistic_shade(base, Vec3::Z, Vec3::Z, Vec3::Z);
        // Dimmer than the same face turned to the light, but nowhere near black — an
        // unlit face still has to read as its own material.
        assert!(shaded.r() < lit.r(), "{shaded:?} vs {lit:?}");
        assert!(shaded.r() > 40, "the ambient floor is too dark: {shaded:?}");
        // #1038: the floor is `REALISTIC_AMBIENT` of the base's **linear** luminance, which
        // re-encodes brighter than a naive 0.30 of the sRGB byte — that is the whole point
        // of lighting in linear space.
        let expect = |c: u8| -> u8 {
            let v = tonemap(srgb_to_linear(c as f32 / 255.0) * REALISTIC_AMBIENT);
            (linear_to_srgb(v) * 255.0).round() as u8
        };
        assert_eq!(
            (shaded.r(), shaded.g(), shaded.b()),
            (expect(180), expect(90), expect(40))
        );
    }

    /// #1038: the tonemap is normalized so adopting it does not darken the whole image —
    /// full white still comes out full white, and the curve is monotonic below it.
    #[test]
    fn the_tonemap_preserves_white_and_is_monotonic() {
        assert!((tonemap(1.0) - 1.0).abs() < 1e-3, "white became {}", tonemap(1.0));
        assert_eq!(tonemap(0.0), 0.0);
        let mut prev = 0.0;
        for i in 1..=50 {
            let v = tonemap(i as f32 / 25.0);
            assert!(v >= prev, "tonemap dipped at {i}: {v} after {prev}");
            assert!(v <= 1.0, "tonemap overshot at {i}: {v}");
            prev = v;
        }
    }

    /// #1038: the sRGB transfer function and its inverse actually round-trip, so decoding
    /// for lighting and re-encoding afterwards is not itself a colour shift.
    #[test]
    fn srgb_round_trips_through_linear() {
        for i in 0..=255u32 {
            let c = i as f32 / 255.0;
            let back = linear_to_srgb(srgb_to_linear(c));
            assert!((back - c).abs() < 1e-4, "{c} round-tripped to {back}");
        }
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let preview_plane = state.doc.construction_planes[pkey(0)].clone();
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
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
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            with_preview.overlay_indices.len() + with_preview.stroke_indices.len()
                > base.overlay_indices.len() + base.stroke_indices.len(),
            "plane creation preview should add outline geometry"
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
        assert!(!bezier_handles_relevant(lkey(0), &empty, &None, &[]));
        // A dragged/selected handle keeps its curve's handles up.
        assert!(bezier_handles_relevant(lkey(0), &empty, &None, &[(lkey(0), true)]));
        assert!(!bezier_handles_relevant(lkey(1), &empty, &None, &[(lkey(0), true)]));
        // The curve selected.
        let mut sel = SceneSelection::default();
        sel.insert(SceneElement::Line(lkey(0)));
        assert!(bezier_handles_relevant(lkey(0), &sel, &None, &[]));
        assert!(!bezier_handles_relevant(lkey(1), &sel, &None, &[]));
        // One of its endpoints selected.
        let mut sel_pt = SceneSelection::default();
        sel_pt.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: lkey(2),
            end: LineEnd::End,
        }));
        assert!(bezier_handles_relevant(lkey(2), &sel_pt, &None, &[]));
        assert!(!bezier_handles_relevant(lkey(3), &sel_pt, &None, &[]));
        // Hovering the curve (as a pick target).
        let hover = Some(ViewportHoverHighlight::PickTarget(PickTargetKind::Line(lkey(4))));
        assert!(bezier_handles_relevant(lkey(4), &empty, &hover, &[]));
        assert!(!bezier_handles_relevant(lkey(5), &empty, &hover, &[]));
    }

    /// How many overlay indices a hover highlight adds — 0 means it drew nothing.
    fn hover_overlay_indices(state: &AppState, hover: ViewportHoverHighlight) -> usize {
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |hover: Option<ViewportHoverHighlight>| {
            let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: hover.clone(),
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: crate::construction::PICK_HOVER_RGBA,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
            // Every buffer a hover can land in: face fills used to go to the overlay, strokes
            // to the screen-space stroke buffer (#1157), translucent solids to the plane-fill
            // layer, and pick targets (including point discs, #1359) to the
            // depth-test-disabled wireframe layer (#153). A check for "did this light up?" has
            // to count them all.
            scene.indices.len()
                + scene.sketch_fill_indices.len()
                + scene.plane_fill_indices.len()
                + scene.overlay_indices.len()
                + scene.stroke_indices.len()
                + scene.wireframe_indices.len()
                + scene.gizmo_indices.len()
        };
        build(Some(hover)) - build(None)
    }

    #[test]
    fn hover_highlight_adds_mesh_geometry() {
        let state = AppState::default();
        // The biased fill quad (6 indices) plus its four border segments (#974): a datum plane
        // hovers like every other face, fill and outline both, rather than through a special
        // case that gave it only the fill. Border strokes are screen-space (#1157): 4×6
        // indices in `stroke_indices`.
        assert_eq!(
            hover_overlay_indices(
                &state,
                ViewportHoverHighlight::SketchFace(FaceId::ConstructionPlane(pkey(0)))
            ),
            30,
            "construction-plane hover should add a biased fill quad and its border"
        );
    }

    /// #977: hovering an Elements-pane row lights what that row *is* — and for the things that
    /// aren't in the 3D view at all (a history operation, a component, a joint), what it made.
    /// Guarded by totality, like the crowd's kinds: a `SceneElement` that draws nothing fails
    /// here rather than shipping as "that row doesn't highlight".
    #[test]
    fn every_pane_row_lights_up_when_hovered() {
        use crate::model::{ExtrudeFace, JointKind, JointRef};
        let mut state = state_with_one_body();
        // A second body, so the ops below have two things to work on.
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            20.0,
            0.0,
            30.0,
            5.0,
            [false; 4],
        );
        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect.to_vec())],
            distance: 7.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,

        });
        assert_eq!(state.doc.bodies.len(), 2);
        // A component holding the first body, and a joint between the two.
        state.doc.components.insert(crate::model::Component {
            name: None,
            parent: None,
            length_unit: None,
            angle_unit: None,
        });
        state
            .doc
            .component_members
            .push((crate::model::ComponentMember::Body(bkey(0)), ckey(0)));
        state.doc.joints.insert(crate::model::Joint {
            members: vec![JointRef::Body(bkey(0)), JointRef::Body(bkey(1))],
            base: 0,
            kind: JointKind::Rigid,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: crate::model::JointLimits::default(),
            name: None,
            frame: Default::default(),
        });

        // A tracing image on the XY plane — its quad's outline is what its row lights.
        let image = state.doc.tracing_images.insert(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: pkey(0),
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 40.0,
            height_mm: 30.0,
            name: None,
            calibration: None,
        });

        let drawn = |element: SceneElement| {
            hover_overlay_indices(&state, ViewportHoverHighlight::Element(element))
        };
        for element in [
            SceneElement::Extrusion(xkey(0)),
            SceneElement::Body(bkey(0)),
            SceneElement::Component(ckey(0)),
            SceneElement::Joint(jkey(0)),
            SceneElement::Image(image),
            SceneElement::Line(lkey(0)),
            SceneElement::Sketch(sketch),
            SceneElement::ConstructionPlane(pkey(0)),
        ] {
            // `Body` and ops that only light *produced* bodies recolour in the main pass
            // rather than as an overlay (#455/#977/#1150), so they add nothing here.
            if matches!(
                element,
                SceneElement::Body(_)
                    | SceneElement::Component(_)
                    | SceneElement::Joint(_)
                    | SceneElement::BooleanOp(_)
                    | SceneElement::MoveOp(_)
                    | SceneElement::MirrorOp(_)
                    | SceneElement::RepeatOp(_)
                    | SceneElement::SliceOp(_)
                    | SceneElement::EdgeTreatmentOp(_)
                    | SceneElement::Revolution(_)
                    | SceneElement::Shape(_)
                    | SceneElement::SweepOp(_)
            ) {
                continue;
            }
            assert!(
                drawn(element.clone()) > 0,
                "hovering {element:?} drew nothing — every pane row must light something"
            );
        }
        // Main-pass recolour path for produced bodies (#1150): a Component holding a body
        // must wash that body purple when its row is hovered.
        let palette = ViewportPalette::default();
        let build = |hover: Option<ViewportHoverHighlight>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &state.cam,
                viewport: test_viewport(),
                palette,
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let derived_tinted = |scene: &ViewportScene| {
            scene
                .vertices
                .iter()
                .filter(|v| {
                    let [r, g, b, a] = v.color;
                    a > 0.9 && b > r && r > g && b > 0.5
                })
                .count()
        };
        let base = build(None);
        let component_hover = build(Some(ViewportHoverHighlight::Element(
            SceneElement::Component(ckey(0)),
        )));
        let joint_hover = build(Some(ViewportHoverHighlight::Element(SceneElement::Joint(
            jkey(0),
        ))));
        assert!(
            derived_tinted(&component_hover) > derived_tinted(&base),
            "hovering a Component must recolor the bodies it holds"
        );
        assert!(
            derived_tinted(&joint_hover) > derived_tinted(&base),
            "hovering a Joint must recolor the bodies it joins"
        );
    }

    /// #974, the general form: **every** kind the crowd can offer must light up when its
    /// Exploder loupe is hovered. The plane's failure was one empty match arm among many that
    /// looked alike, so the guard is totality rather than a case per kind — a new
    /// `PickTargetKind` with nothing behind it fails here rather than shipping as "that one
    /// kind doesn't highlight".
    ///
    /// Two kinds draw *outside* this pass and are named explicitly, so adding a third silent
    /// one is a deliberate act: a constraint's badge glows in the 2D annotation overlay
    /// (#568), and a whole body recolours in the main pass (#902).
    #[test]
    fn every_crowd_kind_lights_up_when_hovered() {
        use crate::construction::GlobalAxis;
        let state = state_with_one_body();
        let solid = crate::extrude::body_solid_mesh(&state.doc, bkey(0)).expect("body mesh");
        let tri = solid.triangles.first().copied().expect("a triangle");
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        let line = state.doc.lines.keys().next().expect("a line");

        let drawn = |kind: PickTargetKind| {
            hover_overlay_indices(&state, ViewportHoverHighlight::PickTarget(kind))
        };
        for kind in [
            PickTargetKind::Line(line),
            PickTargetKind::Point(crate::model::ConstraintPoint::LineEndpoint {
                line,
                end: crate::model::LineEnd::Start,
            }),
            PickTargetKind::BodyEdge {
                body: bkey(0),
                a: tri[0],
                b: tri[1],
            },
            PickTargetKind::BodyFace {
                body: bkey(0),
                triangles: vec![tri],
                normal,
            },
            PickTargetKind::BodyVertex {
                body: bkey(0),
                position: tri[0],
            },
            PickTargetKind::GlobalAxis(GlobalAxis::X),
            PickTargetKind::ConstructionPlane(pkey(0)),
            PickTargetKind::SketchFace(FaceId::ConstructionPlane(pkey(0))),
            PickTargetKind::Ground(Vec3::ZERO),
        ] {
            assert!(
                drawn(kind.clone()) > 0,
                "hovering {kind:?} drew nothing — every kind the crowd offers must light up"
            );
        }
        // The two that draw elsewhere in the frame, by design.
        assert_eq!(drawn(PickTargetKind::Constraint(nkey(0))), 0);
        assert_eq!(drawn(PickTargetKind::Body(bkey(0))), 0);
    }

    /// #974: a datum plane reaches the crowd **twice** — as itself and as the analytic face
    /// over the same surface — and `collect_pick_candidates` keeps whichever is nearer. So an
    /// Exploder loupe could hand the renderer either one, and only one of them drew: the
    /// plane's fill was written out at each call site rather than in the face helper, and the
    /// `FaceId::ConstructionPlane` arm of that helper was empty. Every representation of a
    /// plane must light up, and identically.
    #[test]
    fn every_representation_of_a_plane_hovers() {
        let state = AppState::default();
        let as_plane = hover_overlay_indices(
            &state,
            ViewportHoverHighlight::PickTarget(PickTargetKind::ConstructionPlane(pkey(0))),
        );
        let as_face = hover_overlay_indices(
            &state,
            ViewportHoverHighlight::PickTarget(PickTargetKind::SketchFace(
                FaceId::ConstructionPlane(pkey(0)),
            )),
        );
        let as_element = hover_overlay_indices(
            &state,
            ViewportHoverHighlight::Element(SceneElement::ConstructionPlane(pkey(0))),
        );
        let as_face_element = hover_overlay_indices(
            &state,
            ViewportHoverHighlight::Element(SceneElement::SketchFace(
                FaceId::ConstructionPlane(pkey(0)),
            )),
        );
        assert!(as_plane > 0, "a plane pick target must draw something");
        assert_eq!(as_face, as_plane, "so must the analytic face over it, the same way");
        assert!(as_element > 0, "and the plane as an element");
        assert_eq!(as_face_element, as_plane, "and that element's analytic form");
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        let hovered = build(Some(ViewportHoverHighlight::PickTarget(
            crate::construction::PickTargetKind::BodyEdge {
                body: bkey(0),
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

        // #807: a whole analytic edge (a hole's rim reaches the tools as one edge but many
        // chords) highlights as all of its segments, not just the one under the cursor.
        let one = build(Some(ViewportHoverHighlight::Curve {
            segments: vec![(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0))],
        }));
        let three = build(Some(ViewportHoverHighlight::Curve {
            segments: vec![
                (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)),
                (Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.0)),
                (Vec3::new(10.0, 10.0, 0.0), Vec3::ZERO),
            ],
        }));
        assert!(one.wireframe_indices.len() > base.wireframe_indices.len());
        assert!(
            three.wireframe_indices.len() > one.wireframe_indices.len(),
            "every segment of the edge is highlighted"
        );

        // #1359: a hovered point is a camera-facing disc sitting on a face. If it lands in
        // the depth-tested base mesh, the body wins the near half of the disc and the
        // highlight reads as half-buried. Same Wireframe path as the edge above.
        let point = build(Some(ViewportHoverHighlight::PickTarget(
            crate::construction::PickTargetKind::BodyVertex {
                body: bkey(0),
                position: Vec3::new(25.0, 12.5, 25.0),
            },
        )));
        assert!(
            point.wireframe_indices.len() > base.wireframe_indices.len(),
            "hovered point must land in the depth-disabled (wireframe) layer"
        );
        assert_eq!(
            point.indices.len(),
            base.indices.len(),
            "hovered point must not draw in the depth-tested base mesh"
        );
        assert_eq!(
            point.overlay_indices.len(),
            base.overlay_indices.len(),
            "hovered point must not draw in the depth-tested overlay layer"
        );
    }

    /// #1139: hovering a body-coplanar face (extrude cap/side, or a sketch on one) used to
    /// paint its translucent fill in the depth-tested overlay layer. On a solid that is
    /// coplanar with the face, 0.09 mm of world lift + the overlay pipeline bias is not
    /// enough at typical viewing angles — the body and the gold hover fill alternate who
    /// wins the depth test, and the face reads as a mottled checkerboard. Body-face
    /// selection already uses the depth-disabled wireframe layer (#555); body-coplanar
    /// face *hover* must do the same.
    #[test]
    fn body_face_hover_fill_draws_depth_test_disabled() {
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
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        // Top cap of the one extruded body — coplanar with the solid, same surface the
        // Extrude tool hovers when preparing to push a face.
        let top = FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: crate::model::ExtrudeFace::Polygon(vec![
                lkey(0),
                lkey(1),
                lkey(2),
                lkey(3),
            ]),
            top: true,
        };
        let hovered = build(Some(ViewportHoverHighlight::SketchFace(top)));
        let wire_growth = hovered.wireframe_indices.len() - base.wireframe_indices.len();
        let overlay_growth = hovered
            .overlay_indices
            .len()
            .saturating_sub(base.overlay_indices.len());
        assert!(
            wire_growth >= 6,
            "body-face hover fill must land in the depth-disabled (wireframe) layer, got wire +{wire_growth}"
        );
        assert_eq!(
            overlay_growth, 0,
            "body-face hover must not draw in the depth-tested overlay (that is what z-fought the body), got overlay +{overlay_growth}"
        );
    }

    /// #1140 / #1174: a circle on a body face (e.g. Extrude's profile hover/selection on a side
    /// cap) sits coplanar with the solid. Hover/selection rings stay on the depth-disabled
    /// wireframe layer so they don't z-fight the body. The committed stroke uses depth-tested
    /// screen-space strokes so the solid occludes the ring when the face is not visible (#1174).
    fn state_with_circle_on_body_face() -> (AppState, crate::model::CircleKey) {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = state_with_one_body();
        // Sketch on the extruded body's top cap, then a circle on that face.
        state.apply(Action::BeginSketch {
            face: FaceId::ExtrudeCap {
                extrusion: xkey(0),
                profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                top: true,
            },
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        let ci = state
            .doc
            .circles
            .insert(crate::model::Circle::from_local_center_radius(
                sketch, 5.0, 2.5, 2.0, 0.0,
            ));
        // Close the sketch so the circle is ordinary body-face decoration, not live edit.
        state.sketch_session = None;
        (state, ci)
    }

    fn build_circle_scene(
        state: &AppState,
        hover: Option<ViewportHoverHighlight>,
        selection: &SceneSelection,
    ) -> ViewportScene {
        let cam = state.cam.clone();
        let viewport = test_viewport();
        ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: hover,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: crate::construction::PICK_HOVER_RGBA,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
    }

    #[test]
    fn body_coplanar_circle_face_hover_draws_depth_test_disabled() {
        let (state, ci) = state_with_circle_on_body_face();
        let base = build_circle_scene(&state, None, &state.scene_selection);
        let hovered = build_circle_scene(
            &state,
            Some(ViewportHoverHighlight::SketchFace(FaceId::Circle(ci))),
            &state.scene_selection,
        );
        let wire_growth = hovered.wireframe_indices.len() - base.wireframe_indices.len();
        let overlay_growth = hovered
            .overlay_indices
            .len()
            .saturating_sub(base.overlay_indices.len());
        assert!(
            wire_growth >= 6,
            "circle-face hover fill+border must land in the depth-disabled layer, got wire +{wire_growth}"
        );
        assert_eq!(
            overlay_growth, 0,
            "circle-face hover must not grow the depth-tested overlay, got overlay +{overlay_growth}"
        );
    }

    #[test]
    fn body_coplanar_circle_selection_draws_depth_test_disabled() {
        let (state, ci) = state_with_circle_on_body_face();
        let base = build_circle_scene(&state, None, &state.scene_selection);
        let mut selection = state.scene_selection.clone();
        crate::selection::click_scene_selection(&mut selection, SceneElement::Circle(ci), true);
        let selected = build_circle_scene(&state, None, &selection);
        let wire_growth = selected.wireframe_indices.len() - base.wireframe_indices.len();
        let overlay_growth = selected
            .overlay_indices
            .len()
            .saturating_sub(base.overlay_indices.len());
        assert!(
            wire_growth >= 6,
            "selected body-face circle must land in the depth-disabled layer, got wire +{wire_growth}"
        );
        assert_eq!(
            overlay_growth, 0,
            "selected body-face circle must not grow the depth-tested overlay (z-fights the body), got overlay +{overlay_growth}"
        );
    }

    /// #1174: a committed circle on a body face must depth-test like body-face lines (#1157),
    /// so the solid occludes the ring when that face is behind the camera-facing surface.
    /// The always-on wireframe path from #1140 made the circle show through the cube.
    /// Selection/hover rings stay depth-disabled (see tests above) — only the ordinary stroke.
    #[test]
    fn body_coplanar_committed_circle_stroke_is_depth_tested() {
        let (state, _ci) = state_with_circle_on_body_face();
        let mut plane_state = state_with_one_body();
        {
            use crate::actions::Action;
            plane_state.apply(Action::BeginSketch {
                face: FaceId::ConstructionPlane(pkey(0)),
                viewport: None,
            });
            let sketch = plane_state.sketch_session.unwrap().sketch;
            plane_state
                .doc
                .circles
                .insert(crate::model::Circle::from_local_center_radius(
                    sketch, 5.0, 2.5, 2.0, 0.0,
                ));
            plane_state.sketch_session = None;
        }
        let on_body = build_circle_scene(&state, None, &state.scene_selection);
        let on_plane = build_circle_scene(&plane_state, None, &plane_state.scene_selection);
        assert!(
            !on_body.stroke_indices.is_empty(),
            "body-face circle stroke must use depth-tested screen-space strokes (#1174)"
        );
        assert!(
            on_body.wireframe_indices.is_empty(),
            "committed body-face circle must not use the always-on wireframe layer (shows through solids)"
        );
        assert!(
            !on_plane.stroke_indices.is_empty(),
            "a plane-sketched circle should use screen-space strokes"
        );
        // Same path as a plane circle — both depth-tested; body-coplanar is not special here.
        assert_eq!(
            on_body.stroke_indices.len(),
            on_plane.stroke_indices.len(),
            "body-face and plane-face committed circles should emit the same stroke topology"
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,

            plane_gizmo: Some(gizmo),
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            with_gizmo.gizmo_indices.len() > base.gizmo_indices.len(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            with_gizmo.gizmo_indices.len() > base.gizmo_indices.len(),
            "extrude gizmo should add triangles to the viewport scene"
        );
    }

    /// #1123: the ground lattice carries a soft distance fade so orbiting never snaps
    /// distant grid sections on/off at a hard footprint edge.
    #[test]
    fn ground_grid_sets_a_distance_fade_ramp() {
        use crate::actions::AppState;
        let state = AppState::default();
        let scene = build_scene_for_doc(&state);
        let grid = scene.grid.expect("default scene has a ground grid");
        assert!(
            grid.fade_end_mm > grid.fade_start_mm,
            "fade end must be past fade start, got {}..{}",
            grid.fade_start_mm,
            grid.fade_end_mm
        );
        assert!(
            grid.fade_start_mm > 0.0 && grid.fade_end_mm.is_finite(),
            "fade distances must be positive finite"
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        // Both the grid (#1073) and the origin axes (#1072) are shader-drawn now, so they
        // live in their own buffers rather than the scene's index layers.
        let grid = scene.grid.expect("the ground grid");
        assert_eq!(scene.axis_indices.len(), 3 * 6, "one quad per origin axis");
        assert!(grid.coarse_step > grid.fine_step, "{grid:?}");
        // Widths are pixels, not world units — that is what keeps a line one pixel across
        // whether it is under the camera or at the horizon.
        assert!(grid.fine_width_px > 0.0 && grid.coarse_width_px > 0.0);
        // The footprint is a real quad, not a degenerate one.
        let (a, b, c) = (grid.corners[0], grid.corners[1], grid.corners[3]);
        assert!((b - a).length() > 1.0 && (c - a).length() > 1.0, "{grid:?}");
        assert_eq!(scene.clear_color[0], color32_to_gpu(Color32::from_gray(28))[0]);
    }

    fn count_opaque_stroke_vertices(scene: &ViewportScene, stroke: Color32) -> usize {
        let gpu = color32_to_gpu(stroke);
        let matches = |v: &GpuVertex| {
            v.color[3] > 0.99
                && (v.color[0] - gpu[0]).abs() < 0.02
                && (v.color[1] - gpu[1]).abs() < 0.02
                && (v.color[2] - gpu[2]).abs() < 0.02
        };
        // Screen-space strokes (#1157) live in `stroke_vertices`; legacy world-ribbon
        // strokes (wireframe/gizmo) still sit in `vertices`.
        scene.vertices.iter().filter(|v| matches(v)).count()
            + scene.stroke_vertices.iter().filter(|v| matches(v)).count()
    }

    /// #1072: an axis quad's corners carry both endpoints and a signed pixel half-width, so
    /// the vertex shader can widen it on screen. Check the packing, since nothing else can.
    #[test]
    fn origin_axes_carry_both_endpoints_and_a_pixel_half_width() {
        let scene = build_scene_for_doc(&AppState::default());
        assert_eq!(scene.axis_vertices.len(), 3 * 4, "one quad per axis");
        for quad in scene.axis_vertices.chunks_exact(4) {
            // Corners come in the cyclic order a+, a-, b-, b+: the two ends, each twice.
            assert_eq!(quad[0].position, quad[1].position);
            assert_eq!(quad[2].position, quad[3].position);
            assert_ne!(quad[0].position, quad[2].position);
            for v in quad {
                // `normal.xyz` is the *other* end, so each corner can find the screen
                // direction on its own.
                let other = [v.normal[0], v.normal[1], v.normal[2]];
                assert_ne!(other, v.position, "a corner must name the far end, not itself");
                assert_eq!(v.normal[3].abs(), ORIGIN_AXIS_WIDTH_PX / 2.0, "half-width, in pixels");
            }
            // Each end straddles the line: one corner to each side.
            assert!(quad[0].normal[3] > 0.0 && quad[1].normal[3] < 0.0);
            assert!(quad[2].normal[3] > 0.0 && quad[3].normal[3] < 0.0);
        }
    }

    /// #1124: selecting a world origin axis draws a stroke 2× the hover thickness into the
    /// depth-test-disabled wireframe layer so it bleeds through bodies.
    #[test]
    fn selected_origin_axis_is_thicker_and_bleeds_through() {
        use crate::construction::GlobalAxis;
        use crate::hierarchy::SceneElement;
        let mut state = AppState::default();
        crate::selection::click_scene_selection(
            &mut state.scene_selection,
            SceneElement::GlobalAxis(GlobalAxis::Z),
            false,
        );
        let base = build_scene_for_doc(&AppState::default());
        let selected = build_scene_for_doc(&state);
        assert!(
            selected.wireframe_indices.len() > base.wireframe_indices.len(),
            "selected origin axis must add always-on-top wireframe geometry, base={} selected={}",
            base.wireframe_indices.len(),
            selected.wireframe_indices.len()
        );
        assert_eq!(
            ORIGIN_AXIS_SELECTED_WIDTH_PX,
            ORIGIN_AXIS_HOVER_WIDTH_PX * 2.0,
            "selected must be exactly 2× hover"
        );
    }

    /// #1073: the ground modes each ask for the right thing — a shader grid, a solid fill
    /// with no lattice, or nothing at all.
    #[test]
    fn ground_display_modes_pick_grid_solid_or_nothing() {
        use crate::camera::GroundDisplay;
        let mut state = AppState::default();

        state.cam.set_ground_display(GroundDisplay::Grid);
        assert!(build_scene_for_doc(&state).grid.is_some(), "Grid draws the lattice");

        state.cam.set_ground_display(GroundDisplay::Solid);
        let solid = build_scene_for_doc(&state);
        assert!(solid.grid.is_none(), "Solid is a plain fill, with no lines on it");
        assert!(
            solid.solid_ground.is_some(),
            "Solid fills the footprint on the dedicated ground layer"
        );

        state.cam.set_ground_display(GroundDisplay::None);
        let hidden = build_scene_for_doc(&state);
        assert!(hidden.grid.is_none(), "None hides the ground entirely (#579)");
        assert!(
            hidden.solid_ground.is_none(),
            "None hides solid ground entirely (#579)"
        );
    }

    /// #1295: solid ground is a dark grey-blue, not near-black (scaled grid grey).
    #[test]
    fn solid_ground_is_dark_grey_blue() {
        use crate::camera::GroundDisplay;
        let mut state = AppState::default();
        state.cam.set_ground_display(GroundDisplay::Solid);
        let scene = build_scene_for_doc(&state);
        let expected = SOLID_GROUND_COLOR;
        let solid = scene
            .solid_ground
            .expect("solid ground should be present from above");
        let c = solid.color;
        assert!(
            (c[0] - expected.r() as f32 / 255.0).abs() < 1e-3
                && (c[1] - expected.g() as f32 / 255.0).abs() < 1e-3
                && (c[2] - expected.b() as f32 / 255.0).abs() < 1e-3,
            "solid ground should use SOLID_GROUND_COLOR {expected:?}; got {c:?}"
        );
        // Blue channel dominates red/green slightly (grey-blue, not pure grey or black).
        assert!(
            expected.b() > expected.r() && expected.b() > expected.g(),
            "ground should be blue-tinted, got {expected:?}"
        );
        assert!(
            expected.r() > 20 && expected.r() < 80,
            "ground should be dark but not black, got {expected:?}"
        );
    }

    /// #1295/#1301: solid ground sits at z = 0 with no world-space depth bias (bias
    /// mis-places coplanar geometry, #1088/#1121). It lives on a dedicated no-depth-write
    /// layer so coplanar construction planes composite without z-fighting; body faces on
    /// the ground still re-draw after plane fills (#1215 pattern).
    #[test]
    fn solid_ground_is_unbiased_and_body_base_is_repainted() {
        use crate::camera::GroundDisplay;
        use crate::hierarchy::SceneElement;
        let mut state = state_with_one_body();
        // Hide construction planes so overpaint only comes from solid-ground coplanarity.
        for (pi, _) in state.doc.construction_planes.iter() {
            state
                .element_visibility
                .set_visible(SceneElement::ConstructionPlane(pi), false);
        }
        state.cam.set_ground_display(GroundDisplay::Solid);
        let scene = build_scene_for_doc(&state);

        let solid = scene
            .solid_ground
            .expect("solid ground should use the dedicated no-depth-write layer");
        assert!(
            solid.corners.iter().all(|c| c.z.abs() < 1e-5),
            "solid ground must not use geometric depth bias; corners={:?}",
            solid.corners
        );
        // Must not also land in the opaque base pass (that writes depth and z-fights
        // coplanar construction planes / body bottoms — #1301).
        let ground_in_base = scene.vertices.iter().any(|v| {
            let c = v.color;
            (c[0] - SOLID_GROUND_COLOR.r() as f32 / 255.0).abs() < 1e-3
                && (c[1] - SOLID_GROUND_COLOR.g() as f32 / 255.0).abs() < 1e-3
                && (c[2] - SOLID_GROUND_COLOR.b() as f32 / 255.0).abs() < 1e-3
                && v.position[2].abs() < 1e-5
        });
        assert!(
            !ground_in_base,
            "solid ground must not write depth via the opaque base mesh"
        );

        // Body base on z = 0 is re-indexed into body_over_plane so it wins the depth tie
        // without bias, even when construction planes are hidden.
        assert!(
            scene.body_over_plane_indices.len() >= 6,
            "body faces on solid ground must re-draw after the ground fill; got {}",
            scene.body_over_plane_indices.len()
        );
        let on_ground = scene
            .body_over_plane_indices
            .iter()
            .map(|&i| scene.vertices[i as usize].position[2].abs())
            .all(|z| z < 1e-2);
        assert!(
            on_ground,
            "body_over_plane vertices for a ground-resting body should sit on z = 0"
        );
    }

    /// #1300/#1370: looking up from under the ground must not paint the *solid ground fill*,
    /// but the *grid* lattice still draws for orientation — a grid reads the same viewed
    /// from under the plane, so only the fill is suppressed (#1370).
    #[test]
    fn ground_is_hidden_when_camera_is_below() {
        use crate::camera::GroundDisplay;
        let mut state = AppState::default();
        // South-pole-ish view: eye is under z = 0 looking up.
        state.cam.pitch = -1.4;
        assert!(
            state.cam.eye().z < 0.0,
            "test setup: camera must be below the ground plane"
        );

        state.cam.set_ground_display(GroundDisplay::Solid);
        let solid = build_scene_for_doc(&state);
        assert!(
            solid.solid_ground.is_none(),
            "solid ground must not show from underneath"
        );
        assert!(
            solid.grid.is_none(),
            "solid mode draws no grid (there's no lattice on a solid fill)"
        );
        assert!(
            !solid_ground_color_in_base(&solid),
            "solid ground must not appear in the base mesh from underneath"
        );

        state.cam.set_ground_display(GroundDisplay::Grid);
        let grid = build_scene_for_doc(&state);
        assert!(
            grid.grid.is_some(),
            "ground grid must still show from underneath (#1370)"
        );
        assert!(
            grid.solid_ground.is_none(),
            "no solid fill in grid mode from underneath"
        );

        // Axes still orient the view.
        assert!(
            !solid.axis_indices.is_empty() && !grid.axis_indices.is_empty(),
            "world axes still draw from below"
        );
    }

    /// #1300: from above, solid ground and the grid still show when their modes are on.
    #[test]
    fn ground_is_shown_when_camera_is_above() {
        use crate::camera::GroundDisplay;
        let mut state = AppState::default();
        assert!(
            state.cam.eye().z > 0.0,
            "default camera is above the ground"
        );

        state.cam.set_ground_display(GroundDisplay::Solid);
        assert!(
            build_scene_for_doc(&state).solid_ground.is_some(),
            "solid ground shows from above"
        );

        state.cam.set_ground_display(GroundDisplay::Grid);
        assert!(
            build_scene_for_doc(&state).grid.is_some(),
            "ground grid shows from above"
        );
    }

    fn solid_ground_color_in_base(scene: &ViewportScene) -> bool {
        scene.vertices.iter().any(|v| {
            let c = v.color;
            (c[0] - SOLID_GROUND_COLOR.r() as f32 / 255.0).abs() < 1e-3
                && (c[1] - SOLID_GROUND_COLOR.g() as f32 / 255.0).abs() < 1e-3
                && (c[2] - SOLID_GROUND_COLOR.b() as f32 / 255.0).abs() < 1e-3
        })
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        })
    }

    fn commit_test_rectangle(state: &mut AppState) {
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
        };
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        state.doc.constraints.insert(coincident(point(lkey(0), LineEnd::End), point(lkey(1), LineEnd::Start)));
        state.doc.constraints.insert(coincident(point(lkey(1), LineEnd::End), point(lkey(2), LineEnd::Start)));
        state.doc.constraints.insert(coincident(point(lkey(2), LineEnd::End), point(lkey(0), LineEnd::Start)));
    }

    /// Rectangle and circle both at index 0 on the ground plane, overlapping (#3).
    fn commit_overlapping_rect_and_circle(state: &mut AppState) {
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
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
            face: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
        });
        state.apply(Action::SetExtrudeDistance { distance: 7.0 });
        state.apply(Action::CommitExtrusion);
        assert_eq!(state.doc.bodies.len(), 1);

        let cam = state.cam.clone();
        let build = |editing: Option<crate::model::ExtrusionKey>| {
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: editing,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };

        let with_body = build(None);
        let editing = build(Some(xkey(0)));
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
        let sketch = state.doc.lines[lkey(0)].sketch;
        let before = build_scene_for_doc(&state).vertices.len();

        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)])],
            distance: 8.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,

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
    fn extruded_top_cap_on_slanted_target_plane_is_repainted_after_plane_fills() {
        // #29/#1215: the top cap sits on the target plane. Rather than a world-space depth
        // bias (which mis-places planes, #1088), those triangles are re-indexed into
        // `body_over_plane_indices` and drawn again after the translucent plane wash.
        let mut state = AppState::default();
        retain_ground_plane_only(&mut state.doc);
        commit_test_rectangle(&mut state);
        let sketch = state.doc.lines[lkey(0)].sketch;

        let plane_origin = Vec3::new(0.0, 0.0, 12.0);
        let plane_normal = Vec3::new(0.0, 0.4, 1.0).normalize();
        let mut slanted = crate::face::default_xy_plane();
        slanted.origin = plane_origin;
        slanted.normal = plane_normal;
        state.doc.construction_planes.insert(slanted);

        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)])],
            distance: 6.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,

        });
        state.doc.extrusions[xkey(0)].target = Some(crate::model::ExtrudeTarget::Plane(pkey(1)));

        let raw = crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[xkey(0)]).unwrap();
        let cap_vertex = *raw
            .triangles
            .iter()
            .flat_map(|t| t.iter())
            .find(|p| ((**p - plane_origin).dot(plane_normal)).abs() < 1e-3)
            .expect("expected at least one top-cap vertex on the target plane");

        let scene = build_scene_for_doc(&state);
        assert!(
            !scene.body_over_plane_indices.is_empty(),
            "top-cap triangles on the target plane must be re-drawn after plane fills"
        );
        // Positions stay unbiased — no geometric lift toward the camera.
        assert!(
            scene
                .vertices
                .iter()
                .any(|v| (Vec3::from(v.position) - cap_vertex).length() < 1e-4),
            "cap vertices stay at their raw (export) positions"
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

    /// #1215: a body face that rests on a construction plane is re-drawn after the plane
    /// fill so coplanar solid/plane pairs don't z-fight — without world-space, pipeline, or
    /// frag-depth bias.
    #[test]
    fn body_face_coplanar_with_construction_plane_is_repainted_after_plane_fills() {
        let state = state_with_one_body();
        let scene = build_scene_for_doc(&state);
        // Extruded box on XY: bottom at z=0, and sides on x=0 / y=0 hit YZ / XZ too.
        assert!(
            scene.body_over_plane_indices.len() >= 6,
            "at least the base face (two tris) should re-draw over the plane fill; got {}",
            scene.body_over_plane_indices.len()
        );
        assert!(
            !scene.plane_fill_indices.is_empty(),
            "construction plane fills still draw in the translucent layer"
        );
        // Every overpaint index must point at a real solid vertex.
        let nverts = scene.vertices.len() as u32;
        assert!(
            scene
                .body_over_plane_indices
                .iter()
                .all(|&i| i < nverts),
            "body_over_plane indices must reference solid vertices"
        );
    }

    /// #1215: hiding every construction plane means nothing to z-fight with — the overpaint
    /// layer stays empty.
    #[test]
    fn body_over_plane_is_empty_when_construction_planes_are_hidden() {
        use crate::hierarchy::SceneElement;
        let mut state = state_with_one_body();
        for (pi, _) in state.doc.construction_planes.iter() {
            state
                .element_visibility
                .set_visible(SceneElement::ConstructionPlane(pi), false);
        }
        let scene = build_scene_for_doc(&state);
        assert!(
            scene.body_over_plane_indices.is_empty(),
            "no plane fills → no coplanar overpaint"
        );
    }

    /// #1215 (issue report): a cuboid whose face sits on the YZ datum must re-paint that
    /// face after the plane wash — the left-cube mottling in the bug screenshot.
    #[test]
    fn cuboid_face_on_yz_plane_is_in_body_over_plane_layer() {
        use crate::model::{Primitive, PrimitiveKind};
        let mut state = AppState::default();
        // Cuboid resting on YZ (x = 0): origin on the plane, extruded along +X.
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.origin = [0.0, 50.0, 50.0];
        shape.normal = [1.0, 0.0, 0.0];
        shape.u_axis = [0.0, 1.0, 0.0];
        shape.width = "40".into();
        shape.depth = "25".into();
        shape.height = "80".into();
        state.apply(crate::actions::Action::CreateShape { shape });
        assert_eq!(state.doc.bodies.len(), 1);

        let scene = build_scene_for_doc(&state);
        assert!(
            scene.body_over_plane_indices.len() >= 6,
            "YZ-coplanar face must re-draw after plane fills; got {}",
            scene.body_over_plane_indices.len()
        );
        // Those overpaint vertices must actually lie on x ≈ 0 (the YZ plane).
        let on_yz = scene
            .body_over_plane_indices
            .iter()
            .map(|&i| scene.vertices[i as usize].position[0].abs())
            .all(|x| x < 1e-2);
        assert!(
            on_yz,
            "body_over_plane vertices for a YZ-resting cuboid should sit on x = 0"
        );
    }

    #[test]
    fn extrude_preview_to_slanted_target_plane_shows_slanted_top() {
        // The in-progress (uncommitted) ghost preview should show the actual slanted shape
        // once the gizmo has snapped to a slanted target plane (#63).
        let mut state = AppState::default();
        retain_ground_plane_only(&mut state.doc);
        commit_test_rectangle(&mut state);
        let sketch = state.doc.lines[lkey(0)].sketch;

        let plane_origin = Vec3::new(0.0, 0.0, 12.0);
        let plane_normal = Vec3::new(0.0, 0.4, 1.0).normalize();
        let mut slanted = crate::face::default_xy_plane();
        slanted.origin = plane_origin;
        slanted.normal = plane_normal;
        state.doc.construction_planes.insert(slanted);

        let preview = crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)])],
            distance: 6.0,
            target: Some(crate::model::ExtrudeTarget::Plane(pkey(1))),
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: Some(preview.clone()),
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            editing_extrusion: None,
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
        let sketch = state.doc.lines[lkey(0)].sketch;
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
            crate::face::sketch_geometry_frame(&state.doc, state.doc.lines[lkey(0)].sketch).unwrap();
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: Some(ViewportHoverHighlight::SketchFace(FaceId::Polygon(vec![
                lkey(0),
                lkey(1),
                lkey(2),
                lkey(3),
            ]))),
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            .set_visible(SceneElement::ConstructionPlane(pkey(0)), false);

        let with_plane = build_scene_for_doc(&AppState::default());
        let without_plane = build_scene_for_doc(&hidden);
        let plane_indices =
            with_plane.plane_fill_indices.len() - without_plane.plane_fill_indices.len();
        assert_eq!(
            plane_indices, 6,
            "each construction plane should add only two fill triangles"
        );
    }

    /// #1087: a datum plane shows through the body it passes through — it is a reference
    /// that is part of the scene, not a surface hidden by the geometry it bisects. Its fill
    /// lives in the translucent `plane_fill` layer, drawn after the opaque scene, so the
    /// plane reads over the body everywhere it crosses it.
    #[test]
    fn a_datum_plane_is_drawn_with_the_translucent_fills() {
        // A body, and the default datum planes crossing the space around it.
        let scene = build_scene_for_doc(&state_with_one_body());

        // The plane's fill is in the plane-fill layer, which the renderer draws **after**
        // the opaque scene — that is what lets it show through the body it bisects.
        assert!(
            scene.plane_fill_indices.len() >= 6,
            "the datum planes fill in the translucent layer"
        );
        assert!(!scene.indices.is_empty(), "the body is opaque geometry");
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

    /// #1149/#1153/#1167: outside sketch mode, lines on a light body face use the dark
    /// on-body stroke so they stay readable on the undimmed face.
    #[test]
    fn solid_line_on_body_face_uses_high_contrast_stroke() {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = state_with_one_body();
        state.apply(Action::BeginSketch {
            face: FaceId::ExtrudeCap {
                extrusion: xkey(0),
                profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                top: true,
            },
            viewport: None,
        });
        state.apply(Action::CreateLineSegment {
            x0: 1.0,
            y0: 1.0,
            x1: 8.0,
            y1: 3.0,
            bezier: None,
            dimension: None,
        });
        state.sketch_session = None;

        let palette = ViewportPalette::default();
        let scene = build_scene_for_doc(&state);
        let on_body = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body);
        let bright = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body_in_sketch);
        let plane_blue = count_opaque_stroke_vertices(&scene, palette.rect_line);
        assert!(
            on_body > 0,
            "outside sketch, a solid line on a light body face should use the dark on-body stroke"
        );
        assert_eq!(
            bright, 0,
            "outside sketch on a light face, the bright in-sketch stroke must not be used"
        );
        assert_ne!(
            palette.rect_line_on_body, palette.rect_line,
            "on-body stroke must differ from the plane sketch blue"
        );
        assert!(
            plane_blue < on_body || palette.rect_line != palette.rect_line_on_body,
            "body-face line should not be the plane-sketch blue"
        );
        let c = palette.rect_line_on_body;
        assert!(
            c.b() > c.g() && c.g() > c.r() && c.b() < 100 && c.r() > 30,
            "on-body stroke should be a solid dark blue-grey, got {:?}",
            c
        );
    }

    /// #1167: while a sketch is open, bodies are dimmed and the dark on-body stroke vanishes.
    /// Body-face lines must use the bright stroke so they stay editable.
    #[test]
    fn solid_line_on_body_face_is_bright_while_sketch_is_open() {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = state_with_one_body();
        state.apply(Action::BeginSketch {
            face: FaceId::ExtrudeCap {
                extrusion: xkey(0),
                profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                top: true,
            },
            viewport: None,
        });
        state.apply(Action::CreateLineSegment {
            x0: 1.0,
            y0: 1.0,
            x1: 8.0,
            y1: 3.0,
            bezier: None,
            dimension: None,
        });
        let session = state.sketch_session.expect("test needs an open sketch session");

        let palette = ViewportPalette::default();
        let cam = state.cam.clone();
        // `build_scene_for_doc` always clears the session (most helpers leave one open);
        // build with the real session so we exercise the in-sketch stroke path.
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport: test_viewport(),
            palette: ViewportPalette::default(),
            sketch_session: Some(session),
            selection: &state.scene_selection,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        let bright = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body_in_sketch);
        let dark = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body);
        assert!(
            bright > 0,
            "with a sketch open, body-face lines should use the bright in-sketch stroke"
        );
        assert_eq!(
            dark, 0,
            "with a sketch open, body-face lines must not use the dark out-of-sketch stroke"
        );
        assert!(
            palette.rect_line_on_body_in_sketch.r() > palette.rect_line_on_body.r(),
            "in-sketch stroke must be brighter than the out-of-sketch dark stroke"
        );
    }

    /// #1167: outside sketch mode, a dark body material gets the bright on-body stroke so
    /// the line still contrasts the face.
    #[test]
    fn solid_line_on_dark_body_face_uses_bright_stroke_outside_sketch() {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = state_with_one_body();
        // Paint the body a dark colour (below the 0.35 luminance threshold).
        let mat = state.doc.materials.insert(crate::model::Material {
            name: "Dark".into(),
            color: [30, 30, 35],
        });
        let body = state.doc.bodies.keys().next().expect("one body");
        state.doc.bodies.get_mut(body).expect("body").material = Some(mat);

        state.apply(Action::BeginSketch {
            face: FaceId::ExtrudeCap {
                extrusion: xkey(0),
                profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                top: true,
            },
            viewport: None,
        });
        state.apply(Action::CreateLineSegment {
            x0: 1.0,
            y0: 1.0,
            x1: 8.0,
            y1: 3.0,
            bezier: None,
            dimension: None,
        });
        state.sketch_session = None;

        let palette = ViewportPalette::default();
        let scene = build_scene_for_doc(&state);
        let bright = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body_in_sketch);
        let dark = count_opaque_stroke_vertices(&scene, palette.rect_line_on_body);
        assert!(
            bright > 0,
            "on a dark body face outside sketch, lines should use the bright contrasting stroke"
        );
        assert_eq!(
            dark, 0,
            "dark stroke would vanish on a dark face"
        );
    }

    /// #1157: body-face sketch strokes must not be camera-facing world ribbons (those read as
    /// freestanding 3D rectangles when the face is viewed at a grazing angle). Pack like the
    /// origin axes (#1072): corners carry the endpoints and a pixel half-width so `vs_axis`
    /// widens in screen space — the line paints on the face, not out of it.
    #[test]
    fn body_face_sketch_stroke_is_screen_space_not_world_ribbon() {
        use crate::actions::Action;
        use crate::model::ExtrudeFace;

        let mut state = state_with_one_body();
        state.apply(Action::BeginSketch {
            face: FaceId::ExtrudeCap {
                extrusion: xkey(0),
                profile: ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]),
                top: true,
            },
            viewport: None,
        });
        state.apply(Action::CreateLineSegment {
            x0: 1.0,
            y0: 1.0,
            x1: 8.0,
            y1: 3.0,
            bezier: None,
            dimension: None,
        });
        state.sketch_session = None;

        let palette = ViewportPalette::default();
        let scene = build_scene_for_doc(&state);
        let gpu = color32_to_gpu(palette.rect_line_on_body);
        let stroke_quads: Vec<_> = scene
            .stroke_vertices
            .chunks_exact(4)
            .filter(|quad| {
                quad.iter().all(|v| {
                    v.color[3] > 0.99
                        && (v.color[0] - gpu[0]).abs() < 0.02
                        && (v.color[1] - gpu[1]).abs() < 0.02
                        && (v.color[2] - gpu[2]).abs() < 0.02
                })
            })
            .collect();
        assert!(
            !stroke_quads.is_empty(),
            "body-face sketch stroke must land in screen-space stroke buffer (got {} stroke verts, {} scene verts)",
            scene.stroke_vertices.len(),
            scene.vertices.len()
        );
        // Each quad is a+, a-, b-, b+: two corners share an endpoint position; half-width is
        // in **pixels** (≤ a few px), not a world-space offset that leaves the face plane.
        for quad in &stroke_quads {
            assert_eq!(quad[0].position, quad[1].position, "a-end corners share endpoint");
            assert_eq!(quad[2].position, quad[3].position, "b-end corners share endpoint");
            assert_ne!(quad[0].position, quad[2].position, "segment has length");
            for v in *quad {
                let half = v.normal[3].abs();
                assert!(
                    (0.5..8.0).contains(&half),
                    "half-width must be a few pixels, not world mm; got {half}"
                );
                // other endpoint packed in normal.xyz (same packing as origin axes #1072)
                let other = [v.normal[0], v.normal[1], v.normal[2]];
                assert_ne!(other, v.position, "corner must name the far end");
            }
            assert!(quad[0].normal[3] > 0.0 && quad[1].normal[3] < 0.0);
            assert!(quad[2].normal[3] > 0.0 && quad[3].normal[3] < 0.0);
        }
        // Must not also emit a world-ribbon of the same colour into the ordinary mesh —
        // that is the freestanding 3D rectangle the report showed.
        let world_ribbon = scene
            .vertices
            .iter()
            .filter(|v| {
                v.color[3] > 0.99
                    && (v.color[0] - gpu[0]).abs() < 0.02
                    && (v.color[1] - gpu[1]).abs() < 0.02
                    && (v.color[2] - gpu[2]).abs() < 0.02
            })
            .count();
        assert_eq!(
            world_ribbon, 0,
            "body-face stroke must not also be a camera-facing world ribbon"
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        // Screen-space sketch edges (#1157): four rectangle sides × 4 verts each.
        assert!(scene.stroke_vertices.len() >= 16);
        // The three origin axes are 18 indices, but they moved to their own buffer when they
        // became shader-widened (#1072) — the sketch's own edges are in `stroke_indices` now.
        assert!(scene.stroke_indices.len() >= 24);
    }

    #[test]
    fn circle_uses_more_segments_than_old_cpu_path() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        // The circle's stroke is an overlay, and the grid stopped contributing indices when
        // it became a shader (#1073), so count every layer rather than just the base one.
        let total = scene.indices.len()
            + scene.sketch_fill_indices.len()
            + scene.plane_fill_indices.len()
            + scene.overlay_indices.len();
        assert!(total > CIRCLE_SEGMENTS, "{total}");
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
        assert!(shape_fill_depth_bias(0) > plane_fill_depth_bias(pkey(0)));
    }

    #[test]
    fn stroke_depth_bias_beats_shape_fill_bias() {
        assert!(STROKE_DEPTH_BIAS > shape_fill_depth_bias(0));
        assert!(STROKE_DEPTH_BIAS > plane_fill_depth_bias(pkey(0)));
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

    /// Count screen-space stroke indices (#1157) whose vertices match `color`.
    fn count_stroke_indices_with_color(scene: &ViewportScene, color: Color32) -> usize {
        let target = color32_to_gpu(color);
        scene
            .stroke_indices
            .iter()
            .filter(|&&index| scene.stroke_vertices[index as usize].color == target)
            .count()
    }

    fn count_wireframe_vertices_with_color(scene: &ViewportScene, color: Color32) -> usize {
        let target = color32_to_gpu(color);
        scene
            .wireframe_indices
            .iter()
            .filter(|&&index| scene.vertices[index as usize].color == target)
            .count()
    }

    #[test]
    fn selected_line_uses_highlight_color_only() {
        use crate::model::{FaceId, Line, ShapeKind};
        use crate::selection::SceneSelection;

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        state.doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);

        let palette = ViewportPalette::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let empty_selection = SceneSelection::default();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            SceneElement::Line(lkey(0)),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });

        // Unselected sketch strokes land in the screen-space stroke buffer (#1157).
        let unselected_base =
            count_stroke_indices_with_color(&unselected, palette.rect_line);
        let selected_base =
            count_stroke_indices_with_color(&selected_scene, palette.rect_line);
        // Selection highlight uses the Wireframe layer (depth-disabled world ribbons)
        // so selected lines show through bodies (#1409).
        let selected_highlight =
            count_wireframe_vertices_with_color(&selected_scene, palette.dim_edge_highlight);
        let selected_highlight_stroke =
            count_stroke_indices_with_color(&selected_scene, palette.dim_edge_highlight);

        assert!(
            unselected_base > 0,
            "unselected line should render as a screen-space stroke"
        );
        assert_eq!(
            selected_base, 0,
            "selected line should not render with base stroke color"
        );
        assert!(
            selected_highlight > 0,
            "selected line should render with highlight color in wireframe layer"
        );
        assert_eq!(
            selected_highlight_stroke, 0,
            "selected line should not render in stroke buffer"
        );
    }






    /// #161: hovering an elements-pane row highlights that element in the viewport — a
    /// hovered body draws its aura in the hover color (depth-disabled layer).
    #[test]
    fn pane_hover_element_highlights_body_in_viewport() {
        let state = state_with_one_body();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        // Fill recolour on hover (#455); outline mask is layered on top (#1155).
        let palette = ViewportPalette::default();
        let build = |hover: Option<ViewportHoverHighlight>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        let hovered = build(Some(ViewportHoverHighlight::Element(
            crate::hierarchy::SceneElement::Body(bkey(0)),
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

    /// One extruded box sliced by a mid-height plane → two fragment bodies + a shadow input.
    fn state_with_sliced_body() -> AppState {
        use crate::actions::Action;
        use crate::construction::{definition_from_reference, plane_from_definition, PlaneReference};
        use crate::model::{ConstructionPlaneParent, ExtrudeFace};

        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        let rect = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            0.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        state.apply(Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect.to_vec())],
            distance: 10.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
            symmetric: false,
        
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,

        });
        let plane = plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                5.0,
                0.0,
            ),
            ConstructionPlaneParent::Root,
        );
        state.doc.construction_planes.insert(plane);
        let cutter = crate::model::SliceCutter::Face(FaceId::ConstructionPlane(
            state.doc.construction_planes.keys().last().unwrap(),
        ));
        let result = state.apply(Action::CreateSliceOperation {
            targets: vec![bkey(0)],
            cutters: vec![cutter],
            extend_infinite: true,
        });
        assert!(
            matches!(result, crate::actions::ActionResult::Ok),
            "slice should succeed: {result:?} / {}",
            state.status
        );
        assert_eq!(state.doc.slice_ops.len(), 1);
        assert!(
            state
                .doc
                .slice_ops
                .values()
                .next()
                .is_some_and(|op| op.outputs.len() >= 2),
            "mid-plane cut should yield at least two fragments"
        );
        state
    }

    /// #1150: hovering a Slice row lights its fragment bodies by **recolouring the main
    /// pass**, not by stacking a translucent coplanar copy of the same mesh (which
    /// z-fought the solid into a mottled purple/body-colour checkerboard). The slice's
    /// shadow input also stays hidden on op hover — it occupies the same outer envelope
    /// as the pieces and would z-fight them the same way.
    #[test]
    fn slice_op_hover_recolors_fragments_without_coplanar_overlay() {
        use crate::model::slice_op_key_for_slot as slckey;

        let state = state_with_sliced_body();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let palette = ViewportPalette::default();
        let build = |hover: Option<ViewportHoverHighlight>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let base = build(None);
        let hovered = build(Some(ViewportHoverHighlight::Element(SceneElement::SliceOp(
            slckey(0),
        ))));

        // The derived-output purple is b > r > g (170, 130, 240). A translucent overlay of
        // the same mesh used to land in plane_fill; the main-pass recolour lives in `indices`.
        let derived_tinted = |scene: &ViewportScene| {
            scene
                .vertices
                .iter()
                .filter(|v| {
                    let [r, g, b, a] = v.color;
                    a > 0.9 && b > r && r > g && b > 0.5
                })
                .count()
        };
        assert!(
            derived_tinted(&hovered) > derived_tinted(&base),
            "hovering a Slice op must recolor its fragment fills in the main pass"
        );
        assert_eq!(
            hovered.plane_fill_indices.len(),
            base.plane_fill_indices.len(),
            "Slice op hover must not stack a translucent coplanar body overlay (z-fights)"
        );
    }

    /// #985: hovering a Selection Exploder loupe that stands for a whole body recolors the
    /// body in the scene — the leaf hover arrives as `PickTarget(Body)`, and a hovered group
    /// loupe holding the body arrives through `extra_pick_highlights`; both must read as a
    /// body hover, since `push_hover_highlight` has no marker of its own for a whole solid.
    #[test]
    fn exploder_body_loupe_hover_recolors_the_body() {
        let state = state_with_one_body();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        // Fill recolour on hover (#985); outline mask layers on top (#1155).
        let palette = ViewportPalette::default();
        let build = |hover: Option<ViewportHoverHighlight>,
                     extra: Vec<crate::construction::PickTargetKind>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
                extra_pick_highlights: extra,
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        // Hover-fill vertices read warm (r > g > b), which the plain render never produces.
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
        let base = build(None, Vec::new());
        let leaf = build(
            Some(ViewportHoverHighlight::PickTarget(
                crate::construction::PickTargetKind::Body(bkey(0)),
            )),
            Vec::new(),
        );
        assert!(
            hover_tinted(&leaf) > hover_tinted(&base),
            "hovering a body's leaf loupe must recolor the body"
        );
        let group = build(
            None,
            vec![crate::construction::PickTargetKind::Body(bkey(0))],
        );
        assert!(
            hover_tinted(&group) > hover_tinted(&base),
            "hovering a group loupe holding the body must recolor the body"
        );
    }

    /// #174: a selected body's fill shifts to the saturated selection blue — some base-layer
    /// vertex carries the selected hue (flat shading preserves channel ratios), and none does
    /// when unselected.
    /// #994: construction geometry is scaffolding for the sketch that owns it — a guide to
    /// dimension against, never model geometry. Outside that sketch it was still drawn, dashed,
    /// standing on the face of the finished part. It draws only while its own sketch is open;
    /// solid geometry in the same sketch is untouched.
    #[test]
    fn construction_geometry_draws_only_inside_its_own_sketch() {
        let mut state = state_with_one_body();
        let sketch = state.doc.lines[lkey(0)].sketch;
        // One construction line and one solid line, both in the same sketch.
        let construction = state.doc.lines.len();
        state.doc.lines.insert(crate::model::Line {
            construction: true,
            ..crate::model::Line::from_local_endpoints(sketch, 0.0, 2.0, 10.0, 2.0)
        });
        let solid = state.doc.lines.len();
        state
            .doc
            .lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 0.0, 3.0, 10.0, 3.0));
        // …and a construction circle, which follows the same rule.
        state.doc.circles.insert(crate::model::Circle {
            construction: true,
            ..crate::model::Circle::from_local_center_radius(sketch, 5.0, 2.5, 1.0, 0.0)
        });
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let build = |session: Option<SketchSession>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: session,
                selection: &state.scene_selection,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        // The construction palette colour is what marks it; flat sketch strokes carry it whole.
        let c = ViewportPalette::default().construction;
        let has_construction_hue = |scene: &ViewportScene| {
            scene.vertices.iter().any(|v| {
                let [r, g, b, a] = v.color;
                a > 0.0
                    && b > 0.02
                    && (r / b - c.r() as f32 / c.b() as f32).abs() < 0.02
                    && (g / b - c.g() as f32 / c.b() as f32).abs() < 0.02
            })
        };
        let outside = build(None);
        assert!(
            !has_construction_hue(&outside),
            "construction geometry must not draw outside its sketch"
        );
        let inside = build(Some(SketchSession { sketch }));
        assert!(
            has_construction_hue(&inside),
            "…and must draw inside it, or the guides would be unusable"
        );
        // The solid line in that same sketch is unaffected either way: only the dashed
        // scaffolding is hidden, not the sketch.
        let segment_count = |scene: &ViewportScene| scene.vertices.len();
        assert!(
            segment_count(&outside) > 0 && segment_count(&inside) > segment_count(&outside),
            "hiding construction removes geometry rather than everything"
        );
        let _ = (construction, solid);
    }

    /// #992: while a two-sided joint is being made, its two parts wear **different** fills —
    /// green for the one that moves, blue for the one holding it — so which is which is visible
    /// in the 3D view rather than only readable off the pane. The tint outranks the selection
    /// blue, which would otherwise paint both the same and answer the wrong question.
    #[test]
    fn a_joints_two_sides_wear_different_fills() {
        let state = state_with_one_body();
        // Selected as well, to prove the tint wins over the selection fill.
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(bkey(0)),
            false,
        );
        let cam = state.cam.clone();
        let viewport = test_viewport();
        // Fill recolour (tint vs selection) — outline still applies but doesn't affect fills.
        let palette = ViewportPalette::default();
        let build = |tint: Vec<(crate::model::BodyKey, Color32)>| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: &selected,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: tint,
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: crate::construction::PICK_HOVER_RGBA,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        // Flat shading scales every channel by one factor, so a fill's channel ratios survive.
        let ratios_of = |scene: &ViewportScene, c: Color32| {
            scene.vertices.iter().any(|v| {
                let [r, g, b, a] = v.color;
                let (cr, cg, cb) = (c.r() as f32, c.g() as f32, c.b() as f32);
                a > 0.0
                    && b > 0.05
                    && (r / b - cr / cb).abs() < 0.02
                    && (g / b - cg / cb).abs() < 0.02
            })
        };
        let mobile = build(vec![(bkey(0), SOLID_FILL_JOINT_MOBILE)]);
        assert!(
            ratios_of(&mobile, SOLID_FILL_JOINT_MOBILE),
            "the mobile side takes the joint green even though it is selected"
        );
        assert!(
            !ratios_of(&mobile, SOLID_FILL_SELECTED),
            "and not the selection blue underneath it"
        );
        let fixed = build(vec![(bkey(0), SOLID_FILL_JOINT_FIXED)]);
        assert!(ratios_of(&fixed, SOLID_FILL_JOINT_FIXED), "the fixed side takes the joint blue");
        // The two really are distinguishable, which is the whole point.
        assert!(
            !ratios_of(&fixed, SOLID_FILL_JOINT_MOBILE)
                && !ratios_of(&mobile, SOLID_FILL_JOINT_FIXED),
            "the two sides must not read as the same colour"
        );
        // With no tint the body is back to the ordinary selection fill.
        assert!(ratios_of(&build(Vec::new()), SOLID_FILL_SELECTED));
    }

    #[test]
    fn selected_body_fill_uses_saturated_blue() {
        let state = state_with_one_body();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(bkey(0)),
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
        let palette = ViewportPalette::default();
        let build = |sel: &SceneSelection| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: sel,
                cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
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

    /// #1110/#1155: selected bodies always get solid-body shading (fill recolour) **and**
    /// outline-mask triangles so the GPU can stroke the silhouette.
    #[test]
    fn selected_body_highlights_with_shading_and_outline() {
        let state = state_with_one_body();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(bkey(0)),
            false,
        );
        let has_selected_hue = |scene: &ViewportScene| {
            scene.vertices.iter().any(|v| {
                let [r, g, b, _] = v.color;
                b > 0.05
                    && (r / b - 112.0 / 224.0).abs() < 0.02
                    && (g / b - 152.0 / 224.0).abs() < 0.02
            })
        };
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &selected,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            has_selected_hue(&scene),
            "selected body must recolour its fill (solid-body shading)"
        );
        assert!(
            !scene.mask_indices.is_empty(),
            "selected body must contribute triangles to the outline mask"
        );
        // Mask vertices are pure red (selected channel): R=1, G=0.
        let mask_has_selected = scene.mask_indices.iter().any(|&i| {
            let v = &scene.vertices[i as usize];
            v.color[0] > 0.9 && v.color[1] < 0.1
        });
        assert!(mask_has_selected, "mask triangles must use the selected (R) channel");
    }

    /// #213: a body in a destructive picker's `cut_highlight_bodies` fills the red cut hue,
    /// taking precedence over the blue selection fill.
    #[test]
    fn cut_highlight_body_fill_uses_red() {
        let state = state_with_one_body();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selected,
            crate::hierarchy::SceneElement::Body(bkey(0)),
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
        let build = |cut: Vec<crate::model::BodyKey>, sel: &SceneSelection| {
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
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
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
            has_cut_hue(&build(vec![bkey(0)], &selected)),
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
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        state.doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(0)),
                line_b: ConstraintLine::Line(lkey(1)),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(lkey(0)),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: Some(&graphics),
            constraint_connector_color: Some(Color32::from_rgb(255, 205, 88)),
        });
        assert!(
            with.overlay_indices.len() + with.stroke_indices.len()
                > without.overlay_indices.len() + without.stroke_indices.len()
        );
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
        let plane = offset_toward_camera(on_plane, Vec3::Z, eye, plane_fill_depth_bias(pkey(0)));
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        // The rectangle is four lines plus geometric constraints (#66); the width dimension
        // is the first constraint that carries an evaluated length.
        let width_dim = state
            .doc
            .constraints
            .keys()
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: std::slice::from_ref(&dim_label),
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(!scene.text_vertices.is_empty());
        assert!(!scene.text_indices.is_empty());
        // Sketch-mode dimensions use the depth-disabled wireframe layer (#1280); closed
        // (if any) would use depth-tested strokes (#1157).
        assert!(
            scene.wireframe_indices.len() > 0
                || scene.stroke_vertices.len() > 0
                || scene.vertices.len() > vertex_count_before,
            "dimension should add line geometry (wireframe, stroke, or mesh)"
        );
    }

    /// #1280: while a sketch is open, dimension extension/witness lines and arrows must
    /// land on the depth-disabled wireframe layer so a body between the camera and the
    /// sketch plane cannot hide them (same always-on-top path as open-sketch lines #1200).
    #[test]
    fn open_sketch_dimension_lines_draw_depth_test_disabled() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
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
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let width_dim = state
            .doc
            .constraints
            .keys()
            .find(|&i| crate::constraints::constraint_evaluated_length(&state.doc, i).is_some())
            .expect("rectangle should have a width dimension constraint");
        let (a, b) = crate::constraints::constraint_segment_endpoints(&state.doc, width_dim)
            .expect("width dimension has endpoints");
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
        let empty_sel = crate::selection::SceneSelection::default();
        let empty_vis = crate::hierarchy::ElementVisibility::default();
        let build = |labels: &[crate::gpu_viewport::ViewportDimLabel]| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: Some(session),
                selection: &empty_sel,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &empty_vis,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: labels,
                dim_label_view: Some(view),
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let baseline = build(&[]);
        let with_dim = build(std::slice::from_ref(&dim_label));
        let wire_delta = with_dim.wireframe_indices.len() - baseline.wireframe_indices.len();
        let stroke_delta = with_dim.stroke_indices.len() - baseline.stroke_indices.len();
        assert!(
            wire_delta >= 6,
            "open-sketch dimension lines must use depth-disabled wireframe (#1280), got wire +{wire_delta}"
        );
        assert_eq!(
            stroke_delta, 0,
            "open-sketch dimension lines must not use depth-tested strokes (bodies would occlude them), got stroke +{stroke_delta}"
        );
        assert!(
            !with_dim.text_vertices.is_empty() && !with_dim.text_indices.is_empty(),
            "dimension label text mesh must still be present"
        );
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
            face: FaceId::ConstructionPlane(pkey(0)),
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
        dashed_doc.lines.insert(construction);
        let mut solid_doc = state.doc.clone();
        solid_doc.lines.insert(solid);
        let scene_fields = (
            &cam,
            viewport,
            ViewportPalette::default(),
            Some(session),
            &state.scene_selection,
            &state.element_visibility,
        );
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        // Open-sketch lines land on the depth-disabled wireframe layer (#1200); construction
        // still emits more dash quads than a solid stroke of the same length.
        let dashed_line_indices = dashed_scene.wireframe_indices.len();
        let solid_line_indices = solid_scene.wireframe_indices.len();
        assert!(
            dashed_line_indices > solid_line_indices,
            "dashed construction line should emit more wireframe segments than a solid line (dashed={dashed_line_indices} solid={solid_line_indices})"
        );
        assert_eq!(
            dashed_scene.stroke_indices.len(),
            solid_scene.stroke_indices.len(),
            "open-sketch construction/solid must not use the depth-tested stroke layer"
        );
    }

    /// #1186: projected lines keep construction semantics but draw **solid** cyan, not dashed.
    #[test]
    fn projected_line_produces_solid_stroke_segments_not_dashes() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        let session = state.sketch_session.unwrap();
        let mut projected = crate::model::Line::from_local_endpoints(
            session.sketch,
            0.0,
            0.0,
            80.0,
            0.0,
        );
        projected.construction = true;
        projected.projection = Some(crate::model::ProjectionSource::Plane {
            plane: pkey(2),
        });
        let mut construction = projected.clone();
        construction.projection = None;
        let mut solid = projected.clone();
        solid.construction = false;
        solid.projection = None;
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let mut projected_doc = state.doc.clone();
        projected_doc.lines.insert(projected);
        let mut construction_doc = state.doc.clone();
        construction_doc.lines.insert(construction);
        let mut solid_doc = state.doc.clone();
        solid_doc.lines.insert(solid);
        let empty_sel = crate::selection::SceneSelection::default();
        let empty_vis = crate::hierarchy::ElementVisibility::default();
        let build = |doc: &crate::model::Document| {
            ViewportScene::build(&ViewportSceneInput {
                doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: Some(session),
                selection: &empty_sel,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &empty_vis,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let baseline = build(&state.doc);
        let projected_scene = build(&projected_doc);
        let construction_scene = build(&construction_doc);
        let solid_scene = build(&solid_doc);
        // #1192/#1200: while the host sketch is open, projected / solid / construction lines
        // all live on the depth-disabled wireframe layer. Projected is solid (same quad count
        // as a solid line), construction still dashes (more quads).
        let projected_wire =
            projected_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let projected_stroke =
            projected_scene.stroke_indices.len() - baseline.stroke_indices.len();
        let construction_wire =
            construction_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let solid_wire = solid_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let solid_stroke = solid_scene.stroke_indices.len() - baseline.stroke_indices.len();
        assert_eq!(
            projected_wire, solid_wire,
            "projected line should be solid (wire +{projected_wire}), not dashed like construction (wire +{construction_wire})"
        );
        assert!(
            construction_wire > solid_wire,
            "sanity: construction still dashes (construction +{construction_wire} solid +{solid_wire})"
        );
        assert_eq!(
            projected_stroke, 0,
            "projected line must not grow the depth-tested stroke layer (would hide behind bodies)"
        );
        assert_eq!(
            solid_stroke, 0,
            "open-sketch solid line must not grow the depth-tested stroke layer (#1200)"
        );
    }

    /// #1192/#1200: while editing a sketch, every line of that sketch shows through bodies
    /// (depth-disabled wireframe layer) — projected cyan references and ordinary solid
    /// strokes alike. Closed sketches still depth-test (see
    /// `closed_sketch_solid_line_still_depth_tests`).
    #[test]
    fn open_sketch_lines_draw_depth_test_disabled() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        let session = state.sketch_session.unwrap();
        let mut projected = crate::model::Line::from_local_endpoints(
            session.sketch,
            0.0,
            0.0,
            80.0,
            0.0,
        );
        projected.construction = true;
        projected.projection = Some(crate::model::ProjectionSource::Plane {
            plane: pkey(2),
        });
        let mut solid = projected.clone();
        solid.construction = false;
        solid.projection = None;
        let mut construction = projected.clone();
        construction.construction = true;
        construction.projection = None;
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let mut projected_doc = state.doc.clone();
        projected_doc.lines.insert(projected);
        let mut solid_doc = state.doc.clone();
        solid_doc.lines.insert(solid);
        let mut construction_doc = state.doc.clone();
        construction_doc.lines.insert(construction);
        let empty_sel = crate::selection::SceneSelection::default();
        let empty_vis = crate::hierarchy::ElementVisibility::default();
        let build = |doc: &crate::model::Document| {
            ViewportScene::build(&ViewportSceneInput {
                doc,
                cam: &cam,
                viewport,
                palette: ViewportPalette::default(),
                sketch_session: Some(session),
                selection: &empty_sel,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                element_visibility: &empty_vis,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        let baseline = build(&state.doc);
        let projected_scene = build(&projected_doc);
        let solid_scene = build(&solid_doc);
        let construction_scene = build(&construction_doc);
        let projected_wire =
            projected_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let projected_stroke =
            projected_scene.stroke_indices.len() - baseline.stroke_indices.len();
        let solid_stroke = solid_scene.stroke_indices.len() - baseline.stroke_indices.len();
        let solid_wire = solid_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let construction_wire =
            construction_scene.wireframe_indices.len() - baseline.wireframe_indices.len();
        let construction_stroke =
            construction_scene.stroke_indices.len() - baseline.stroke_indices.len();
        assert!(
            projected_wire >= 6,
            "projected line must land in the depth-disabled wireframe layer (#1192), got wire +{projected_wire}"
        );
        assert_eq!(
            projected_stroke, 0,
            "projected line must not grow depth-tested strokes (bodies would occlude it), got stroke +{projected_stroke}"
        );
        // #1200: ordinary solid/construction sketch lines of the open sketch also show through.
        assert!(
            solid_wire >= 6,
            "open-sketch solid line must use depth-disabled wireframe (#1200), got wire +{solid_wire}"
        );
        assert_eq!(
            solid_stroke, 0,
            "open-sketch solid line must not use depth-tested strokes, got stroke +{solid_stroke}"
        );
        assert!(
            construction_wire > solid_wire,
            "open-sketch construction still dashes on the wireframe layer (construction +{construction_wire} solid +{solid_wire})"
        );
        assert_eq!(
            construction_stroke, 0,
            "open-sketch construction must not use depth-tested strokes, got stroke +{construction_stroke}"
        );
    }

    /// #1200 / #1157: when no sketch is open, committed solid lines still depth-test so
    /// bodies in front occlude them. Only the open sketch's geometry is always-on.
    #[test]
    fn closed_sketch_solid_line_still_depth_tests() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.lines.insert(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 80.0, 0.0,
        ));
        state.sketch_session = None;
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let empty_sel = crate::selection::SceneSelection::default();
        let empty_vis = crate::hierarchy::ElementVisibility::default();
        let scene = ViewportScene::build(&ViewportSceneInput {
            doc: &state.doc,
            cam: &cam,
            viewport,
            palette: ViewportPalette::default(),
            sketch_session: None,
            selection: &empty_sel,
            cut_highlight_bodies: Vec::new(),
            faded_bodies: Vec::new(),
            sketch_repeat_ghost: Vec::new(),
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            element_visibility: &empty_vis,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
            hover_color: Color32::WHITE,
            document_health: &DocumentHealth::default(),
            constraint_graphics: None,
            constraint_connector_color: None,
        });
        assert!(
            !scene.stroke_indices.is_empty(),
            "closed-sketch solid line must use depth-tested strokes (#1157), got stroke indices empty"
        );
        assert!(
            scene.wireframe_indices.is_empty(),
            "closed-sketch solid line must not use always-on wireframe, got wire +{}",
            scene.wireframe_indices.len()
        );
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

    /// #1202: round caps — a point past a shared endpoint is painted only within half-width
    /// of the geometric vertex (outside the stroke bodies). Square (butt) ends left the
    /// rectangle corners sticking out past the joint, which read as each line overshooting.
    #[test]
    fn stroke_capsule_meets_cleanly_at_shared_endpoint() {
        let a = egui::vec2(0.0, 0.0);
        let b = egui::vec2(100.0, 0.0);
        let c = egui::vec2(50.0, 80.0);
        let unit = |v: egui::Vec2| v / v.length();
        // Two segments meeting at `b` (coincident line ends).
        let half = 1.5_f32;
        // On the geometric vertex: covered.
        assert!(point_in_stroke_capsule(b, a, b, half));
        assert!(point_in_stroke_capsule(b, b, c, half));
        // Half-width past the free end of ab (beyond b, away from a): still covered (round cap).
        let past_ab = b + unit(b - a) * half;
        assert!(point_in_stroke_capsule(past_ab, a, b, half));
        // Further than half-width past the free end: not covered — no overshoot past the cap.
        let overshoot = b + unit(b - a) * (half + 0.25);
        assert!(
            !point_in_stroke_capsule(overshoot, a, b, half),
            "capsule must not paint past half-width of the endpoint"
        );
        // Square-end corner of a thick stroke sits at the endpoint ± half perp and would also
        // extend half past the end along the line direction — that corner is outside a
        // capsule of radius half, so round caps hide the star-shaped joint overshoot.
        let square_corner = b + egui::vec2(half, half); // along + perp for horizontal ab
        assert!(
            !point_in_stroke_capsule(square_corner, a, b, half),
            "square-end corner at (half, half) past the end must be outside the capsule"
        );
        // Bisector of the two free-end outward directions, past half — outside both capsules.
        let out1 = unit(b - a);
        let out2 = unit(b - c);
        let outside = b + unit(out1 + out2) * (half + 0.3);
        assert!(
            !point_in_stroke_capsule(outside, a, b, half)
                && !point_in_stroke_capsule(outside, b, c, half),
            "nothing past the joint's circular silhouette should paint"
        );
    }

    /// #1426: Face Snap's rotation gizmo is yellow, matching the A→A connector.
    #[test]
    fn face_snap_rotation_gizmo_is_yellow() {
        assert_eq!(
            MOVE_ROTATION_GIZMO,
            crate::theme::MOVE_CONNECTOR,
            "the Face Snap spin gizmo should be the same yellow as the connector"
        );
    }
}
/// Manual perf probe for the selection aura (#145): `cargo test aura_perf_probe -- --ignored
/// --nocapture` prints scene-build times with and without a selected body. Ignored by
/// default (timing-based, machine-dependent) — for eyeballing regressions, not CI.
#[cfg(test)]
mod perf_probe {
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::circle_key_for_slot as rkey;
    use super::*;
    use crate::actions::{Action, AppState, Tool};
    use crate::hierarchy::{ElementVisibility, SceneElement};
    use crate::selection::SceneSelection;

    #[test]
    #[ignore]
    fn aura_perf_probe() {
        // A round body (many triangles) plus a second body, big viewport.
        let mut state = AppState::default();
        state.apply(Action::BeginSketch { face: crate::model::FaceId::ConstructionPlane(pkey(0)), viewport: None });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.circles.insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: crate::model::ExtrudeFace::Circle(rkey(0)) });
        state.apply(Action::SetExtrudeDistance { distance: 60.0 });
        state.apply(Action::CommitExtrusion);
        state.apply(Action::ExitSketch);

        let viewport = UiRect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1700.0, 1700.0));
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(bkey(0)), false);
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
                editing_extrusion: None,
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                plane_preview: None,
                active_sketch_face: None,
                palette: ViewportPalette::default(),
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
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
        let t0 = crate::time::Instant::now();
        for _ in 0..n { build(&empty); }
        let base = t0.elapsed() / n;
        let t1 = crate::time::Instant::now();
        for _ in 0..n { build(&selection); }
        let with_aura = t1.elapsed() / n;
        println!("scene build: base {base:?}  with aura {with_aura:?}  aura delta {:?}", with_aura.saturating_sub(base));
    }
}

#[cfg(test)]
mod cut_preview_tests {
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use crate::actions::{Action, AppState, Tool};
    use crate::hierarchy::ElementVisibility;
    use crate::selection::SceneSelection;

    fn one_body_state() -> AppState {
        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: crate::model::FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        state
            .doc
            .circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace {
            face: crate::model::ExtrudeFace::Circle(rkey(0)),
        });
        state.apply(Action::SetExtrudeDistance { distance: 60.0 });
        state.apply(Action::CommitExtrusion);
        state.apply(Action::ExitSketch);
        state
    }

    fn scene_with_cut(
        state: &AppState,
        cut_highlight_bodies: Vec<crate::model::BodyKey>,
    ) -> ViewportScene {
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
            sketch_ghost_lines: Vec::new(),
            edit_preview_meshes: std::collections::HashMap::new(),
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_solid: None,
            repeat_ghosts: Vec::new(),
            cut_surface_ghosts: Vec::new(),
            editing_extrusion: None,
            preview_cut_body: None,
            preview_replacement: PreviewReplacement::default(),
            highlighted_bezier_handles: Vec::new(),
            plane_preview: None,
            active_sketch_face: None,
            palette: ViewportPalette::default(),
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            arrow_gizmos: Vec::new(),
            move_rotation_gizmos: Vec::new(),
            revolve_arc_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
            extra_pick_highlights: Vec::new(),
            colored_pick_highlights: Vec::new(),
            colored_element_highlights: Vec::new(),
            tinted_bodies: Vec::new(),
            colored_segments: Vec::new(),
            parameter_highlight_elements: Vec::new(),
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
        let cut = scene_with_cut(&state, vec![bkey(0)]);
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


/// #1141: a body with circular hole cuts must not thrash feature-edge extraction while
/// the camera orbits in wireframe / solid+wireframe. The feature-edge cache is what keeps
/// those modes interactive on holey parts.
#[cfg(test)]
mod issue_1141_hole_orbit {
    use super::*;
    use crate::actions::AppState;
    use crate::hierarchy::ElementVisibility;
    use crate::selection::SceneSelection;

    fn load_report_doc() -> AppState {
        let bytes = include_bytes!("../../tests/fixtures/issue_1141.json");
        let mut state = AppState::default();
        state.doc = crate::storage::from_json_bytes(bytes).expect("load report doc");
        state.doc.bump_mesh_rev();
        state
    }

    fn hole_body(doc: &crate::model::Document) -> crate::model::BodyKey {
        doc.bodies
            .iter()
            .find(|(_, b)| !b.shadow)
            .map(|(i, _)| i)
            .expect("non-shadow body")
    }

    #[test]
    fn holey_body_meshes_with_feature_edges_and_cylinders() {
        let state = load_report_doc();
        let bi = hole_body(&state.doc);
        let mesh = crate::extrude::body_solid_mesh(&state.doc, bi).expect("mesh");
        assert!(
            mesh.triangles.len() > 100,
            "expected a meshed body with holes, got {} tris",
            mesh.triangles.len()
        );
        let edges = crate::extrude::body_feature_edges(&state.doc, bi);
        // A rectangular bar with two circular holes has many rim segments.
        assert!(
            edges.len() > 50,
            "expected dense circular feature edges, got {}",
            edges.len()
        );
        let cyls = crate::extrude::body_cylinders(&state.doc, bi);
        assert_eq!(cyls.len(), 2, "two hole walls");
    }

    #[test]
    fn feature_edge_cache_is_stable_across_repeated_reads() {
        let state = load_report_doc();
        let bi = hole_body(&state.doc);
        let a = crate::extrude::body_feature_edges(&state.doc, bi);
        let b = crate::extrude::body_feature_edges(&state.doc, bi);
        assert_eq!(a.len(), b.len());
        assert!(std::rc::Rc::ptr_eq(&a, &b), "second read must hit the cache");
    }

    #[test]
    fn solid_wireframe_scene_builds_for_holey_body_while_orbiting() {
        let mut state = load_report_doc();
        let bi = hole_body(&state.doc);
        // Warm caches once.
        let _ = crate::extrude::body_solid_mesh(&state.doc, bi);
        let _ = crate::extrude::body_feature_edges(&state.doc, bi);
        let _ = crate::extrude::body_smooth_normals(&state.doc, bi);

        let build = |state: &AppState| {
            let mut cam = state.cam.clone();
            cam.set_shading_mode(crate::camera::ShadingMode::SolidWireframe);
            let viewport = UiRect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
            let selection = SceneSelection::default();
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                sketch_session: None,
                element_visibility: &ElementVisibility::default(),
                selection: &selection,
                cut_highlight_bodies: Vec::new(),
                faded_bodies: Vec::new(),
                sketch_repeat_ghost: Vec::new(),
                sketch_ghost_lines: Vec::new(),
                edit_preview_meshes: std::collections::HashMap::new(),
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_solid: None,
                repeat_ghosts: Vec::new(),
                cut_surface_ghosts: Vec::new(),
                editing_extrusion: None,
                preview_cut_body: None,
                preview_replacement: PreviewReplacement::default(),
                highlighted_bezier_handles: Vec::new(),
                plane_preview: None,
                active_sketch_face: None,
                palette: ViewportPalette::default(),
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                arrow_gizmos: Vec::new(),
                move_rotation_gizmos: Vec::new(),
                revolve_arc_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                extra_pick_highlights: Vec::new(),
                colored_pick_highlights: Vec::new(),
                colored_element_highlights: Vec::new(),
                tinted_bodies: Vec::new(),
                colored_segments: Vec::new(),
                parameter_highlight_elements: Vec::new(),
                hover_color: Color32::WHITE,
                document_health: &crate::document_health::DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };
        for _ in 0..30 {
            state.cam.orbit(egui::vec2(4.0, 2.0));
            let scene = build(&state);
            assert!(
                !scene.wireframe_indices.is_empty(),
                "solid+wireframe must draw feature edges for the hole rims"
            );
            assert!(!scene.vertices.is_empty());
        }
    }
}
