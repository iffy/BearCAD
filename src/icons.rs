//! Toolbar and pane icons rasterized from bundled SVG assets.

use crate::geometric_constraints::GeometricConstraintType;
use crate::model::ConstraintKind;
use eframe::egui::{
    self, Color32, ColorImage, Context, Id, Painter, Rect, TextureHandle, TextureOptions, Ui,
    WidgetText,
};
use std::collections::HashMap;

pub const ICON_DISPLAY_SIZE: f32 = 18.0;
/// Combine Mode-row boolean icons (#1568): a step up from pane icons so the two-circle
/// glyphs stay readable, without matching the workbench toolbar.
pub const COMBINE_MODE_ICON_SIZE: f32 = 24.0;
const ICON_RASTER_SIZE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconId {
    Select,
    Rectangle,
    Line,
    Circle,
    Dimension,
    Constraint,
    Plane,
    Face,
    Lock,
    Parallel,
    Perpendicular,
    Equal,
    Coincident,
    Midpoint,
    Vertical,
    Horizontal,
    Home,
    Perspective,
    Orthographic,
    Sketch,
    Extrude,
    Loft,
    Revolve,
    /// An assembly (#923): parts joined into one thing.
    Assembly,
    /// The Create Shape tool's shapes (#909).
    ShapeCuboid,
    ShapeCylinder,
    ShapeSphere,
    Sweep,
    Combine,
    Move,
    Mirror,
    /// Rectangle anchor-mode radio icons (#532).
    RectCorner,
    RectCenter,
    CircleCenter,
    CircleEdge,
    Repeat,
    Offset,
    Slice,
    /// The Shell tool (#1156): hollow a body to a wall thickness.
    Shell,
    /// The Joint tool (#894): two links sharing a pin.
    Joint,
    /// Per-kind joint icons (#899), drawn in the pane and at the joint's frame in 3D.
    JointRigid,
    JointSlider,
    JointRevolute,
    JointCylindrical,
    JointPlanar,
    JointBall,
    JointPinSlot,
    JointScrew,
    Text,
    Body,
    Component,
    AutoZoom,
    ShadowBody,
    Plus,
    Showing,
    Hidden,
    Chamfer,
    Fillet,
    Drawing,
    Gear,
    ShadingWireframe,
    ShadingTransparentSolid,
    ShadingSolid,
    ShadingSolidWireframe,
    ShadingRealistic,
    GroundGrid,
    GroundSolid,
    ViewList,
    ViewGraph,
    /// Extrude body-mode picker icons (#35).
    NewBody,
    AddToBody,
    CutBody,
    /// Checkmark on the blue **primary button** that commits a tool's action (#586).
    Confirm,
    /// The Project tool (#593): an old-school film projector.
    Project,
    /// Small "remove" ✕ used by the element picker's row-remove button (#256).
    Close,
    /// Boolean-operation icons for the Combine tool (#267): two circles with kept regions
    /// solid and removed regions in faint red.
    BooleanUnion,
    BooleanCut,
    BooleanIntersect,
    BooleanDifference,
    /// Zoom-to-fit toolbar action (#279).
    Zoom,
    /// A body/sketch projection placed on a technical drawing (#281).
    Projection,
    /// Repeat tool distance/gap measurement toggles (#257).
    RepeatDistEnd,
    RepeatDistStart,
    RepeatGapBetween,
    RepeatGapOffset,
    /// Extrude taper measure toggles (#1243): length-per-side vs draft angle.
    TaperDistance,
    TaperAngle,
    /// Revolve Angle/Revolutions measure toggles (#1246): degrees vs turn count.
    RevolveAngle,
    RevolveRevolutions,
    /// Repeat tool along-the-path / around-the-axis toggle (#839).
    RepeatAlongPath,
    RepeatAroundAxis,
    /// Elements-pane filter funnel (#275/#291).
    Filter,
    /// Drawing workbench "Back to the 3D model" toolbar action (#325): a left arrow SVG (never
    /// a font glyph, which renders as an empty box on some platforms).
    Back,
    /// Export toolbar action (#348): an arrow leaving a tray; opens an SVG/PDF picker.
    Export,
    /// Import toolbar action (#352): an arrow dropping into a tray; opens an STL/STEP/Image picker.
    Import,
    /// A tracing image (#389): a framed picture.
    Image,
    /// A sketch's child components (#389): the sketch pencil with a tree of child geometry.
    SketchComponents,
    /// A drawing's child components (#389): the drawing sheet with a tree of child elements.
    DrawingComponents,
    /// Tutorials launcher (#1254): a graduation cap on the status-bar button.
    GraduationCap,
}

impl IconId {
    #[cfg(test)]
    pub const ALL: [Self; 97] = [
        Self::Select,
        Self::Rectangle,
        Self::Line,
        Self::Circle,
        Self::Dimension,
        Self::Constraint,
        Self::Plane,
        Self::Face,
        Self::Lock,
        Self::Parallel,
        Self::Perpendicular,
        Self::Equal,
        Self::Coincident,
        Self::Midpoint,
        Self::Vertical,
        Self::Horizontal,
        Self::Home,
        Self::Perspective,
        Self::Orthographic,
        Self::Sketch,
        Self::Extrude,
        Self::Loft,
        Self::Revolve,
        Self::Assembly,
        Self::ShapeCuboid,
        Self::ShapeCylinder,
        Self::ShapeSphere,
        Self::Sweep,
        Self::Combine,
        Self::Move,
        Self::Mirror,
        Self::RectCorner,
        Self::RectCenter,
        Self::CircleCenter,
        Self::CircleEdge,
        Self::Repeat,
        Self::Offset,
        Self::Slice,
        Self::Shell,
        Self::Joint,
        Self::JointRigid,
        Self::JointSlider,
        Self::JointRevolute,
        Self::JointCylindrical,
        Self::JointPlanar,
        Self::JointBall,
        Self::JointPinSlot,
        Self::JointScrew,
        Self::Text,
        Self::ShadowBody,
        Self::Body,
        Self::Plus,
        Self::Showing,
        Self::Hidden,
        Self::Chamfer,
        Self::Fillet,
        Self::Drawing,
        Self::Gear,
        Self::ShadingWireframe,
        Self::ShadingTransparentSolid,
        Self::ShadingSolid,
        Self::ShadingSolidWireframe,
        Self::ShadingRealistic,
        Self::GroundGrid,
        Self::GroundSolid,
        Self::ViewList,
        Self::ViewGraph,
        Self::NewBody,
        Self::AddToBody,
        Self::CutBody,
        Self::Confirm,
        Self::Project,
        Self::Close,
        Self::BooleanUnion,
        Self::BooleanCut,
        Self::BooleanIntersect,
        Self::BooleanDifference,
        Self::Zoom,
        Self::Projection,
        Self::RepeatDistEnd,
        Self::RepeatDistStart,
        Self::RepeatGapBetween,
        Self::RepeatGapOffset,
        Self::TaperDistance,
        Self::TaperAngle,
        Self::RevolveAngle,
        Self::RevolveRevolutions,
        Self::RepeatAlongPath,
        Self::RepeatAroundAxis,
        Self::Filter,
        Self::Back,
        Self::Export,
        Self::Import,
        Self::Image,
        Self::SketchComponents,
        Self::DrawingComponents,
        Self::GraduationCap,
    ];

    pub fn svg_source(self) -> &'static str {
        match self {
            Self::Select => include_str!("assets/icons/select.svg"),
            Self::Rectangle => include_str!("assets/icons/rectangle.svg"),
            Self::Line => include_str!("assets/icons/line.svg"),
            Self::Circle => include_str!("assets/icons/circle.svg"),
            Self::Dimension => include_str!("assets/icons/dimension.svg"),
            Self::Constraint => include_str!("assets/icons/constraint.svg"),
            Self::Plane => include_str!("assets/icons/plane.svg"),
            Self::Face => include_str!("assets/icons/face.svg"),
            Self::Lock => include_str!("assets/icons/lock.svg"),
            Self::Parallel => include_str!("assets/icons/parallel.svg"),
            Self::Perpendicular => include_str!("assets/icons/perpendicular.svg"),
            Self::Equal => include_str!("assets/icons/equal.svg"),
            Self::Coincident => include_str!("assets/icons/coincident.svg"),
            Self::Midpoint => include_str!("assets/icons/midpoint.svg"),
            Self::Vertical => include_str!("assets/icons/vertical.svg"),
            Self::Horizontal => include_str!("assets/icons/horizontal.svg"),
            Self::Home => include_str!("assets/icons/home.svg"),
            Self::Perspective => include_str!("assets/icons/perspective.svg"),
            Self::Orthographic => include_str!("assets/icons/orthographic.svg"),
            Self::Sketch => include_str!("assets/icons/sketch.svg"),
            Self::Extrude => include_str!("assets/icons/extrude.svg"),
            Self::Loft => include_str!("assets/icons/loft.svg"),
            Self::Revolve => include_str!("assets/icons/revolve.svg"),
            Self::Assembly => include_str!("assets/icons/assembly.svg"),
            Self::ShapeCuboid => include_str!("assets/icons/shape_cuboid.svg"),
            Self::ShapeCylinder => include_str!("assets/icons/shape_cylinder.svg"),
            Self::ShapeSphere => include_str!("assets/icons/shape_sphere.svg"),
            Self::Sweep => include_str!("assets/icons/sweep.svg"),
            Self::Combine => include_str!("assets/icons/combine.svg"),
            Self::Move => include_str!("assets/icons/move.svg"),
            Self::Mirror => include_str!("assets/icons/mirror.svg"),
            Self::RectCorner => include_str!("assets/icons/rect_corner.svg"),
            Self::RectCenter => include_str!("assets/icons/rect_center.svg"),
            Self::CircleCenter => include_str!("assets/icons/circle_center.svg"),
            Self::CircleEdge => include_str!("assets/icons/circle_edge.svg"),
            Self::Repeat => include_str!("assets/icons/repeat.svg"),
            Self::Offset => include_str!("assets/icons/offset.svg"),
            Self::Slice => include_str!("assets/icons/slice.svg"),
            Self::Shell => include_str!("assets/icons/shell.svg"),
            Self::Joint => include_str!("assets/icons/joint.svg"),
            Self::JointRigid => include_str!("assets/icons/joint_rigid.svg"),
            Self::JointSlider => include_str!("assets/icons/joint_slider.svg"),
            Self::JointRevolute => include_str!("assets/icons/joint_revolute.svg"),
            Self::JointCylindrical => include_str!("assets/icons/joint_cylindrical.svg"),
            Self::JointPlanar => include_str!("assets/icons/joint_planar.svg"),
            Self::JointBall => include_str!("assets/icons/joint_ball.svg"),
            Self::JointPinSlot => include_str!("assets/icons/joint_pin_slot.svg"),
            Self::JointScrew => include_str!("assets/icons/joint_screw.svg"),
            Self::Text => include_str!("assets/icons/text.svg"),
            Self::ShadowBody => include_str!("assets/icons/shadow_body.svg"),
            Self::Body => include_str!("assets/icons/body.svg"),
            Self::Component => include_str!("assets/icons/component.svg"),
            Self::AutoZoom => include_str!("assets/icons/auto_zoom.svg"),
            Self::Plus => include_str!("assets/icons/plus.svg"),
            Self::Showing => include_str!("assets/icons/showing.svg"),
            Self::Hidden => include_str!("assets/icons/hidden.svg"),
            Self::Chamfer => include_str!("assets/icons/chamfer.svg"),
            Self::Fillet => include_str!("assets/icons/fillet.svg"),
            Self::Drawing => include_str!("assets/icons/drawing.svg"),
            Self::Gear => include_str!("assets/icons/gear.svg"),
            Self::ShadingWireframe => include_str!("assets/icons/wireframe.svg"),
            Self::ShadingTransparentSolid => include_str!("assets/icons/transparent_solid.svg"),
            Self::ShadingSolid => include_str!("assets/icons/solid.svg"),
            Self::ShadingSolidWireframe => include_str!("assets/icons/solid_wireframe.svg"),
            Self::ShadingRealistic => include_str!("assets/icons/realistic.svg"),
            Self::GroundGrid => include_str!("assets/icons/ground_grid.svg"),
            Self::GroundSolid => include_str!("assets/icons/ground_solid.svg"),
            Self::ViewList => include_str!("assets/icons/view_list.svg"),
            Self::ViewGraph => include_str!("assets/icons/view_graph.svg"),
            Self::NewBody => include_str!("assets/icons/new_body.svg"),
            Self::AddToBody => include_str!("assets/icons/add_to_body.svg"),
            Self::CutBody => include_str!("assets/icons/cut_body.svg"),
            Self::Confirm => include_str!("assets/icons/confirm.svg"),
            Self::Project => include_str!("assets/icons/project.svg"),
            Self::Close => include_str!("assets/icons/x.svg"),
            Self::BooleanUnion => include_str!("assets/icons/boolean_union.svg"),
            Self::BooleanCut => include_str!("assets/icons/boolean_cut.svg"),
            Self::BooleanIntersect => include_str!("assets/icons/boolean_intersect.svg"),
            Self::BooleanDifference => include_str!("assets/icons/boolean_difference.svg"),
            Self::Zoom => include_str!("assets/icons/zoom.svg"),
            Self::Projection => include_str!("assets/icons/projection.svg"),
            Self::RepeatDistEnd => include_str!("assets/icons/repeat_dist_end.svg"),
            Self::RepeatDistStart => include_str!("assets/icons/repeat_dist_start.svg"),
            Self::RepeatGapBetween => include_str!("assets/icons/repeat_gap_between.svg"),
            Self::RepeatGapOffset => include_str!("assets/icons/repeat_gap_offset.svg"),
            Self::TaperDistance => include_str!("assets/icons/taper_distance.svg"),
            Self::TaperAngle => include_str!("assets/icons/taper_angle.svg"),
            Self::RevolveAngle => include_str!("assets/icons/revolve_angle.svg"),
            Self::RevolveRevolutions => include_str!("assets/icons/revolve_revolutions.svg"),
            Self::RepeatAlongPath => include_str!("assets/icons/repeat_along_path.svg"),
            Self::RepeatAroundAxis => include_str!("assets/icons/repeat_around_axis.svg"),
            Self::Filter => include_str!("assets/icons/filter.svg"),
            Self::Back => include_str!("assets/icons/back.svg"),
            Self::Export => include_str!("assets/icons/export.svg"),
            Self::Import => include_str!("assets/icons/import.svg"),
            Self::Image => include_str!("assets/icons/image.svg"),
            Self::SketchComponents => include_str!("assets/icons/sketch-components.svg"),
            Self::DrawingComponents => include_str!("assets/icons/drawing-components.svg"),
            Self::GraduationCap => include_str!("assets/icons/graduation_cap.svg"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Rectangle => "Rectangle",
            Self::Line => "Line",
            Self::Circle => "Circle",
            Self::Dimension => "Dimension",
            Self::Constraint => "Constraint",
            Self::Plane => "Plane",
            Self::Face => "Face",
            Self::Lock => "Lock",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Equal => "Equal",
            Self::Coincident => "Coincident",
            Self::Midpoint => "Midpoint",
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::Home => "Home",
            Self::Perspective => "Perspective",
            Self::Orthographic => "Orthographic",
            Self::Sketch => "Sketch",
            Self::Extrude => "Extrude",
            Self::Loft => "Loft",
            Self::Revolve => "Revolve",
            Self::Assembly => "Assembly",
            Self::ShapeCuboid => "Cuboid",
            Self::ShapeCylinder => "Cylinder",
            Self::ShapeSphere => "Sphere",
            Self::Sweep => "Sweep",
            Self::Combine => "Combine",
            Self::Move => "Move",
            Self::Mirror => "Mirror",
            Self::RectCorner => "Corner-anchored",
            Self::RectCenter => "Centre-anchored",
            Self::CircleCenter => "Centre + radius",
            Self::CircleEdge => "Edge to opposite edge",
            Self::Repeat => "Repeat",
            Self::Offset => "Offset",
            Self::Slice => "Slice",
            Self::Shell => "Shell",
            Self::Joint => "Joint",
            Self::JointRigid => "Rigid joint",
            Self::JointSlider => "Slider joint",
            Self::JointRevolute => "Revolute joint",
            Self::JointCylindrical => "Cylindrical joint",
            Self::JointPlanar => "Planar joint",
            Self::JointBall => "Ball joint",
            Self::JointPinSlot => "Pin-slot joint",
            Self::JointScrew => "Screw joint",
            Self::Text => "Text",
            Self::ShadowBody => "Shadow body",
            Self::Body => "Body",
            Self::Component => "Component",
            Self::AutoZoom => "Auto-zoom",
            Self::Plus => "Plus",
            Self::Showing => "Showing",
            Self::Hidden => "Hidden",
            Self::Chamfer => "Chamfer",
            Self::Fillet => "Fillet",
            Self::Drawing => "Drawing",
            Self::Gear => "Gear",
            Self::ShadingWireframe => "Wireframe",
            Self::ShadingTransparentSolid => "Transparent solid",
            Self::ShadingSolid => "Solid",
            Self::ShadingSolidWireframe => "Solid + wireframe",
            Self::ShadingRealistic => "Realistic",
            Self::GroundGrid => "Ground grid",
            Self::GroundSolid => "Solid ground",
            Self::ViewList => "List view",
            Self::ViewGraph => "Graph view",
            Self::NewBody => "New body",
            Self::AddToBody => "Add to body",
            Self::CutBody => "Cut body",
            Self::Confirm => "Confirm",
            Self::Project => "Project",
            Self::Close => "Close",
            Self::BooleanUnion => "Combine",
            Self::BooleanCut => "Cut",
            Self::BooleanIntersect => "Intersect",
            Self::BooleanDifference => "Difference",
            Self::Zoom => "Zoom to fit",
            Self::Projection => "Projection",
            Self::RepeatDistEnd => "Distance to end",
            Self::RepeatDistStart => "Distance to start",
            Self::RepeatGapBetween => "Gap between",
            Self::RepeatGapOffset => "Start-to-start offset",
            Self::TaperDistance => "Taper by distance",
            Self::TaperAngle => "Taper by angle",
            Self::RevolveAngle => "Angle in degrees",
            Self::RevolveRevolutions => "Revs (turns)",
            Self::RepeatAlongPath => "Along the path",
            Self::RepeatAroundAxis => "Around the path",
            Self::Filter => "Filter",
            Self::Back => "Back",
            Self::Export => "Export",
            Self::Import => "Import",
            Self::Image => "Image",
            Self::SketchComponents => "Sketch components",
            Self::DrawingComponents => "Drawing components",
            Self::GraduationCap => "Graduation cap",
        }
    }
}

/// The per-kind joint icon (#899).
pub fn icon_for_joint_kind(kind: &crate::model::JointKind) -> IconId {
    match kind {
        crate::model::JointKind::Rigid => IconId::JointRigid,
        crate::model::JointKind::Slider => IconId::JointSlider,
        crate::model::JointKind::Revolute => IconId::JointRevolute,
        crate::model::JointKind::Cylindrical => IconId::JointCylindrical,
        crate::model::JointKind::Planar => IconId::JointPlanar,
        crate::model::JointKind::Ball => IconId::JointBall,
        crate::model::JointKind::PinSlot => IconId::JointPinSlot,
        crate::model::JointKind::Screw { .. } => IconId::JointScrew,
    }
}

pub fn icon_for_visibility(visible: bool) -> IconId {
    if visible {
        IconId::Showing
    } else {
        IconId::Hidden
    }
}

pub fn icon_for_projection_mode(mode: crate::camera::ProjectionMode) -> IconId {
    match mode {
        crate::camera::ProjectionMode::Natural => IconId::Perspective,
        crate::camera::ProjectionMode::Orthographic => IconId::Orthographic,
    }
}

pub fn icon_for_shading_mode(mode: crate::camera::ShadingMode) -> IconId {
    match mode {
        crate::camera::ShadingMode::Wireframe => IconId::ShadingWireframe,
        crate::camera::ShadingMode::TransparentSolid => IconId::ShadingTransparentSolid,
        crate::camera::ShadingMode::Solid => IconId::ShadingSolid,
        crate::camera::ShadingMode::SolidWireframe => IconId::ShadingSolidWireframe,
        crate::camera::ShadingMode::Realistic => IconId::ShadingRealistic,
    }
}

pub fn icon_for_ground_display(mode: crate::camera::GroundDisplay) -> IconId {
    match mode {
        crate::camera::GroundDisplay::Grid => IconId::GroundGrid,
        crate::camera::GroundDisplay::Solid => IconId::GroundSolid,
        crate::camera::GroundDisplay::None => IconId::Hidden,
    }
}

pub fn icon_for_constraint(kind: GeometricConstraintType) -> IconId {
    match kind {
        GeometricConstraintType::Parallel => IconId::Parallel,
        GeometricConstraintType::Perpendicular => IconId::Perpendicular,
        GeometricConstraintType::Equal => IconId::Equal,
        GeometricConstraintType::Coincident => IconId::Coincident,
        GeometricConstraintType::Midpoint => IconId::Midpoint,
        GeometricConstraintType::AlongXAxis => IconId::Horizontal,
        GeometricConstraintType::AlongYAxis => IconId::Vertical,
    }
}

pub fn icon_for_constraint_kind(kind: &ConstraintKind) -> IconId {
    match kind {
        ConstraintKind::Distance { .. } => IconId::Dimension,
        // A line constrained parallel to a sketch axis is the axis-based Horizontal/Vertical
        // (#577/#580): show the horizontal/vertical glyph, which depicts exactly that; a plain
        // line-to-line parallel keeps the parallel glyph.
        ConstraintKind::Parallel { line_a, line_b } => {
            let axis = |l: &crate::model::ConstraintLine| match l {
                crate::model::ConstraintLine::OriginAxis(a) => Some(*a),
                _ => None,
            };
            match axis(line_a).or_else(|| axis(line_b)) {
                Some(crate::model::SketchAxis::X) => IconId::Horizontal,
                Some(crate::model::SketchAxis::Y) => IconId::Vertical,
                None => IconId::Parallel,
            }
        }
        ConstraintKind::Perpendicular { .. } => IconId::Perpendicular,
        ConstraintKind::Equal { .. } => IconId::Equal,
        ConstraintKind::Coincident { .. } => IconId::Coincident,
        ConstraintKind::Midpoint { .. } => IconId::Midpoint,
        ConstraintKind::Angle { .. } => IconId::Constraint,
        ConstraintKind::Tangent { .. } => IconId::Coincident,
    }
}

fn rasterize_svg(svg: &str, size: u32) -> ColorImage {
    let svg = svg.replace("currentColor", "#ffffff");
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("valid svg");
    let mut pixmap =
        tiny_skia::Pixmap::new(size, size).expect("pixmap allocation should succeed");
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let svg_size = tree.size();
    let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let pixels = pixmap
        .pixels()
        .iter()
        .map(|pixel| {
            Color32::from_rgba_unmultiplied(pixel.red(), pixel.green(), pixel.blue(), pixel.alpha())
        })
        .collect();

    ColorImage {
        size: [size as usize, size as usize],
        pixels,
        ..Default::default()
    }
}

fn texture_for_icon(ctx: &Context, id: IconId) -> egui::TextureId {
    let cache_id = Id::new("icon_textures");
    let mut cache = ctx
        .data(|d| d.get_temp::<HashMap<IconId, TextureHandle>>(cache_id))
        .unwrap_or_default();

    if let Some(handle) = cache.get(&id) {
        return handle.id();
    }

    let image = rasterize_svg(id.svg_source(), ICON_RASTER_SIZE);
    let handle = ctx.load_texture(
        format!("icon_{}", id.label()),
        image,
        TextureOptions::LINEAR,
    );
    let texture_id = handle.id();
    cache.insert(id, handle);
    ctx.data_mut(|d| d.insert_temp(cache_id, cache));
    texture_id
}

pub fn sized_texture(ctx: &Context, id: IconId) -> egui::load::SizedTexture {
    sized_texture_at(ctx, id, ICON_DISPLAY_SIZE)
}

pub fn sized_texture_at(ctx: &Context, id: IconId, size: f32) -> egui::load::SizedTexture {
    egui::load::SizedTexture::new(texture_for_icon(ctx, id), egui::vec2(size, size))
}

/// Draw an icon inline in a row, at the pane's usual icon size (#921).
pub fn show_icon(ui: &mut Ui, id: IconId) {
    ui.add(egui::Image::new(sized_texture_at(ui.ctx(), id, 16.0)));
}

pub fn paint_icon(painter: &Painter, ctx: &Context, id: IconId, rect: Rect, tint: Color32) {
    let texture_id = texture_for_icon(ctx, id);
    painter.image(
        texture_id,
        rect,
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        tint,
    );
}

/// A selectable menu/combo row with its icon **before** the label (#999).
///
/// Laid out as one widget rather than painting the glyph over a space-indented string: two
/// spaces are about seven pixels and the icon is fourteen, so the glyph landed on the first
/// letter of every entry — and any font or scale change would have moved the collision around.
pub fn selectable_icon_label(
    ui: &mut Ui,
    id: IconId,
    selected: bool,
    label: impl Into<WidgetText>,
) -> egui::Response {
    let image = egui::Image::new(sized_texture_at(ui.ctx(), id, 14.0));
    ui.add(
        egui::Button::image_and_text(image, label)
            .frame(false)
            .selected(selected),
    )
}

pub fn selectable_icon_button(
    ui: &mut Ui,
    id: IconId,
    selected: bool,
    tooltip: impl Into<WidgetText>,
) -> egui::Response {
    selectable_icon_button_at(ui, id, selected, tooltip, ICON_DISPLAY_SIZE)
}

/// [`selectable_icon_button`] at an explicit icon size (the workbench toolbar runs
/// larger than pane icons, #461).
pub fn selectable_icon_button_at(
    ui: &mut Ui,
    id: IconId,
    selected: bool,
    tooltip: impl Into<WidgetText>,
    size: f32,
) -> egui::Response {
    let response = ui.add(
        egui::Button::new(egui::Image::new(sized_texture_at(ui.ctx(), id, size)))
            .frame(true)
            .selected(selected),
    );
    response.on_hover_text(tooltip)
}

/// A toggle button showing a **group of icons** (#382) — the Elements-pane filter renders one
/// per element category (e.g. Extrude+Revolve+Combine for "Operations"). Draws like a
/// selectable `ImageButton`; the icons dim while the toggle is off.
pub fn selectable_icon_group(
    ui: &mut Ui,
    icons: &[IconId],
    selected: bool,
    tooltip: impl Into<WidgetText>,
) -> egui::Response {
    let pad = 4.0;
    let gap = 2.0;
    let n = icons.len() as f32;
    let size = egui::vec2(
        pad * 2.0 + n * ICON_DISPLAY_SIZE + (n - 1.0).max(0.0) * gap,
        pad * 2.0 + ICON_DISPLAY_SIZE,
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let tint = if selected {
            Color32::WHITE
    } else {
        Color32::from_gray(110)
    };
    let ctx = ui.ctx().clone();
        for (i, icon) in icons.iter().enumerate() {
            let x = rect.min.x + pad + i as f32 * (ICON_DISPLAY_SIZE + gap);
            let r = Rect::from_min_size(
                egui::pos2(x, rect.min.y + pad),
                egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE),
            );
            paint_icon(ui.painter(), &ctx, *icon, r, tint);
        }
    }
    response.on_hover_text(tooltip)
}

/// An icon button that tints **gold on hover** (#440), signalling it's clickable/toggleable.
pub fn icon_button_hover_gold(
    ui: &mut Ui,
    id: IconId,
    tooltip: impl Into<WidgetText>,
) -> egui::Response {
    let texture = sized_texture(ui.ctx(), id);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE), egui::Sense::click());
    let tint = if response.hovered() {
        egui::Color32::from_rgb(255, 210, 90)
    } else {
        egui::Color32::WHITE
    };
    egui::Image::new(texture).tint(tint).paint_at(ui, rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip)
}

/// An icon button drawn in an explicit `tint`, brightening to `hover_tint` while hovered
/// (#642) — for icons whose *color* carries the state, like the Repeat tool's green
/// computed-value lock against its two grey siblings.
pub fn tinted_icon_button(
    ui: &mut Ui,
    id: IconId,
    tint: Color32,
    hover_tint: Color32,
    tooltip: impl Into<WidgetText>,
) -> egui::Response {
    let texture = sized_texture(ui.ctx(), id);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE), egui::Sense::click());
    let tint = if response.hovered() { hover_tint } else { tint };
    egui::Image::new(texture).tint(tint).paint_at(ui, rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip)
}

pub fn icon_button(ui: &mut Ui, id: IconId, tooltip: impl Into<WidgetText>) -> egui::Response {
    ui.add(
        egui::Button::new(egui::Image::new(sized_texture(ui.ctx(), id))).frame(false),
    )
    .on_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use super::*;

    #[test]
    fn all_icons_rasterize_with_visible_pixels() {
        for id in IconId::ALL {
            let image = rasterize_svg(id.svg_source(), ICON_RASTER_SIZE);
            assert_eq!(image.size, [ICON_RASTER_SIZE as usize, ICON_RASTER_SIZE as usize]);
            assert!(
                image.pixels.iter().any(|pixel| pixel.a() > 0),
                "{} should rasterize visible pixels",
                id.label()
            );
        }
    }

    /// #267: the boolean-op icons that carve something away actually render red pixels for the
    /// removed region (union keeps everything, so it has none).
    #[test]
    fn boolean_op_icons_render_their_removed_region_in_red() {
        // Red-dominant pixels; the removed regions are faint (low opacity), so a modest
        // dominance margin distinguishes them from the neutral kept fills.
        let has_red = |id: IconId| {
            rasterize_svg(id.svg_source(), ICON_RASTER_SIZE)
                .pixels
                .iter()
                .any(|p| p.a() > 0 && p.r() as i32 > p.g() as i32 + 15 && p.r() as i32 > p.b() as i32 + 15)
        };
        assert!(has_red(IconId::BooleanCut), "cut shows removed region in red");
        assert!(has_red(IconId::BooleanIntersect), "intersect shows removed region in red");
        assert!(has_red(IconId::BooleanDifference), "difference shows removed lens in red");
        assert!(!has_red(IconId::BooleanUnion), "union keeps everything, no red");
    }

    #[test]
    fn hud_icons_map_to_projection_modes() {
        use crate::camera::ProjectionMode;

        assert_eq!(
            icon_for_projection_mode(ProjectionMode::Natural),
            IconId::Perspective
        );
        assert_eq!(
            icon_for_projection_mode(ProjectionMode::Orthographic),
            IconId::Orthographic
        );
    }

    #[test]
    fn hud_icons_map_to_shading_modes() {
        use crate::camera::ShadingMode;

        assert_eq!(
            icon_for_shading_mode(ShadingMode::Wireframe),
            IconId::ShadingWireframe
        );
        assert_eq!(
            icon_for_shading_mode(ShadingMode::TransparentSolid),
            IconId::ShadingTransparentSolid
        );
        assert_eq!(
            icon_for_shading_mode(ShadingMode::Solid),
            IconId::ShadingSolid
        );
        assert_eq!(
            icon_for_shading_mode(ShadingMode::SolidWireframe),
            IconId::ShadingSolidWireframe
        );
        assert_eq!(
            icon_for_shading_mode(ShadingMode::Realistic),
            IconId::ShadingRealistic
        );
    }

    #[test]
    fn visibility_icons_reflect_state() {
        assert_eq!(icon_for_visibility(true), IconId::Showing);
        assert_eq!(icon_for_visibility(false), IconId::Hidden);
    }

    #[test]
    fn constraint_icons_map_to_expected_assets() {
        assert_eq!(
            icon_for_constraint(GeometricConstraintType::Parallel),
            IconId::Parallel
        );
        assert_eq!(
            icon_for_constraint(GeometricConstraintType::Perpendicular),
            IconId::Perpendicular
        );
        assert_eq!(
            icon_for_constraint(GeometricConstraintType::Coincident),
            IconId::Coincident
        );
        assert_eq!(
            icon_for_constraint(GeometricConstraintType::Midpoint),
            IconId::Midpoint
        );
    }

    #[test]
    fn stored_constraint_kinds_map_to_expected_icons() {
        use crate::model::{
            ConstraintEntity, ConstraintLine, ConstraintPoint, DistanceTarget, LineEnd,
        };

        assert_eq!(
            icon_for_constraint_kind(&ConstraintKind::Distance {
                target: DistanceTarget::LineLength(lkey(0)),
            }),
            IconId::Dimension
        );
        assert_eq!(
            icon_for_constraint_kind(&ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(0)),
                line_b: ConstraintLine::Line(lkey(1)),
            }),
            IconId::Parallel
        );
        assert_eq!(
            icon_for_constraint_kind(&ConstraintKind::Angle {
                line_a: ConstraintLine::Line(lkey(0)),
                line_b: ConstraintLine::Line(lkey(1)),
                rotation_sign: 1,
            }),
            IconId::Constraint
        );
        assert_eq!(
            icon_for_constraint_kind(&ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(0),
                    end: LineEnd::Start,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(1),
                    end: LineEnd::End,
                }),
            }),
            IconId::Coincident
        );
    }
}
