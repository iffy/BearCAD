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
use eframe::egui::Color32;
use egui::Rect as UiRect;
use glam::{Mat4, Quat, Vec3};

pub const GRID_EXTENT: f32 = 200.0;
pub const GRID_STEP: f32 = 20.0;
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
/// Fill for a **selected** body (#174): a more saturated blue than the neutral body grey,
/// so selection reads on the body itself, not just its aura outline.
pub const SOLID_FILL_SELECTED: Color32 = Color32::from_rgb(112, 152, 224);
/// Highlighted fill for the in-progress extrusion preview.
pub const SOLID_PREVIEW_FILL: Color32 = Color32::from_rgb(120, 215, 230);
/// Opacity of the in-progress extrusion preview body (before it is committed).
pub const SOLID_PREVIEW_OPACITY: f32 = 0.4;
/// Fill opacity for committed bodies in `ShadingMode::TransparentSolid` (#33).
pub const TRANSPARENT_SOLID_OPACITY: f32 = 0.45;
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
const BODY_SILHOUETTE_WIDTH: f32 = 4.0;
/// How far the silhouette glow sits *outside* the body's edges, screen pixels — an aura around
/// the shape rather than a line on it. Each segment is also extended by the same amount at both
/// ends so the offset outlines still meet at corners.
const BODY_SILHOUETTE_OFFSET_PX: f32 = 5.0;

const GIZMO_OFFSET_STROKE_PX: f32 = 2.5;
const GIZMO_OFFSET_STROKE_HOVER_PX: f32 = 4.0;
/// Direction cones on gizmo handles: solid cones pointing along each direction the handle
/// can move, with a small gap between the handle disc and the cone base. Sizes are screen
/// px so the cones hold a constant on-screen size like the disc handle itself.
const GIZMO_CONE_GAP_PX: f32 = 9.0;
const GIZMO_CONE_LENGTH_PX: f32 = 11.0;
const GIZMO_CONE_RADIUS_PX: f32 = 4.5;
const GIZMO_HANDLE_RADIUS_PX: f32 = 6.0;
const GIZMO_HOVER_INNER_RADIUS_PX: f32 = 9.0;
const GIZMO_HOVER_OUTER_RADIUS_PX: f32 = 14.0;
const GIZMO_ANGLE_CIRCLE_SEGMENTS: usize = 48;
const GIZMO_ANGLE_STROKE_PX: f32 = 1.5;
const GIZMO_ANGLE_STROKE_HOVER_PX: f32 = 2.5;
const GIZMO_ANGLE_CONE_GAP_PX: f32 = 7.0;
const GIZMO_ANGLE_CONE_LENGTH_PX: f32 = 8.0;
const GIZMO_ANGLE_CONE_RADIUS_PX: f32 = 3.5;
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
    /// A computed boolean-combined region (#16/#62): rendered as a filled/outlined polygon
    /// directly from its resolved world-space loop, since (unlike `SketchFace`) it has no
    /// `FaceId` of its own — it's not a stored shape, just `ExtrudeFace::Boolean`'s on-demand
    /// geometry.
    BooleanRegion { world_loop: Vec<Vec3> },
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
    pub element_visibility: &'a ElementVisibility,
    pub preview_rect: Option<PreviewRect>,
    pub preview_line: Option<Line>,
    pub preview_circle: Option<Circle>,
    /// In-progress extrusion (rendered as a translucent preview solid).
    pub preview_extrusion: Option<crate::model::Extrusion>,
    /// Index of the extrusion currently being edited, if any. Its committed body
    /// is suppressed so only the ghost preview is shown while editing.
    pub editing_extrusion: Option<usize>,
    /// Target body when the in-progress extrusion is a **cut** (#142): its committed solid is
    /// suppressed and replaced by a translucent preview of the *cut result* (that body with
    /// `preview_extrusion` subtracted) rather than the additive block, so the preview looks
    /// like the finished cut. `None` for add/new-body extrudes (block preview) or when the
    /// kernel can't build the cut result.
    pub preview_cut_body: Option<usize>,
    pub plane_preview: Option<ViewportPlanePreview>,
    pub active_sketch_face: Option<FaceId>,
    pub dimension_labels: &'a [ViewportDimLabel],
    pub dim_label_view: Option<PlanarLabelView>,
    pub plane_gizmo: Option<ViewportPlaneGizmo>,
    pub extrude_gizmo: Option<ViewportExtrudeGizmo>,
    /// Push/pull gizmo for the in-progress chamfer/fillet tool; reuses the same offset-gizmo
    /// mesh as [`ViewportSceneInput::extrude_gizmo`] (#37/#38).
    pub vertex_treatment_gizmo: Option<ViewportExtrudeGizmo>,
    /// Live preview of the treated corner while the chamfer/fillet gizmo is being placed or
    /// dragged (#76): world-space polyline from the first line's far endpoint, through the
    /// truncated point, the bridge, the other truncated point, to the second line's far
    /// endpoint. Recomputed every frame from the live gizmo amount.
    pub vertex_treatment_preview: Option<VertexTreatmentPreviewGeom>,
    pub hover_highlight: Option<ViewportHoverHighlight>,
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
        let body_meshes: Vec<Option<crate::extrude::SolidMesh>> = (0..input.doc.bodies.len())
            .map(|bi| {
                let body = &input.doc.bodies[bi];
                let visible = !body.deleted
                    && input
                        .element_visibility
                        .effective_visible(input.doc, SceneElement::Body(bi));
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
            if preview_cut.as_ref().is_some_and(|(cut_bi, _)| *cut_bi == bi) {
                continue;
            }
            let Some(solid) = body_meshes[bi].as_ref() else {
                continue;
            };
            // A selected body fills in the saturated selection blue (#174); the aura
            // outline still draws on top via `push_selection`.
            let fill = if input.selection.is_selected(SceneElement::Body(bi)) {
                SOLID_FILL_SELECTED
            } else {
                SOLID_FILL
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
                    mesh.push_solid(&solid, fill, input.cam, cap_plane);
                }
                crate::camera::ShadingMode::TransparentSolid => {
                    mesh.push_solid_translucent(&solid, fill, TRANSPARENT_SOLID_OPACITY);
                }
                crate::camera::ShadingMode::Wireframe => {
                    mesh.push_solid_wireframe(
                        &solid,
                        WIREFRAME_LINE_COLOR,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::SolidWireframe => {
                    mesh.push_solid(&solid, fill, input.cam, cap_plane);
                    mesh.push_solid_wireframe(
                        &solid,
                        WIREFRAME_LINE_COLOR,
                        input.cam,
                        input.viewport,
                        &vp,
                    );
                }
                crate::camera::ShadingMode::Realistic => {
                    mesh.push_solid_realistic(&solid, fill, input.cam, cap_plane);
                }
            }
        }
        // Live preview of the in-progress extrusion (semi-transparent until committed). A cut
        // (#142) previews the whole cut *result* solid — the target body with this extrusion
        // subtracted — in place of the additive block, so it looks like the finished cut.
        if let Some((_, solid)) = &preview_cut {
            mesh.push_solid_translucent(solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
        } else if let Some(preview) = input.preview_extrusion.as_ref() {
            if let Some(solid) = crate::extrude::extrusion_mesh(input.doc, preview) {
                mesh.push_solid_translucent(&solid, SOLID_PREVIEW_FILL, SOLID_PREVIEW_OPACITY);
            }
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

        // Draggable tangent-handle markers for curved lines in the active sketch (#54): a
        // dashed guide from each endpoint to its handle, plus a disc at the handle itself.
        if let Some(session) = input.sketch_session {
            mesh.set_index_layer(MeshIndexLayer::Gizmo);
            let handle_color = input.palette.preview;
            for line in input.doc.lines.iter() {
                if line.deleted || line.sketch != session.sketch {
                    continue;
                }
                let Some([c0, c1]) = line.bezier else {
                    continue;
                };
                let Some(frame) = sketch_geometry_frame(input.doc, line.sketch) else {
                    continue;
                };
                let p0 = crate::face::local_to_world(&frame, line.x0, line.y0);
                let p1 = crate::face::local_to_world(&frame, line.x1, line.y1);
                let h0 = crate::face::local_to_world(&frame, c0.0, c0.1);
                let h1 = crate::face::local_to_world(&frame, c1.0, c1.1);
                mesh.push_dashed_line_segment(p0, h0, handle_color, 1.5, input.cam, input.viewport, &vp);
                mesh.push_dashed_line_segment(p1, h1, handle_color, 1.5, input.cam, input.viewport, &vp);
                mesh.push_point_marker(h0, handle_color, 5.0, input.cam, input.viewport, &vp);
                mesh.push_point_marker(h1, handle_color, 5.0, input.cam, input.viewport, &vp);
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
    ) {
        let e = GRID_EXTENT;
        // Solid ground (#159): one filled plane in the grid's grey (darkened so bodies and
        // sketches still read against it), pushed slightly away from the camera with the
        // same bias the grid lines use so body bottoms resting on z = 0 never z-fight it.
        // The axis lines below still draw on top for orientation.
        if cam.ground_display() == crate::camera::GroundDisplay::Solid {
            let eye = cam.eye();
            let fill = sketch_ground_color(scale_color(palette.grid, 0.55), dim);
            let corners = [
                Vec3::new(-e, -e, 0.0),
                Vec3::new(e, -e, 0.0),
                Vec3::new(e, e, 0.0),
                Vec3::new(-e, e, 0.0),
            ];
            let lifted = offset_corners_toward_camera(corners, Vec3::Z, eye, GRID_DEPTH_BIAS);
            self.push_quad_fill(lifted, fill);
        } else {
            let mut t = -e;
            while t <= e + 0.001 {
            let base = if t.abs() < 0.001 {
                palette.grid_axis
            } else {
                palette.grid
            };
            let color = sketch_ground_color(base, dim);
            self.push_line_segment_with_bias(
                Vec3::new(-e, t, 0.0),
                Vec3::new(e, t, 0.0),
                color,
                1.0,
                cam,
                viewport,
                view_proj,
                GRID_DEPTH_BIAS,
            );
            self.push_line_segment_with_bias(
                Vec3::new(t, -e, 0.0),
                Vec3::new(t, e, 0.0),
                color,
                1.0,
                cam,
                viewport,
                view_proj,
                GRID_DEPTH_BIAS,
            );
                t += GRID_STEP;
            }
        }
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(e, 0.0, 0.0),
            sketch_ground_color(palette.x_axis, dim),
            2.0,
            cam,
            viewport,
            view_proj,
            GRID_DEPTH_BIAS,
        );
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(0.0, e, 0.0),
            sketch_ground_color(palette.y_axis, dim),
            2.0,
            cam,
            viewport,
            view_proj,
            GRID_DEPTH_BIAS,
        );
        self.push_line_segment_with_bias(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, e),
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
        body_meshes: &[Option<crate::extrude::SolidMesh>],
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        base_color: Color32,
    ) {
        if selection.is_empty() {
            return;
        }
        // Selected bodies get one combined screen-space aura (#145/#148); a selected
        // extrusion contributes only its own solid (#154), so picking an Extrude element
        // outlines just the part it created rather than the whole body it merged into.
        let mut aura_bodies: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut aura_extrusions: std::collections::HashSet<usize> = std::collections::HashSet::new();
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
                // A selected body (or extrusion) gets a glowing blue outline traced around its
                // camera-facing silhouette (#145).
                SceneElement::Body(index) => {
                    aura_bodies.insert(index);
                }
                SceneElement::Extrusion(index) => {
                    aura_extrusions.insert(index);
                }
                _ => {}
            }
        }
        if !aura_bodies.is_empty() || !aura_extrusions.is_empty() {
            self.push_body_aura(
                doc,
                &aura_bodies,
                &aura_extrusions,
                body_meshes,
                BODY_SILHOUETTE_COLOR,
                cam,
                viewport,
                view_proj,
            );
        }
    }

    /// Draw the selection aura (#145/#148) around the selected bodies as a **2D screen-space
    /// effect**: all selected bodies are rasterized into one projected footprint, the aura is
    /// the iso-contour of that footprint's distance field at [`BODY_SILHOUETTE_OFFSET_PX`]
    /// (see [`AuraField`]), and a contour stretch is dropped where a *non-selected* body sits
    /// in front of the silhouette it outlines. Union-by-construction: overlapping bodies get
    /// one outline, and bodies closer than twice the offset have their auras join. The
    /// contour is unprojected onto the plane through `cam.target` facing the camera, where
    /// the px-width line renderer is exact, and drawn glow-under-core.
    fn push_body_aura(
        &mut self,
        doc: &Document,
        bodies: &std::collections::HashSet<usize>,
        extrusions: &std::collections::HashSet<usize>,
        body_meshes: &[Option<crate::extrude::SolidMesh>],
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
    ) {
        let eye = cam.eye();
        let project = |w: Vec3| cam.project(w, viewport, view_proj);

        // Project every body once up front; the selected geometry's screen bounds (clipped
        // to the viewport) define the field region, so grid work scales with the selection.
        // `body_meshes` is the per-frame mesh list the body render already built — hidden
        // bodies are `None` there, so they neither get an aura nor occlude one.
        type ScreenTri = ([egui::Pos2; 3], [f32; 3]);
        let mut sel_tris: Vec<ScreenTri> = Vec::new();
        let mut other_tris: Vec<ScreenTri> = Vec::new();
        let mut sel_bounds: Option<UiRect> = None;
        let stamp = |tri: &[Vec3; 3],
                         selected: bool,
                         sel_tris: &mut Vec<ScreenTri>,
                         other_tris: &mut Vec<ScreenTri>,
                         sel_bounds: &mut Option<UiRect>| {
            let (Some(a), Some(b), Some(c)) = (project(tri[0]), project(tri[1]), project(tri[2]))
            else {
                return;
            };
            if selected {
                let tri_rect = UiRect::from_min_max(
                    egui::pos2(a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y)),
                    egui::pos2(a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y)),
                );
                *sel_bounds = Some(match *sel_bounds {
                    Some(r) => r.union(tri_rect),
                    None => tri_rect,
                });
            }
            let depths = [
                (tri[0] - eye).length(),
                (tri[1] - eye).length(),
                (tri[2] - eye).length(),
            ];
            if selected {
                sel_tris.push(([a, b, c], depths));
            } else {
                other_tris.push(([a, b, c], depths));
            }
        };
        for (bi, mesh) in body_meshes.iter().enumerate() {
            let Some(mesh) = mesh else {
                continue;
            };
            let selected = bodies.contains(&bi);
            for tri in &mesh.triangles {
                stamp(tri, selected, &mut sel_tris, &mut other_tris, &mut sel_bounds);
            }
        }
        // A selected extrusion contributes just its own analytic solid (#154): the aura then
        // outlines the part that extrusion created, while the rest of its body (stamped as
        // non-selected above) occludes the outline wherever it stands in front. Skip
        // extrusions whose whole body is already selected — the body footprint covers them.
        for &ei in extrusions {
            let Some(ext) = doc.extrusions.get(ei) else {
                continue;
            };
            if ext.deleted
                || crate::model::body_index_for_extrusion(doc, ei)
                    .is_some_and(|bi| bodies.contains(&bi))
            {
                continue;
            }
            let Some(mesh) = crate::extrude::extrusion_mesh(doc, ext) else {
                continue;
            };
            for tri in &mesh.triangles {
                stamp(tri, true, &mut sel_tris, &mut other_tris, &mut sel_bounds);
            }
        }
        let Some(sel_bounds) = sel_bounds else {
            return;
        };
        let region = sel_bounds.intersect(viewport.expand(AURA_MARGIN_PX));
        let Some(mut field) = AuraField::new(region) else {
            return;
        };
        for (pts, depths) in &sel_tris {
            field.stamp_triangle(*pts, *depths, true);
        }
        field.finalize();
        // Non-selected bodies only matter to the occlusion test, which reads their depth in
        // the narrow band around the silhouette — stamp them band-restricted so a large
        // unselected body doesn't pay for its whole projected area.
        if !other_tris.is_empty() {
            let mut band = vec![false; field.w * field.h];
            for &i in &field.touched {
                band[i] = true;
            }
            for (pts, depths) in &other_tris {
                field.stamp_other_in_band(*pts, *depths, &band);
            }
        }
        let contour = field.visible_contour(BODY_SILHOUETTE_OFFSET_PX);
        if contour.is_empty() {
            return;
        }

        // Unproject onto the camera-facing plane through the target: overlay lines there
        // project at exactly the requested pixel width (`line_screen_quad`'s world-per-px
        // conversion is anchored at the target distance).
        let forward = (cam.target - eye).normalize_or_zero();
        if forward.length_squared() < 0.5 {
            return;
        }
        let unproject = |p: egui::Pos2| -> Option<Vec3> {
            cam.ray_plane_hit(p, viewport, view_proj, cam.target, forward)
        };
        // Chain the raw marching-squares steps into polylines, smooth, and thin them out —
        // then draw each as one solid mitered stroke, so the aura reads as a single clean line.
        // Drawn through the depth-test-disabled layer: the aura's geometry sits on the
        // camera-target plane, and depth testing it would let unrelated nearer geometry (the
        // ground grid, bodies in front) clip it — occlusion by non-selected bodies is already
        // decided per contour stretch above, and nothing else may cover the aura.
        let restore_layer = self.index_layer;
        self.set_index_layer(MeshIndexLayer::Wireframe);
        for (points, closed) in chain_contour_segments(&contour) {
            let smoothed = chaikin_smooth(&points, closed, 2);
            let sparse = resample_polyline(&smoothed, AURA_DRAW_STEP_PX);
            if sparse.len() >= 2 {
                self.push_aura_stroke(
                    &sparse,
                    closed,
                    BODY_SILHOUETTE_WIDTH,
                    color,
                    &unproject,
                );
            }
        }
        self.set_index_layer(restore_layer);
    }

    /// Draw a screen-space polyline as one continuous mitered stroke, unprojected onto the
    /// camera-facing target plane. Unlike a chain of per-segment `push_line_segment` quads,
    /// consecutive segments share their join vertices, so direction changes leave no notches
    /// (where an under-layer would peek through) and no protruding corners.
    fn push_aura_stroke(
        &mut self,
        points: &[egui::Pos2],
        closed: bool,
        width_px: f32,
        color: Color32,
        unproject: &impl Fn(egui::Pos2) -> Option<Vec3>,
    ) {
        // A closed chain arrives with its first point repeated at the end; drop the
        // duplicate and wrap the neighbor lookups instead.
        let pts = if closed && points.len() >= 2 && points.first() == points.last() {
            &points[..points.len() - 1]
        } else {
            points
        };
        let n = pts.len();
        if n < 2 {
            return;
        }
        let half = width_px * 0.5;
        let perp = |v: egui::Vec2| egui::vec2(-v.y, v.x);
        let seg_dir = |i: usize| -> egui::Vec2 {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            let d = b - a;
            let len = d.length();
            if len < 1e-6 {
                egui::Vec2::ZERO
            } else {
                d / len
            }
        };
        // Left/right stroke border per vertex: offset along the miter (average of the two
        // adjacent segment perpendiculars), scaled to keep the stroke width, clamped so
        // near-reversals don't spike.
        let mut left: Vec<Option<Vec3>> = Vec::with_capacity(n);
        let mut right: Vec<Option<Vec3>> = Vec::with_capacity(n);
        for i in 0..n {
            let din = if i > 0 {
                seg_dir(i - 1)
            } else if closed {
                seg_dir(n - 1)
            } else {
                seg_dir(0)
            };
            let dout = if i + 1 < n || closed { seg_dir(i) } else { seg_dir(i - 1) };
            let mut tangent = din + dout;
            if tangent.length() < 1e-6 {
                tangent = dout;
            }
            let tangent = tangent.normalized();
            let normal = perp(tangent);
            // Miter length correction, clamped at 2x so sharp kinks bevel instead of spiking.
            let cos_half = normal.dot(perp(dout)).abs().max(0.5);
            let offset = normal * (half / cos_half);
            left.push(unproject(pts[i] + offset));
            right.push(unproject(pts[i] - offset));
        }
        let quads = if closed { n } else { n - 1 };
        for i in 0..quads {
            let j = (i + 1) % n;
            let (Some(l0), Some(r0), Some(l1), Some(r1)) = (left[i], right[i], left[j], right[j])
            else {
                continue;
            };
            self.push_triangle(l0, r0, r1, color);
            self.push_triangle(l0, r1, l1, color);
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
            ViewportHoverHighlight::BooleanRegion { world_loop } => {
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
        body_meshes: &[Option<crate::extrude::SolidMesh>],
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
            SceneElement::FaceEdge(_) | SceneElement::Image(_) => {}
            // Bodies and extrusions get their aura, tinted with the hover color.
            SceneElement::Body(index) => {
                let bodies: std::collections::HashSet<usize> = [index].into_iter().collect();
                self.push_body_aura(
                    doc,
                    &bodies,
                    &std::collections::HashSet::new(),
                    body_meshes,
                    color,
                    cam,
                    viewport,
                    view_proj,
                );
            }
            SceneElement::Extrusion(index) => {
                let extrusions: std::collections::HashSet<usize> = [index].into_iter().collect();
                self.push_body_aura(
                    doc,
                    &std::collections::HashSet::new(),
                    &extrusions,
                    body_meshes,
                    color,
                    cam,
                    viewport,
                    view_proj,
                );
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
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        self.push_line_segment(a, b, color, 4.0, cam, viewport, view_proj);
        for p in [a, b] {
            push_screen_disc(self, p, 5.0, color, cam, viewport, view_proj, project);
        }
    }

    /// Hover highlight for a (possibly curved) polyline: the sampled path plus discs at its
    /// two true endpoints only (not at every interior sample).
    fn push_polyline_hover(
        &mut self,
        points: &[Vec3],
        color: Color32,
        cam: &Camera,
        viewport: UiRect,
        view_proj: &Mat4,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ) {
        for pair in points.windows(2) {
            self.push_line_segment(pair[0], pair[1], color, 4.0, cam, viewport, view_proj);
        }
        if let (Some(&a), Some(&b)) = (points.first(), points.last()) {
            for p in [a, b] {
                push_screen_disc(self, p, 5.0, color, cam, viewport, view_proj, project);
            }
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

/// Grid resolution of the aura's screen-space footprint field, pixels per cell. Small enough
/// that the marching-squares contour (which interpolates on the distance field) stays smooth
/// under the glow width; doubled automatically for huge extents via [`AURA_MAX_CELLS`].
const AURA_CELL_PX: f32 = 1.5;
/// Cap on the aura field's cell count; the cell size grows until the grid fits, so a
/// selection filling the screen gets a slightly coarser (still smoothed) contour instead of
/// a multi-million-cell field recomputed every frame.
const AURA_MAX_CELLS: usize = 600_000;
/// Extra field margin beyond the viewport so a silhouette cut off by the screen edge closes
/// offscreen instead of drawing an aura line along the viewport border.
const AURA_MARGIN_PX: f32 = BODY_SILHOUETTE_OFFSET_PX + 8.0;
/// Relative depth slack for the aura's occlusion test — a non-selected surface must be
/// meaningfully in front of the selected silhouette (not just touching it) to occlude.
const AURA_OCCLUSION_DEPTH_SLACK: f32 = 2e-3;

/// Scanline-rasterize a screen-space triangle over the aura grid's cell centers, calling
/// `write(cell_index, depth)` for each covered center. Per grid row, the row's center line
/// is intersected with the triangle edges to get the covered x-span and the depth at its
/// ends (depth is affine across a triangle in screen space), then depth steps linearly —
/// O(covered cells) with a couple of adds per cell, instead of a barycentric solve per cell.
fn aura_scanline(
    w: usize,
    h: usize,
    cell: f32,
    origin: egui::Pos2,
    p: [egui::Pos2; 3],
    depth: [f32; 3],
    mut write: impl FnMut(usize, f32),
) {
    let min_y = p[0].y.min(p[1].y).min(p[2].y);
    let max_y = p[0].y.max(p[1].y).max(p[2].y);
    let gy0 = ((min_y - origin.y) / cell).ceil().max(0.0) as usize;
    let gy1 = (((max_y - origin.y) / cell).floor()).min(h as f32 - 1.0);
    if gy1 < 0.0 {
        return;
    }
    let gy1 = gy1 as usize;
    let edges = [(0usize, 1usize), (1, 2), (2, 0)];
    for gy in gy0..=gy1 {
        let cy = origin.y + gy as f32 * cell;
        // Crossings of the row's center line with the triangle edges: (x, depth) pairs.
        let mut lo: Option<(f32, f32)> = None;
        let mut hi: Option<(f32, f32)> = None;
        for &(a, b) in &edges {
            let (pa, pb) = (p[a], p[b]);
            if (pa.y - cy) * (pb.y - cy) > 0.0 || (pa.y - pb.y).abs() < 1e-12 {
                continue;
            }
            let t = ((cy - pa.y) / (pb.y - pa.y)).clamp(0.0, 1.0);
            let x = pa.x + (pb.x - pa.x) * t;
            let d = depth[a] + (depth[b] - depth[a]) * t;
            if lo.is_none_or(|(lx, _)| x < lx) {
                lo = Some((x, d));
            }
            if hi.is_none_or(|(hx, _)| x > hx) {
                hi = Some((x, d));
            }
        }
        let (Some((x_lo, d_lo)), Some((x_hi, d_hi))) = (lo, hi) else {
            continue;
        };
        let gx0 = ((x_lo - origin.x) / cell).ceil().max(0.0) as usize;
        let gx1f = ((x_hi - origin.x) / cell).floor();
        if gx1f < 0.0 {
            continue;
        }
        let gx1 = (gx1f as usize).min(w - 1);
        if gx0 > gx1 {
            continue;
        }
        let span = (x_hi - x_lo).max(1e-12);
        let d_step = (d_hi - d_lo) / span * cell;
        let mut d = d_lo + ((origin.x + gx0 as f32 * cell) - x_lo) / span * (d_hi - d_lo);
        let row = gy * w;
        for gx in gx0..=gx1 {
            write(row + gx, d);
            d += d_step;
        }
    }
}

/// Screen-space footprint field backing the selection aura (#145/#148): all selected bodies
/// rasterized into one coarse grid (with per-cell depth), a chamfer distance transform of
/// that mask, and the minimum depth of *non-selected* bodies per cell for occlusion. The
/// aura is the marching-squares iso-contour of the distance field, drawn only where no
/// non-selected body sits in front of the nearest selected silhouette.
struct AuraField {
    w: usize,
    h: usize,
    cell: f32,
    /// Screen position of cell (0, 0)'s center.
    origin: egui::Pos2,
    /// Min depth (eye distance) of selected-body coverage per cell; +inf where uncovered.
    sel_depth: Vec<f32>,
    /// Min depth of non-selected-body coverage per cell; +inf where uncovered.
    other_depth: Vec<f32>,
    /// Distance (px) to the nearest selected-covered cell (0 inside), computed only within
    /// the narrow band the aura contour can reach (+inf beyond it).
    dist_px: Vec<f32>,
    /// Depth of that nearest selected cell (the silhouette the aura outlines).
    near_depth: Vec<f32>,
    /// Cells with a finite `dist_px` (mask boundary + the band around it), filled by
    /// [`Self::finalize`] — the only places the iso-contour can pass, so marching squares
    /// visits just their incident squares instead of sweeping the grid.
    touched: Vec<usize>,
}

impl AuraField {
    /// Field over `region` (screen px) expanded by [`AURA_MARGIN_PX`] — pass the selected
    /// bodies' projected bounds (clipped to the viewport), not the whole viewport, so the
    /// per-frame grid work scales with the selection's on-screen size.
    fn new(region: UiRect) -> Option<Self> {
        if !region.is_positive() {
            return None;
        }
        let mut cell = AURA_CELL_PX;
        let (mut w, mut h);
        loop {
            w = ((region.width() + 2.0 * AURA_MARGIN_PX) / cell).ceil() as usize + 1;
            h = ((region.height() + 2.0 * AURA_MARGIN_PX) / cell).ceil() as usize + 1;
            if w.saturating_mul(h) <= AURA_MAX_CELLS {
                break;
            }
            cell *= 1.25;
        }
        if w < 2 || h < 2 {
            return None;
        }
        let n = w * h;
        Some(Self {
            w,
            h,
            cell,
            origin: egui::pos2(
                region.min.x - AURA_MARGIN_PX + cell * 0.5,
                region.min.y - AURA_MARGIN_PX + cell * 0.5,
            ),
            sel_depth: vec![f32::INFINITY; n],
            other_depth: vec![f32::INFINITY; n],
            dist_px: vec![f32::INFINITY; n],
            near_depth: vec![f32::INFINITY; n],
            touched: Vec::new(),
        })
    }

    fn cell_center(&self, x: usize, y: usize) -> egui::Pos2 {
        egui::pos2(
            self.origin.x + x as f32 * self.cell,
            self.origin.y + y as f32 * self.cell,
        )
    }

    /// Rasterize a projected triangle into the field: min-depth per covered cell center
    /// (depth stepped linearly along each row — depth is affine across the triangle in
    /// screen space), into the selected or non-selected layer.
    fn stamp_triangle(&mut self, p: [egui::Pos2; 3], depth: [f32; 3], selected: bool) {
        let layer = if selected {
            &mut self.sel_depth
        } else {
            &mut self.other_depth
        };
        aura_scanline(self.w, self.h, self.cell, self.origin, p, depth, |i, d| {
            if d < layer[i] {
                layer[i] = d;
            }
        });
    }

    /// Rasterize a non-selected triangle's depth, but only into `band` cells (the narrow
    /// band around the selected silhouette where the occlusion test reads `other_depth`) —
    /// so unselected bodies never pay for their full projected area.
    fn stamp_other_in_band(&mut self, p: [egui::Pos2; 3], depth: [f32; 3], band: &[bool]) {
        let layer = &mut self.other_depth;
        aura_scanline(self.w, self.h, self.cell, self.origin, p, depth, |i, d| {
            if band[i] && d < layer[i] {
                layer[i] = d;
            }
        });
    }

    /// Narrow-band distance transform of the selected mask: exact Euclidean cell-to-cell
    /// distances seeded only from the mask's **boundary cells** and written only within the
    /// band the aura contour can reach (`BODY_SILHOUETTE_OFFSET_PX` plus interpolation
    /// slack). O(boundary × band²) instead of a full-grid chamfer sweep — for a typical
    /// selection that's a few hundred thousand cheap updates rather than tens of millions.
    /// Propagates the boundary cell's depth alongside the distance and records every cell
    /// with a finite distance in `touched` (the marching-squares candidate band).
    fn finalize(&mut self) {
        let (w, h) = (self.w, self.h);
        for i in 0..w * h {
            if self.sel_depth[i].is_finite() {
                self.dist_px[i] = 0.0;
                self.near_depth[i] = self.sel_depth[i];
            }
        }
        let band_px = BODY_SILHOUETTE_OFFSET_PX + 3.0 * self.cell;
        let r = (band_px / self.cell).ceil() as i32;
        let mut touched = Vec::new();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = y as usize * w + x as usize;
                if self.dist_px[i] != 0.0 {
                    continue; // not a mask cell
                }
                // Only boundary mask cells seed the band (a grid edge counts as boundary so
                // an offscreen-cut silhouette still closes its contour there).
                let boundary = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        return true;
                    }
                    self.dist_px[ny as usize * w + nx as usize] != 0.0
                });
                if !boundary {
                    continue;
                }
                touched.push(i);
                let depth = self.sel_depth[i];
                for dy in -r..=r {
                    let ny = y + dy;
                    if ny < 0 || ny >= h as i32 {
                        continue;
                    }
                    for dx in -r..=r {
                        let nx = x + dx;
                        if nx < 0 || nx >= w as i32 {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if self.dist_px[j] == 0.0 {
                            continue; // inside the mask
                        }
                        let d = self.cell * ((dx * dx + dy * dy) as f32).sqrt();
                        if d < self.dist_px[j] {
                            if !self.dist_px[j].is_finite() {
                                touched.push(j);
                            }
                            self.dist_px[j] = d;
                            self.near_depth[j] = depth;
                        }
                    }
                }
            }
        }
        self.touched = touched;
    }

    /// The aura contour: marching squares over the distance field at `iso_px`, dropping the
    /// stretches where a non-selected body is in front of the silhouette being outlined.
    /// Only squares incident to `touched` cells are visited — the contour cannot pass
    /// anywhere else. Returns screen-space segments.
    fn visible_contour(&self, iso_px: f32) -> Vec<(egui::Pos2, egui::Pos2)> {
        let mut out = Vec::new();
        let (w, h) = (self.w, self.h);
        let mut seen = vec![false; w * h];
        for &cell in &self.touched {
            let (cx, cy) = (cell % w, cell / w);
            for (x, y) in [
                (cx.wrapping_sub(1), cy.wrapping_sub(1)),
                (cx, cy.wrapping_sub(1)),
                (cx.wrapping_sub(1), cy),
                (cx, cy),
            ] {
                // `wrapping_sub` underflow lands far above the grid bounds and is skipped.
                if x >= w - 1 || y >= h - 1 {
                    continue;
                }
                let i00 = y * w + x;
                if seen[i00] {
                    continue;
                }
                seen[i00] = true;
                let (i10, i01, i11) = (i00 + 1, i00 + w, i00 + w + 1);
                let v = [
                    self.dist_px[i00],
                    self.dist_px[i10],
                    self.dist_px[i11],
                    self.dist_px[i01],
                ];
                let inside = [v[0] <= iso_px, v[1] <= iso_px, v[2] <= iso_px, v[3] <= iso_px];
                let case = (inside[0] as usize)
                    | (inside[1] as usize) << 1
                    | (inside[2] as usize) << 2
                    | (inside[3] as usize) << 3;
                if case == 0 || case == 15 {
                    continue;
                }
                // Occlusion: the aura at this square outlines the nearest selected surface;
                // a non-selected body covering the square *in front of* that surface hides it.
                let near = self.near_depth[i00];
                let other = self.other_depth[i00];
                if other < near - near.abs() * AURA_OCCLUSION_DEPTH_SLACK {
                    continue;
                }
                let p00 = self.cell_center(x, y);
                let step = self.cell;
                let lerp_t = |a: f32, b: f32| {
                    if (b - a).abs() < 1e-12 {
                        0.5
                    } else {
                        ((iso_px - a) / (b - a)).clamp(0.0, 1.0)
                    }
                };
                // Edge crossing points: top (00→10), right (10→11), bottom (01→11), left (00→01).
                let top = egui::pos2(p00.x + step * lerp_t(v[0], v[1]), p00.y);
                let right = egui::pos2(p00.x + step, p00.y + step * lerp_t(v[1], v[2]));
                let bottom = egui::pos2(p00.x + step * lerp_t(v[3], v[2]), p00.y + step);
                let left = egui::pos2(p00.x, p00.y + step * lerp_t(v[0], v[3]));
                let mut emit = |a: egui::Pos2, b: egui::Pos2| out.push((a, b));
                match case {
                    1 | 14 => emit(left, top),
                    2 | 13 => emit(top, right),
                    4 | 11 => emit(right, bottom),
                    8 | 7 => emit(bottom, left),
                    3 | 12 => emit(left, right),
                    6 | 9 => emit(top, bottom),
                    5 => {
                        emit(left, top);
                        emit(right, bottom);
                    }
                    10 => {
                        emit(top, right);
                        emit(bottom, left);
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

/// Chain unordered contour segments (as emitted by marching squares, whose shared endpoints
/// are bitwise-identical across neighboring cells) into polylines. Returns each chain with
/// whether it closed back on itself. Occlusion-filtered gaps naturally become open chains.
fn chain_contour_segments(segments: &[(egui::Pos2, egui::Pos2)]) -> Vec<(Vec<egui::Pos2>, bool)> {
    type Key = (i32, i32);
    let key = |p: egui::Pos2| -> Key { ((p.x * 8.0).round() as i32, (p.y * 8.0).round() as i32) };
    let mut adjacency: std::collections::HashMap<Key, Vec<usize>> = std::collections::HashMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        adjacency.entry(key(*a)).or_default().push(i);
        adjacency.entry(key(*b)).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut chains = Vec::new();
    // Walk from every unused segment; extend forward until the chain closes or dead-ends,
    // then extend the other way for open chains.
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (a, b) = segments[start];
        let mut points = vec![a, b];
        let mut closed = false;
        for reversed in [false, true] {
            if closed {
                break;
            }
            if reversed {
                points.reverse();
            }
            loop {
                let tip = *points.last().unwrap();
                let Some(candidates) = adjacency.get(&key(tip)) else {
                    break;
                };
                let Some(&next) = candidates.iter().find(|&&i| !used[i]) else {
                    break;
                };
                used[next] = true;
                let (na, nb) = segments[next];
                let far = if key(na) == key(tip) { nb } else { na };
                if key(far) == key(points[0]) {
                    closed = true;
                    break;
                }
                points.push(far);
            }
        }
        chains.push((points, closed));
    }
    chains
}

/// Minimum spacing of drawn aura polyline vertices, px — comfortably above the glow stroke
/// width so the per-segment screen quads read as one continuous stroke.
const AURA_DRAW_STEP_PX: f32 = 7.0;

/// Keep only points at least `step_px` apart (plus both endpoints), so the drawn stroke's
/// segments stay longer than the stroke width.
fn resample_polyline(points: &[egui::Pos2], step_px: f32) -> Vec<egui::Pos2> {
    let Some((&first, rest)) = points.split_first() else {
        return Vec::new();
    };
    let mut out = vec![first];
    let mut since = 0.0;
    for pair in points.windows(2) {
        since += (pair[1] - pair[0]).length();
        if since >= step_px {
            out.push(pair[1]);
            since = 0.0;
        }
    }
    if let Some(&last) = rest.last() {
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

/// One or more rounds of Chaikin corner-cutting. Closed chains smooth across the wrap; open
/// chains keep their endpoints (so an occlusion-clipped aura still ends where it was cut).
/// A closed result gets its first point repeated at the end, ready for `windows(2)` drawing.
fn chaikin_smooth(points: &[egui::Pos2], closed: bool, iterations: usize) -> Vec<egui::Pos2> {
    let mut pts = points.to_vec();
    for _ in 0..iterations {
        if pts.len() < 3 {
            break;
        }
        let mut next = Vec::with_capacity(pts.len() * 2);
        if !closed {
            next.push(pts[0]);
        }
        let edges = if closed { pts.len() } else { pts.len() - 1 };
        for i in 0..edges {
            let p = pts[i];
            let q = pts[(i + 1) % pts.len()];
            next.push(p + (q - p) * 0.25);
            next.push(p + (q - p) * 0.75);
        }
        if !closed {
            next.push(*pts.last().unwrap());
        }
        pts = next;
    }
    if closed {
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
    }
    pts
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
            self.push_line_segment(origin, tip, stroke_color, stroke, cam, viewport, view_proj);
            // The handle drags along both normal directions: one cone each way,
            // slightly offset from the handle disc.
            for sign in [1.0f32, -1.0] {
                push_gizmo_cone(
                    self,
                    tip,
                    n * sign,
                    GIZMO_CONE_GAP_PX,
                    GIZMO_CONE_LENGTH_PX,
                    GIZMO_CONE_RADIUS_PX,
                    stroke_color,
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
        // The angle handle drags along the circle both ways: one cone along each
        // tangent direction, slightly offset from the handle disc.
        for sign in [-1.0f32, 1.0] {
            push_gizmo_cone(
                self,
                handle,
                tangent * sign,
                GIZMO_ANGLE_CONE_GAP_PX,
                GIZMO_ANGLE_CONE_LENGTH_PX,
                GIZMO_ANGLE_CONE_RADIUS_PX,
                angle_color,
                project,
            );
        }
    }
}

/// Solid direction cone for a gizmo handle: base circle at `handle + dir * gap`, apex at
/// `handle + dir * (gap + length)`, pointing away from the handle along a direction it can
/// be dragged. Sized in screen px (converted to world units at the handle) and lightly
/// Lambert-shaded per triangle so it reads as a small solid, not a flat silhouette.
fn push_gizmo_cone(
    mesh: &mut SceneMesh<'_>,
    handle: Vec3,
    dir_world: Vec3,
    gap_px: f32,
    length_px: f32,
    radius_px: f32,
    color: Color32,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let dir = dir_world.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return;
    }
    let gap = pixels_to_world_distance(project, handle, dir, gap_px);
    let length = pixels_to_world_distance(project, handle, dir, length_px);
    if length < 1e-6 {
        return;
    }
    let mut u = dir.cross(Vec3::Z);
    if u.length_squared() < 1e-6 {
        u = dir.cross(Vec3::X);
    }
    let u = u.normalize_or_zero();
    let v = dir.cross(u);
    let base_center = handle + dir * gap;
    let apex = base_center + dir * length;
    let radius = pixels_to_world_distance(project, base_center, u, radius_px);
    if radius < 1e-6 {
        return;
    }
    // Same fixed light as `push_solid`, so gizmo cones match the scene's shading.
    let light = Vec3::new(0.35, 0.45, 0.82).normalize_or_zero();
    const SEGMENTS: usize = 16;
    let cap_shade = 0.4 + 0.6 * dir.dot(light).abs();
    let cap_color = scale_color(color, cap_shade);
    for i in 0..SEGMENTS {
        let a0 = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let p0 = base_center + (u * a0.cos() + v * a0.sin()) * radius;
        let p1 = base_center + (u * a1.cos() + v * a1.sin()) * radius;
        let normal = (p1 - p0).cross(apex - p0).normalize_or_zero();
        let shade = 0.4 + 0.6 * normal.dot(light).abs();
        mesh.push_triangle(p0, p1, apex, scale_color(color, shade));
        mesh.push_triangle(base_center, p1, p0, cap_color);
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
    let Some(sa) = cam.project(a, viewport, view_proj) else {
        return None;
    };
    let Some(sb) = cam.project(b, viewport, view_proj) else {
        return None;
    };
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 7.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
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
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: Some(ViewportHoverHighlight::SketchFace(
                FaceId::ConstructionPlane(0),
            )),
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
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_cut_body: None,
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,

            plane_gizmo: Some(gizmo),
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 10.0,
            y0: 0.0,
            x1: 5.0,
            y1: 8.0,
            bezier: None,
        });
        state.apply(crate::actions::Action::CreateLineSegment {
            x0: 5.0,
            y0: 8.0,
            x1: 0.0,
            y1: 0.0,
            bezier: None,
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
        });
        state.apply(crate::actions::Action::CommitRectangle);
        state.creating_circle = Some(crate::actions::CreatingCircle {
            origin: glam::Vec3::new(40.0, 25.0, 0.0),
            text: "40".into(),
            last_mouse: glam::Vec3::new(60.0, 25.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
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
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_cut_body: None,
                editing_extrusion: editing,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
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
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 8.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
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
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 6.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: Some(preview.clone()),
            editing_extrusion: None,
            preview_cut_body: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: Some(ViewportHoverHighlight::SketchFace(FaceId::Polygon(vec![
                0, 1, 2, 3,
            ]))),
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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

    /// Test-sized aura field over a 100x100 px viewport at cell size [`AURA_CELL_PX`].
    fn test_aura_field() -> AuraField {
        AuraField::new(UiRect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0)))
            .expect("field")
    }

    /// Stamp an axis-aligned screen-space rectangle at a fixed depth.
    fn stamp_rect(field: &mut AuraField, min: egui::Pos2, max: egui::Pos2, depth: f32, selected: bool) {
        let (a, b, c, d) = (
            min,
            egui::pos2(max.x, min.y),
            max,
            egui::pos2(min.x, max.y),
        );
        field.stamp_triangle([a, b, c], [depth; 3], selected);
        field.stamp_triangle([a, c, d], [depth; 3], selected);
    }

    /// #145: the aura contour rings the selected footprint at the offset distance — every
    /// contour point sits close to `BODY_SILHOUETTE_OFFSET_PX` outside the stamped square,
    /// none on its edges.
    #[test]
    fn aura_contour_rings_the_footprint_at_the_offset() {
        let mut field = test_aura_field();
        stamp_rect(&mut field, egui::pos2(40.0, 40.0), egui::pos2(60.0, 60.0), 100.0, true);
        field.finalize();
        let contour = field.visible_contour(BODY_SILHOUETTE_OFFSET_PX);
        assert!(!contour.is_empty(), "a selected square must produce a contour");
        for (a, b) in &contour {
            for p in [a, b] {
                // Signed distance of `p` to the square (positive outside).
                let dx = (40.0 - p.x).max(p.x - 60.0).max(0.0);
                let dy = (40.0 - p.y).max(p.y - 60.0).max(0.0);
                let dist = (dx * dx + dy * dy).sqrt();
                assert!(
                    dist > BODY_SILHOUETTE_OFFSET_PX * 0.5 && dist < BODY_SILHOUETTE_OFFSET_PX * 1.6,
                    "contour point should sit ~offset px outside the square, got {dist} at {p:?}"
                );
            }
        }
    }

    /// #145: two selected squares closer than twice the offset merge into one aura — no
    /// contour line passes through the gap between them.
    #[test]
    fn aura_joins_across_a_small_gap_between_selected_bodies() {
        let mut field = test_aura_field();
        // 4 px gap (x 48..52) between two squares; offset 5 px each side spans the gap.
        stamp_rect(&mut field, egui::pos2(28.0, 40.0), egui::pos2(48.0, 60.0), 100.0, true);
        stamp_rect(&mut field, egui::pos2(52.0, 40.0), egui::pos2(72.0, 60.0), 100.0, true);
        field.finalize();
        let contour = field.visible_contour(BODY_SILHOUETTE_OFFSET_PX);
        assert!(!contour.is_empty());
        for (a, b) in &contour {
            for p in [a, b] {
                assert!(
                    !(p.x > 48.5 && p.x < 51.5 && p.y > 42.0 && p.y < 58.0),
                    "auras should join across the gap — no contour inside it, got {p:?}"
                );
            }
        }
        // Sanity: the joined aura still exists above the gap (one shared outline).
        assert!(
            contour.iter().any(|(a, _)| a.x > 45.0 && a.x < 55.0 && a.y < 40.0),
            "joined outline should bridge over the gap"
        );
    }

    /// #145: a non-selected body in *front* of the silhouette occludes the aura there; the
    /// same body behind it does not.
    #[test]
    fn aura_occluded_only_by_nearer_unselected_bodies() {
        for (occluder_depth, expect_left_side) in [(50.0, false), (200.0, true)] {
            let mut field = test_aura_field();
            stamp_rect(&mut field, egui::pos2(40.0, 40.0), egui::pos2(60.0, 60.0), 100.0, true);
            // Non-selected slab covering the square's left flank (where the left aura runs).
            stamp_rect(
                &mut field,
                egui::pos2(20.0, 20.0),
                egui::pos2(45.0, 80.0),
                occluder_depth,
                false,
            );
            field.finalize();
            let contour = field.visible_contour(BODY_SILHOUETTE_OFFSET_PX);
            let has_left = contour
                .iter()
                .any(|(a, b)| a.x < 38.0 && b.x < 38.0 && a.y > 42.0 && a.y < 58.0);
            assert_eq!(
                has_left, expect_left_side,
                "occluder at depth {occluder_depth}: left aura present = {has_left}"
            );
            // The right flank is never covered by the occluder — always present.
            assert!(
                contour.iter().any(|(a, b)| a.x > 62.0 && b.x > 62.0),
                "right aura must survive regardless of the occluder"
            );
        }
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
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_cut_body: None,
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: hover,
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
        assert!(
            count_indices_with_color(
                &hovered,
                &hovered.wireframe_indices,
                crate::construction::PICK_HOVER_RGBA
            ) > count_indices_with_color(
                &base,
                &base.wireframe_indices,
                crate::construction::PICK_HOVER_RGBA
            ),
            "a pane-hovered body must draw its aura in the hover color"
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
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_cut_body: None,
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
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

    /// #145: selecting a body draws a glowing blue silhouette outline in the overlay layer.
    #[test]
    fn selected_body_draws_blue_silhouette_outline() {
        use crate::model::{Body, BodySource, ExtrudeFace, FaceId};
        use crate::selection::SceneSelection;

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        let rect = crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        state.doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            deleted: false,
            edge_treatments: Vec::new(),
        });
        state.doc.bodies.push(Body {
            source: BodySource::Extrusion(0),
            name: None,
            deleted: false,
        });

        let palette = ViewportPalette::default();
        let cam = state.cam.clone();
        let viewport = test_viewport();
        let mut selected = SceneSelection::default();
        crate::selection::click_scene_selection(&mut selected, SceneElement::Body(0), false);

        let build = |sel: &SceneSelection| {
            ViewportScene::build(&ViewportSceneInput {
                doc: &state.doc,
                cam: &cam,
                viewport,
                palette,
                sketch_session: None,
                selection: sel,
                element_visibility: &state.element_visibility,
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                preview_cut_body: None,
                editing_extrusion: None,
                plane_preview: None,
                active_sketch_face: None,
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
                hover_color: Color32::WHITE,
                document_health: &DocumentHealth::default(),
                constraint_graphics: None,
                constraint_connector_color: None,
            })
        };

        let unselected = build(&SceneSelection::default());
        let selected_scene = build(&selected);

        assert_eq!(
            count_indices_with_color(&unselected, &unselected.wireframe_indices, BODY_SILHOUETTE_COLOR),
            0,
            "no silhouette without a selected body"
        );
        // The aura draws in the depth-test-disabled (wireframe) layer so the ground grid and
        // other nearer geometry can't clip it — its occlusion is decided CPU-side instead.
        assert!(
            count_indices_with_color(&selected_scene, &selected_scene.wireframe_indices, BODY_SILHOUETTE_COLOR) > 0,
            "selected body should draw a blue silhouette outline in the depth-disabled layer"
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            &cam,
            viewport,
            &vp,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: &state.element_visibility,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: std::slice::from_ref(&dim_label),
            dim_label_view: Some(view),

            plane_gizmo: None,

            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
            element_visibility: scene_fields.5,
            preview_rect: None,
            preview_line: None,
            preview_circle: None,
            preview_extrusion: None,
            preview_cut_body: None,
            editing_extrusion: None,
            plane_preview: None,
            active_sketch_face: None,
            dimension_labels: &[],
            dim_label_view: None,
            plane_gizmo: None,
            extrude_gizmo: None,
            vertex_treatment_gizmo: None,
            vertex_treatment_preview: None,
            hover_highlight: None,
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
                preview_rect: None,
                preview_line: None,
                preview_circle: None,
                preview_extrusion: None,
                editing_extrusion: None,
                preview_cut_body: None,
                plane_preview: None,
                active_sketch_face: None,
                palette: ViewportPalette::default(),
                dimension_labels: &[],
                dim_label_view: None,
                plane_gizmo: None,
                extrude_gizmo: None,
                vertex_treatment_gizmo: None,
                vertex_treatment_preview: None,
                hover_highlight: None,
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
