//! Technical drawings (#180): view projection and vector export, independent of the egui
//! drawing pane so it can be unit-tested and reused for print/PDF output. A drawing renders
//! black-on-white to either **SVG** (prints to PDF via any browser/OS print dialog) or a
//! direct single-page **PDF** — both drive the identical layout through the [`Canvas`] trait,
//! so the two exports never drift.

use crate::model::{Document, DrawingOrientation, DrawingView};
use glam::Vec3;

/// In-plane `(right, up)` world axes a drawing view projects onto: a point `p` maps to
/// `(p·right, p·up)`. The six orthographic directions plus a standard isometric view.
pub fn view_axes(orientation: DrawingOrientation) -> (Vec3, Vec3) {
    use DrawingOrientation as O;
    match orientation {
        O::Front => (Vec3::X, Vec3::Z),
        O::Back => (-Vec3::X, Vec3::Z),
        O::Right => (Vec3::Y, Vec3::Z),
        O::Left => (-Vec3::Y, Vec3::Z),
        // Looking down from above, world +Y (the back) runs up the page; from below it runs
        // down it (#1643). `right × up` is the direction out of the page toward the eye, so
        // these must be +Z for Top and −Z for Bottom — the same directions the bear reports.
        O::Top => (Vec3::X, Vec3::Y),
        O::Bottom => (Vec3::X, -Vec3::Y),
        O::Isometric => {
            let out = Vec3::new(1.0, 1.0, 1.0).normalize();
            let right = Vec3::Z.cross(out).normalize();
            let up = out.cross(right).normalize();
            (right, up)
        }
        // A diagonal edge view (#339): the camera looks along the average of its two faces'
        // into-page directions (the 45° bisector), with world +Z as up (Gram-Schmidt'd square to
        // that direction — no cube edge points straight up, so this is always well-defined).
        O::Edge(e) => {
            let (fa, fb) = e.faces();
            let (ra, ua) = view_axes(fa);
            let (rb, ub) = view_axes(fb);
            let out = (ra.cross(ua) + rb.cross(ub)).normalize();
            let up = (Vec3::Z - Vec3::Z.dot(out) * out).normalize();
            let right = up.cross(out).normalize();
            (right, up)
        }
        // A corner three-quarter view (#344): the camera looks along the average of its three
        // faces' into-page directions (the corner's diagonal), world +Z up. No corner points
        // straight up, so the Gram-Schmidt up is always well-defined.
        O::Corner(c) => {
            let (fa, fb, fc) = c.faces();
            let face_out = |f| {
                let (r, u) = view_axes(f);
                r.cross(u)
            };
            let out = (face_out(fa) + face_out(fb) + face_out(fc)).normalize();
            let up = (Vec3::Z - Vec3::Z.dot(out) * out).normalize();
            let right = up.cross(out).normalize();
            (right, up)
        }
        // A free (arbitrary) angle (#345): use the stored basis directly, re-orthonormalised
        // defensively so a slightly-off basis still projects cleanly.
        O::Free { right, up } => {
            let r = Vec3::from_array(right).normalize_or(Vec3::X);
            let u0 = Vec3::from_array(up).normalize_or(Vec3::Z);
            let out = r.cross(u0).normalize_or(Vec3::Y);
            let u = out.cross(r).normalize_or(Vec3::Z);
            (r, u)
        }
    }
}

fn dequant(q: [i32; 3]) -> Vec3 {
    Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) / 100.0
}

/// The orientation of an aligned child projection (#296) placed in `dir` relative to a parent
/// showing `parent`. Derived by the glass-box unfolding: the child shares one of the parent's
/// screen axes and rotates 90° about it. `None` if the result isn't one of the six orthographic
/// views (e.g. the parent is Isometric), so alignment is offered only for orthographic parents.
pub fn aligned_child_orientation(
    parent: DrawingOrientation,
    dir: crate::model::AlignDir,
) -> Option<DrawingOrientation> {
    use crate::model::AlignDir;
    // Only the six straight-on views unfold into aligned children; iso/edge/corner parents don't.
    if !can_be_aligned_base(parent) {
        return None;
    }
    let (r, u) = view_axes(parent);
    let o = r.cross(u); // out of the page, toward the eye, for this view basis
    // Third-angle unfolding: roll the glass box about the shared screen axis. The view placed
    // above shows what the eye sees after climbing over the top (up becomes the eye direction).
    let (cr, cu) = match dir {
        AlignDir::Below => (r, o),
        AlignDir::Above => (r, -o),
        AlignDir::Right => (-o, u),
        AlignDir::Left => (o, u),
    };
    // The unfolded child may be a *rotated* face view (e.g. a Top base's Left/Right children),
    // which isn't an axis-aligned canonical orientation (#351). Its true rotated basis comes from
    // `resolved_view_axes`; here we just pick the nearest face by view direction for its label, so
    // all four directions are offerable rather than only the ones that happen to stay canonical.
    orientation_from_axes(cr, cu).or_else(|| nearest_face_by_view_dir(cr.cross(cu)))
}

/// Whether a projection can be the base of an aligned child (#296/#1225): only the six
/// straight-on faces unfold into orthographic neighbours; iso/edge/corner/free cannot.
pub fn can_be_aligned_base(orientation: DrawingOrientation) -> bool {
    matches!(
        orientation,
        DrawingOrientation::Front
            | DrawingOrientation::Back
            | DrawingOrientation::Left
            | DrawingOrientation::Right
            | DrawingOrientation::Top
            | DrawingOrientation::Bottom
    )
}

/// Labels on a projection card's right-click menu (#1225). Orientation is changed in the
/// context pane (navigation bear), not from a long dump of every view here. Orthographic
/// cards offer **Create aligned view** (arms the Aligned-view tool with this card as base);
/// every card can still be Removed.
pub fn projection_card_context_actions(orientation: DrawingOrientation) -> Vec<&'static str> {
    let mut out = Vec::new();
    if can_be_aligned_base(orientation) {
        out.push("Create aligned view");
    }
    out.push("Remove");
    out
}

/// The orthographic orientations an aligned child may take while staying **in line** with its
/// base (#332): rotating the view about the screen axis the two share keeps the alignment intact.
/// A horizontally-placed child (Left/Right) shares the parent's vertical (up) axis, so it can be
/// any view whose up axis matches the parent's (Front/Back/Left/Right for a Front parent); a
/// vertically-placed child (Above/Below) shares the horizontal (right) axis. The parent's own
/// derived child orientation is always included. Empty for a non-orthographic (Isometric) parent.
pub fn aligned_inline_orientations(
    parent: DrawingOrientation,
    dir: crate::model::AlignDir,
) -> Vec<DrawingOrientation> {
    let (pr, pu) = view_axes(parent);
    // Which parent screen axis the child shares depends on the drag direction.
    let shared = if dir.shares_pos_x() { pr } else { pu };
    let axis_matches = |a: Vec3, b: Vec3| a.dot(b).abs() > 0.9;
    // The ring the child can rotate through about the shared edge (#367): the straight-on faces
    // *and* the diagonal edge views that keep that shared axis (front-right-edge, right-back-edge,
    // …), excluding the base's own orientation. Anything involving the perpendicular pole (top or
    // bottom for a left/right ring) is filtered out because its axis no longer matches.
    let candidates = DrawingOrientation::ALL
        .iter()
        .copied()
        .filter(|o| !matches!(o, DrawingOrientation::Isometric))
        .chain(crate::model::EdgeView::ALL.iter().map(|e| DrawingOrientation::Edge(*e)));
    candidates
        .filter(|o| *o != parent)
        .filter(|o| {
            let (r, u) = view_axes(*o);
            if dir.shares_pos_x() {
                axis_matches(r, shared)
            } else {
                axis_matches(u, shared)
            }
        })
        .collect()
}

/// The projection basis `(right, up)` a view actually renders with (#357). A non-aligned view uses
/// `view_axes(orientation)`; an **aligned child** uses the glass-box **unfolding** of its parent's
/// basis about their shared screen axis, so it stays lined up *and correctly rotated* for any base
/// orientation — e.g. a Top base yields Front below, Back above, and rotated Left/Right to the
/// sides (#351). `views` is the drawing's view list; the parent is looked up by `aligned_parent`
/// (recursively, so chains stay consistent).
pub fn resolved_view_axes(views: &[DrawingView], view: &DrawingView) -> (Vec3, Vec3) {
    use crate::model::AlignDir;
    if let (Some(p), Some(dir)) = (view.aligned_parent, view.aligned_dir) {
        if let Some(parent) = views.get(p) {
            if !std::ptr::eq(parent, view) {
                let (pr, pu) = resolved_view_axes(views, parent);
                let po = pr.cross(pu); // out of the parent's page, toward the eye
                // Same third-angle roll as `aligned_child_orientation` (#1643).
                let (r0, u0) = match dir {
                    AlignDir::Below => (pr, po),
                    AlignDir::Above => (pr, -po),
                    AlignDir::Right => (-po, pu),
                    AlignDir::Left => (po, pu),
                };
                // Render the user's chosen ring orientation in the parent's *unfolded* frame (#367):
                // `unfold` is the rotation that carries the default orientation's canonical basis
                // onto the unfolded basis (r0, u0); applying it to the chosen orientation's basis
                // keeps the child lined up while showing the new angle. No pick → default → identity.
                if let Some(default) = aligned_child_orientation(parent.orientation, dir) {
                    if view.orientation != default {
                        let (dr, du) = view_axes(default);
                        let canon = glam::Mat3::from_cols(dr, du, dr.cross(du));
                        let unfolded = glam::Mat3::from_cols(r0, u0, r0.cross(u0));
                        let rot = unfolded * canon.transpose();
                        let (cr, cu) = view_axes(view.orientation);
                        return ((rot * cr).normalize_or_zero(), (rot * cu).normalize_or_zero());
                    }
                }
                return (r0, u0);
            }
        }
    }
    view_axes(view.orientation)
}

/// The on-page position of a view (#296), resolving an aligned child's shared axis to its
/// parent's so the two always line up regardless of which was dragged. Non-aligned views (and
/// children whose parent is gone) return their own stored `(pos_x, pos_y)`.
pub fn resolved_view_pos(doc: &Document, drawing: crate::model::DrawingKey, view: usize) -> (f32, f32) {
    let Some(d) = doc.drawings.get(drawing) else {
        return (0.5, 0.5);
    };
    let Some(v) = d.views.get(view) else {
        return (0.5, 0.5);
    };
    match (v.aligned_parent, v.aligned_dir) {
        (Some(p), Some(dir)) if p != view => {
            if let Some(parent) = d.views.get(p) {
                // Resolve the parent recursively so chains of aligned views stay consistent.
                let (px, py) = resolved_view_pos(doc, drawing, p);
                let _ = parent;
                if dir.shares_pos_x() {
                    return (px, v.pos_y);
                } else {
                    return (v.pos_x, py);
                }
            }
            (v.pos_x, v.pos_y)
        }
        _ => (v.pos_x, v.pos_y),
    }
}

/// A view's effective print scale (#296/#300): an aligned child inherits its parent's scale
/// (walking the chain), so a whole aligned group prints at one scale. Non-aligned views use
/// their own `scale`.
pub fn resolved_view_scale(doc: &Document, drawing: crate::model::DrawingKey, view: usize) -> Option<String> {
    let d = doc.drawings.get(drawing)?;
    let v = d.views.get(view)?;
    match v.aligned_parent {
        Some(p) if p != view && d.views.get(p).is_some() => {
            resolved_view_scale(doc, drawing, p)
        }
        _ => v.scale.clone(),
    }
}

/// A view's stored card size as page fractions, floored to the minimum (#1207).
pub fn view_size_frac(view: &DrawingView) -> (f32, f32) {
    (
        view.size_x.max(crate::model::MIN_VIEW_SIZE_FRAC),
        view.size_y.max(crate::model::MIN_VIEW_SIZE_FRAC),
    )
}

/// Clamp a page-fraction card size into the allowed range (#1207).
pub fn clamp_view_size_frac(size: f32) -> f32 {
    size.clamp(crate::model::MIN_VIEW_SIZE_FRAC, 1.0)
}

/// Views that share **width** (`size_x`) with `view` via Above/Below alignment links (#1207).
/// The graph is undirected so resizing a child updates its parent and siblings.
pub fn views_sharing_size_x(views: &[DrawingView], view: usize) -> Vec<usize> {
    views_sharing_size_axis(views, view, /* share_x */ true)
}

/// Views that share **height** (`size_y`) with `view` via Left/Right alignment links (#1207).
pub fn views_sharing_size_y(views: &[DrawingView], view: usize) -> Vec<usize> {
    views_sharing_size_axis(views, view, /* share_x */ false)
}

fn views_sharing_size_axis(views: &[DrawingView], view: usize, share_x: bool) -> Vec<usize> {
    if view >= views.len() {
        return Vec::new();
    }
    let n = views.len();
    let mut adj = vec![Vec::new(); n];
    for (i, v) in views.iter().enumerate() {
        if let (Some(p), Some(dir)) = (v.aligned_parent, v.aligned_dir) {
            if p < n && p != i && dir.shares_pos_x() == share_x {
                adj[i].push(p);
                adj[p].push(i);
            }
        }
    }
    let mut seen = vec![false; n];
    let mut stack = vec![view];
    let mut out = Vec::new();
    seen[view] = true;
    while let Some(i) = stack.pop() {
        out.push(i);
        for &j in &adj[i] {
            if !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Write `size_x`/`size_y` onto `view` and every linked view on each axis (#1207).
/// Width propagates across Above/Below links; height across Left/Right.
pub fn apply_view_size(views: &mut [DrawingView], view: usize, size_x: f32, size_y: f32) {
    if view >= views.len() {
        return;
    }
    let sx = clamp_view_size_frac(size_x);
    let sy = clamp_view_size_frac(size_y);
    let share_x = views_sharing_size_x(views, view);
    let share_y = views_sharing_size_y(views, view);
    for i in share_x {
        views[i].size_x = sx;
    }
    for i in share_y {
        views[i].size_y = sy;
    }
}

/// The root of an aligned scale chain (#364/#1207): walk `aligned_parent` until free.
pub fn aligned_scale_root(views: &[DrawingView], view: usize) -> usize {
    let mut root = view;
    let mut guard = 0;
    while guard < views.len() {
        guard += 1;
        let Some(v) = views.get(root) else {
            break;
        };
        match v.aligned_parent {
            Some(p) if p != root && views.get(p).is_some() => root = p,
            _ => break,
        }
    }
    root
}

/// Card corner positions in page-local coordinates for a view centred at `center` with
/// size `(cell_w, cell_h)` — TL, TR, BR, BL (#1207).
pub fn view_card_corners(center: glam::Vec2, cell_w: f32, cell_h: f32) -> [glam::Vec2; 4] {
    let hx = cell_w * 0.5;
    let hy = cell_h * 0.5;
    [
        glam::Vec2::new(center.x - hx, center.y - hy), // TL
        glam::Vec2::new(center.x + hx, center.y - hy), // TR
        glam::Vec2::new(center.x + hx, center.y + hy), // BR
        glam::Vec2::new(center.x - hx, center.y + hy), // BL
    ]
}

/// Which corner grip (0=TL…3=BL) of a card is under `pointer`, if any (#1207).
pub fn view_resize_handle_hit(
    pointer: glam::Vec2,
    corners: &[glam::Vec2; 4],
    radius: f32,
) -> Option<usize> {
    let r2 = radius * radius;
    corners
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let d2 = (*c - pointer).length_squared();
            (d2 <= r2).then_some((d2, i))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, i)| i)
}

/// New card size (page fractions) when a corner grip is dragged, keeping the card centre
/// fixed (#1207). `page` is the page size in the same units as `pointer`/`center`.
pub fn view_size_from_corner_drag(
    center: glam::Vec2,
    pointer: glam::Vec2,
    page: glam::Vec2,
) -> (f32, f32) {
    let half = (pointer - center).abs();
    let sx = clamp_view_size_frac(2.0 * half.x / page.x.max(1e-3));
    let sy = clamp_view_size_frac(2.0 * half.y / page.y.max(1e-3));
    (sx, sy)
}

/// Whether a view card shows its border and Delete ✕ chrome (#1229).
/// Idle cards are bare; selection, pointer hover, aligned-view parent highlight, and
/// element-picker row hover each turn the chrome on.
pub fn view_card_chrome_active(
    selected: bool,
    hovered: bool,
    align_parent: bool,
    picker_hover: bool,
) -> bool {
    selected || hovered || align_parent || picker_hover
}

/// The projected `(right, up)` bounding box of a view's geometry, or `None` if it has none.
fn view_projected_bbox(
    doc: &Document,
    views: &[DrawingView],
    view: usize,
) -> Option<(glam::Vec2, glam::Vec2)> {
    let v = views.get(view)?;
    let world_edges = drawing_view_dimensionable_edges(doc, views, v);
    if world_edges.is_empty() {
        return None;
    }
    let (right, up) = resolved_view_axes(views, v);
    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (a, b) in &world_edges {
        for p in [a, b] {
            let pr = glam::Vec2::new(p.dot(right), p.dot(up));
            min = min.min(pr);
            max = max.max(pr);
        }
    }
    Some((min, max))
}

/// A view's own projected extent (size), floored to a tiny positive value per axis.
fn view_projected_extent(doc: &Document, views: &[DrawingView], view: usize) -> glam::Vec2 {
    match view_projected_bbox(doc, views, view) {
        Some((min, max)) => (max - min).max(glam::Vec2::splat(1e-3)),
        None => glam::Vec2::splat(1.0),
    }
}

/// Auto-fit scale for a view within an `area_w`×`area_h` card, filling `fit` of it. An aligned
/// child inherits its parent's auto-fit scale (walking to the aligned root) so a whole aligned
/// group renders at one size — a prerequisite for their edges lining up (#364).
pub fn view_autofit_scale(
    doc: &Document,
    views: &[DrawingView],
    view: usize,
    area_w: f32,
    area_h: f32,
    fit: f32,
) -> f32 {
    if let Some(v) = views.get(view) {
        if let Some(p) = v.aligned_parent {
            if p != view && views.get(p).is_some() {
                return view_autofit_scale(doc, views, p, area_w, area_h, fit);
            }
        }
    }
    let e = view_projected_extent(doc, views, view);
    (area_w / e.x).min(area_h / e.y) * fit
}

/// The bbox center to render a view's geometry about. An aligned child adopts its parent's center
/// along their **shared** projected axis (horizontal for above/below, vertical for left/right) so
/// the part's edges line up across the aligned group, not just the view cards (#364).
pub fn view_render_center(doc: &Document, views: &[DrawingView], view: usize) -> glam::Vec2 {
    let (min, max) =
        view_projected_bbox(doc, views, view).unwrap_or((glam::Vec2::ZERO, glam::Vec2::ZERO));
    let mut center = (min + max) * 0.5;
    if let Some(v) = views.get(view) {
        if let (Some(p), Some(dir)) = (v.aligned_parent, v.aligned_dir) {
            if p != view && views.get(p).is_some() {
                let parent_center = view_render_center(doc, views, p);
                if dir.shares_pos_x() {
                    center.x = parent_center.x;
                } else {
                    center.y = parent_center.y;
                }
            }
        }
    }
    center
}

/// Projected 2D endpoints of a view's dimensionable edges (creases + silhouettes).
fn view_projected_points(
    doc: &Document,
    views: &[DrawingView],
    view: usize,
) -> Option<Vec<glam::Vec2>> {
    let v = views.get(view)?;
    let world_edges = drawing_view_dimensionable_edges(doc, views, v);
    if world_edges.is_empty() {
        return None;
    }
    let (right, up) = resolved_view_axes(views, v);
    let mut pts = Vec::with_capacity(world_edges.len() * 2);
    for (a, b) in &world_edges {
        pts.push(glam::Vec2::new(a.dot(right), a.dot(up)));
        pts.push(glam::Vec2::new(b.dot(right), b.dot(up)));
    }
    Some(pts)
}

/// Silhouette extreme on the shared axis, with the facing extreme of the perpendicular among
/// projected points that sit at that shared extreme (#1206). AABB corners float above
/// irregular silhouettes (edge views, multi-body); this lands on the body edge itself.
///
/// - `shared_is_x`: shared axis is X (above/below children) vs Y (left/right).
/// - `at_min_shared`: left/bottom extreme vs right/top.
/// - `want_min_perp`: facing direction along the perpendicular (min = bottom/left face).
fn silhouette_facing_point(
    pts: &[glam::Vec2],
    shared_is_x: bool,
    at_min_shared: bool,
    want_min_perp: bool,
) -> Option<glam::Vec2> {
    if pts.is_empty() {
        return None;
    }
    let shared = |p: glam::Vec2| if shared_is_x { p.x } else { p.y };
    let perp = |p: glam::Vec2| if shared_is_x { p.y } else { p.x };
    let mut s_min = f32::MAX;
    let mut s_max = f32::MIN;
    for p in pts {
        let s = shared(*p);
        s_min = s_min.min(s);
        s_max = s_max.max(s);
    }
    let extreme_s = if at_min_shared { s_min } else { s_max };
    // A hair of the shared-axis span so float noise and near-extreme tessellation still count.
    let eps = ((s_max - s_min).abs() * 1e-4).max(1e-3);
    let mut best_perp = if want_min_perp { f32::MAX } else { f32::MIN };
    let mut found = false;
    for p in pts {
        if (shared(*p) - extreme_s).abs() <= eps {
            let v = perp(*p);
            if want_min_perp {
                best_perp = best_perp.min(v);
            } else {
                best_perp = best_perp.max(v);
            }
            found = true;
        }
    }
    if !found {
        return None;
    }
    Some(if shared_is_x {
        glam::Vec2::new(extreme_s, best_perp)
    } else {
        glam::Vec2::new(best_perp, extreme_s)
    })
}

/// The two dashed projection lines connecting an aligned child to its base view (#377):
/// endpoints in each view's **own projected 2D space** — `(parent_point, child_point)` per
/// line — which the renderers map through the owning view's own to-device transform. The
/// lines sit at the silhouette extremes of the shared axis (far left/right for an
/// above/below child, top/bottom for a left/right one) and connect the **facing body edges**
/// of the two views (#1206) — not floating AABB corners. `None` if the view isn't a valid
/// aligned child or either view has no geometry.
pub fn aligned_projection_lines(
    doc: &Document,
    views: &[DrawingView],
    child: usize,
) -> Option<[(glam::Vec2, glam::Vec2); 2]> {
    use crate::model::AlignDir;
    let v = views.get(child)?;
    let (p, dir) = (v.aligned_parent?, v.aligned_dir?);
    if p == child {
        return None;
    }
    views.get(p)?;
    let ppts = view_projected_points(doc, views, p)?;
    let cpts = view_projected_points(doc, views, child)?;
    // Shared-axis coordinates coincide between the two views (#364); endpoints land on the
    // facing silhouette at each extreme so the dashed lines touch the body edge (#1206).
    Some(match dir {
        // Shared = X. Parent faces down (min y) / child faces up (max y), or the reverse.
        AlignDir::Below => [
            (
                silhouette_facing_point(&ppts, true, true, true)?,
                silhouette_facing_point(&cpts, true, true, false)?,
            ),
            (
                silhouette_facing_point(&ppts, true, false, true)?,
                silhouette_facing_point(&cpts, true, false, false)?,
            ),
        ],
        AlignDir::Above => [
            (
                silhouette_facing_point(&ppts, true, true, false)?,
                silhouette_facing_point(&cpts, true, true, true)?,
            ),
            (
                silhouette_facing_point(&ppts, true, false, false)?,
                silhouette_facing_point(&cpts, true, false, true)?,
            ),
        ],
        // Shared = Y. Parent faces right (max x) / child faces left (min x), or the reverse.
        AlignDir::Right => [
            (
                silhouette_facing_point(&ppts, false, true, false)?,
                silhouette_facing_point(&cpts, false, true, true)?,
            ),
            (
                silhouette_facing_point(&ppts, false, false, false)?,
                silhouette_facing_point(&cpts, false, false, true)?,
            ),
        ],
        AlignDir::Left => [
            (
                silhouette_facing_point(&ppts, false, true, true)?,
                silhouette_facing_point(&cpts, false, true, false)?,
            ),
            (
                silhouette_facing_point(&ppts, false, false, true)?,
                silhouette_facing_point(&cpts, false, false, false)?,
            ),
        ],
    })
}

/// Match a `(right, up)` axis pair back to one of the six orthographic [`DrawingOrientation`]s.
fn orientation_from_axes(right: Vec3, up: Vec3) -> Option<DrawingOrientation> {
    use DrawingOrientation as O;
    const ALL: [O; 6] = [O::Front, O::Back, O::Left, O::Right, O::Top, O::Bottom];
    ALL.into_iter().find(|&o| {
        let (r, u) = view_axes(o);
        (r - right).length() < 1e-3 && (u - up).length() < 1e-3
    })
}

/// The straight-on face whose view direction (into the page) best matches `view_dir` — used to
/// **label** a rotated aligned child (#351) by the face it looks at, when its unfolded basis isn't
/// an axis-aligned canonical orientation.
fn nearest_face_by_view_dir(view_dir: Vec3) -> Option<DrawingOrientation> {
    use DrawingOrientation as O;
    const ALL: [O; 6] = [O::Front, O::Back, O::Left, O::Right, O::Top, O::Bottom];
    ALL.into_iter().max_by(|&a, &b| {
        let dir = |o| {
            let (r, u) = view_axes(o);
            r.cross(u).dot(view_dir)
        };
        dir(a).partial_cmp(&dir(b)).unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn edge_key(a: Vec3, b: Vec3) -> crate::model::DrawingEdgeKey {
    crate::model::normalized_edge_key(
        crate::hierarchy::quantize_body_point(a),
        crate::hierarchy::quantize_body_point(b),
    )
}

/// A circle detected among a view's feature edges (#313), in **world** space so it can be
/// classified once and projected per view: a tessellated curve (cylinder rim, extruded-circle
/// boundary) drawn as one smooth circle (or a foreshortened line when edge-on) with a single
/// diameter dimension rather than a dimension per short segment.
#[derive(Clone, Copy, Debug)]
pub struct WorldCircle {
    pub center: Vec3,
    pub radius: f32,
    /// Unit normal of the circle's plane.
    pub normal: Vec3,
}

/// How a [`WorldCircle`] appears in a particular orthographic view (#313/#319).
#[derive(Debug)]
pub enum ProjectedCircle {
    /// The circle faces the viewer (roughly): a round outline.
    Round { center: glam::Vec2, radius: f32 },
    /// The circle is (near) edge-on: it projects to a line — the foreshortened diameter.
    EdgeOn { a: glam::Vec2, b: glam::Vec2 },
    /// The circle is seen at an angle (#1775): an ellipse, carried as its two semi-axis
    /// vectors from `center` — `major` is always the full radius, `minor` the foreshortened
    /// one, so the outline matches the body's silhouette instead of floating past it.
    Angled { center: glam::Vec2, major: glam::Vec2, minor: glam::Vec2 },
}

/// Classify a view's world feature edges (#313): find tessellated circles (clean degree-2
/// cycles that fit a planar circle) so the renderers can draw them smooth and dimension only
/// the diameter, in any orientation. Straight edges are everything else.
pub fn classify_world_circles(edges: &[(Vec3, Vec3)]) -> Vec<WorldCircle> {
    use std::collections::HashMap;
    // Quantize endpoints (0.01 mm) so shared vertices merge into one index.
    let q = |p: Vec3| {
        (
            (p.x * 100.0).round() as i64,
            (p.y * 100.0).round() as i64,
            (p.z * 100.0).round() as i64,
        )
    };
    let mut index_of: HashMap<(i64, i64, i64), usize> = HashMap::new();
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vid = |p: Vec3| {
        *index_of.entry(q(p)).or_insert_with(|| {
            verts.push(p);
            verts.len() - 1
        })
    };
    let mut e_verts: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
    let mut seen_pairs: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for &(a, b) in edges {
        let (ia, ib) = (vid(a), vid(b));
        let pair = if ia <= ib { (ia, ib) } else { (ib, ia) };
        if ia != ib && !seen_pairs.insert(pair) {
            continue;
        }
        e_verts.push((ia, ib));
    }
    let n = verts.len();
    let mut degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(a, b)) in e_verts.iter().enumerate() {
        if a == b {
            continue;
        }
        degree[a] += 1;
        degree[b] += 1;
        adj[a].push(ei);
        adj[b].push(ei);
    }
    let mut seen = vec![false; e_verts.len()];
    let mut circles = Vec::new();
    for start in 0..e_verts.len() {
        if seen[start] || e_verts[start].0 == e_verts[start].1 {
            continue;
        }
        let mut stack = vec![start];
        let mut comp_edges = Vec::new();
        let mut comp_verts: Vec<usize> = Vec::new();
        let mut clean = true;
        seen[start] = true;
        while let Some(ei) = stack.pop() {
            comp_edges.push(ei);
            for &v in [e_verts[ei].0, e_verts[ei].1].iter() {
                if degree[v] != 2 {
                    clean = false;
                }
                comp_verts.push(v);
                for &ne in &adj[v] {
                    if !seen[ne] {
                        seen[ne] = true;
                        stack.push(ne);
                    }
                }
            }
        }
        comp_verts.sort_unstable();
        comp_verts.dedup();
        if !clean || comp_edges.len() < 8 || comp_verts.len() != comp_edges.len() {
            continue;
        }
        let center =
            comp_verts.iter().map(|&v| verts[v]).sum::<Vec3>() / comp_verts.len() as f32;
        let radii: Vec<f32> = comp_verts.iter().map(|&v| (verts[v] - center).length()).collect();
        let mean_r = radii.iter().sum::<f32>() / radii.len() as f32;
        if mean_r < 1e-2 {
            continue;
        }
        let max_dev = radii.iter().map(|r| (r - mean_r).abs()).fold(0.0f32, f32::max);
        if max_dev > mean_r * 0.06 {
            continue;
        }
        // Plane normal from the strongest pair of centre-relative spokes — order-free, since
        // `comp_verts` is in vertex-index order (not loop order), and the index assignment
        // follows the caller's edge order. The sign is then **canonicalized** (#376): a rim's
        // facing is meaningless for rendering, but it decides which end of an edge-on
        // diameter line the label hangs past, and the editor and export each rebuild this
        // from their own pass — an arbitrary sign made the label jump sides between them.
        let spoke0 = verts[comp_verts[0]] - center;
        let mut normal = Vec3::ZERO;
        for &v in &comp_verts[1..] {
            let cand = spoke0.cross(verts[v] - center);
            if cand.length_squared() > normal.length_squared() {
                normal = cand;
            }
        }
        let mut normal = normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        let flip = normal.z < -1e-6
            || (normal.z.abs() <= 1e-6
                && (normal.y < -1e-6 || (normal.y.abs() <= 1e-6 && normal.x < 0.0)));
        if flip {
            normal = -normal;
        }
        // Require coplanarity (all vertices near the plane).
        let coplanar = comp_verts
            .iter()
            .all(|&v| (verts[v] - center).dot(normal).abs() <= mean_r * 0.06);
        if coplanar {
            circles.push(WorldCircle { center, radius: mean_r, normal });
        }
    }
    circles
}

/// Project a world circle into a view's 2D space (#313/#319): round when it faces the viewer,
/// a foreshortened line when edge-on. An orthographic projection turns a circle into an
/// ellipse whose **minor** semi-axis is `r·|n·d|` (`d` = view direction) and whose major is
/// always `r`, along the projection of `d × n` — computed from those, not by projecting two
/// arbitrary in-plane axes: at a 45° edge view both such axes foreshorten equally (~0.7·r),
/// which used to miss the edge-on case and draw a smaller round circle floating at each cap
/// of a side-viewed cylinder (#369).
pub fn project_world_circle(c: &WorldCircle, right: Vec3, up: Vec3) -> ProjectedCircle {
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let c2 = project(c.center);
    let d = right.cross(up).normalize_or_zero();
    let nd = c.normal.dot(d);
    if nd.abs() < 0.15 {
        // Edge-on: the major axis is the in-plane direction perpendicular to the view.
        let w = d.cross(c.normal).normalize_or_zero();
        let major = glam::Vec2::new(w.dot(right), w.dot(up)) * c.radius;
        ProjectedCircle::EdgeOn { a: c2 - major, b: c2 + major }
    } else if nd.abs() > 0.99 {
        ProjectedCircle::Round { center: c2, radius: c.radius }
    } else {
        // Angled (#1775): the ellipse's major semi-axis is the rim's horizon — the in-plane
        // direction perpendicular to the tilt — at the full radius; the minor one runs along
        // the tilt (the normal's own projection) foreshortened to r·|n·d|.
        let w = d.cross(c.normal).normalize_or_zero();
        let major = glam::Vec2::new(w.dot(right), w.dot(up)) * c.radius;
        let minor = project(c.normal).normalize_or_zero() * (c.radius * nd.abs());
        ProjectedCircle::Angled { center: c2, major, minor }
    }
}

/// The closed polyline tracing an angled circle's ellipse (#1775): `segments` points,
/// starting at the major semi-axis end and stepping round through the minor one. Everything
/// that strokes an angled circle — the editor, the export canvas, hover picking — samples
/// this so they all draw the same outline.
pub fn angled_circle_points(
    center: glam::Vec2,
    major: glam::Vec2,
    minor: glam::Vec2,
    segments: usize,
) -> Vec<glam::Vec2> {
    let segments = segments.max(4);
    (0..segments)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / segments as f32;
            center + major * t.cos() + minor * t.sin()
        })
        .collect()
}

/// How many chords a detected circle's rim is redrawn with before hidden-line removal
/// (#1841). The mesh tessellates a circle into 48; twice that keeps the arcs that survive
/// reading as a curve rather than a polygon.
pub const SMOOTH_CIRCLE_SEGMENTS: usize = 96;

/// A detected circle as a closed world-space polyline: `segments` chords, the last point
/// repeating the first so `windows(2)` walks the whole rim.
pub fn world_circle_points(c: &WorldCircle, segments: usize) -> Vec<Vec3> {
    let segments = segments.max(4);
    let u = if c.normal.cross(Vec3::Z).length_squared() > 1e-6 {
        c.normal.cross(Vec3::Z).normalize()
    } else {
        c.normal.cross(Vec3::X).normalize_or_zero()
    };
    let v = c.normal.cross(u).normalize_or_zero();
    (0..=segments)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / segments as f32;
            c.center + u * (c.radius * t.cos()) + v * (c.radius * t.sin())
        })
        .collect()
}

/// Whether a world segment is a chord of `c` — both ends on its rim, so it is part of the
/// tessellated polygon that circle was detected from.
pub fn world_segment_on_circle(a: Vec3, b: Vec3, c: &WorldCircle) -> bool {
    let tol = c.radius * 0.02 + 1e-3;
    let on = |p: Vec3| {
        let d = p - c.center;
        d.dot(c.normal).abs() < tol && (d.length() - c.radius).abs() < tol
    };
    on(a) && on(b)
}

/// One projected circle as the chords a renderer strokes it with: the ellipse (or round
/// outline) closed back on its first point, or the single foreshortened line edge-on.
pub fn projected_circle_chords(pc: &ProjectedCircle) -> Vec<(glam::Vec2, glam::Vec2)> {
    let closed = |pts: Vec<glam::Vec2>| {
        (0..pts.len()).map(|i| (pts[i], pts[(i + 1) % pts.len()])).collect()
    };
    match pc {
        ProjectedCircle::Round { center, radius } => closed(
            (0..48)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / 48.0;
                    *center + glam::Vec2::new(t.cos(), t.sin()) * *radius
                })
                .collect(),
        ),
        ProjectedCircle::EdgeOn { a, b } => vec![(*a, *b)],
        ProjectedCircle::Angled { center, major, minor } => {
            closed(angled_circle_points(*center, *major, *minor, 48))
        }
    }
}

/// The model lines a projection strokes, in view space — what the editor and the exports both
/// put on the page for the geometry itself (#1841). A wireframe view's detected circles come
/// through whole; every other style's rims are already in `styled.segments`, hidden where the
/// solid hides them.
pub fn view_stroked_lines(
    styled: &StyledViewGeometry,
    pcircles: &[ProjectedCircle],
    view: &DrawingView,
) -> Vec<(glam::Vec2, glam::Vec2)> {
    let whole = view_strokes_whole_circles(view);
    let mut out: Vec<(glam::Vec2, glam::Vec2)> = styled
        .segments
        .iter()
        .filter(|(a, b)| !(whole && projected_segment_on_circle(*a, *b, pcircles)))
        .copied()
        .collect();
    if whole {
        for pc in pcircles {
            out.extend(projected_circle_chords(pc));
        }
    }
    out
}

/// [`view_stroked_lines`] for one view of a document, working the styled geometry and the
/// detected circles out for itself. Scripts read this back with `bearcad.drawing_view_lines`.
pub fn drawing_view_lines(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> Vec<(glam::Vec2, glam::Vec2)> {
    let (right, up) = resolved_view_axes(views, view);
    let pcircles: Vec<ProjectedCircle> = classify_world_circles(&drawing_view_world_edges(doc, view))
        .iter()
        .map(|c| project_world_circle(c, right, up))
        .collect();
    view_stroked_lines(&styled_view_geometry(doc, views, view), &pcircles, view)
}

/// Whether this view strokes a detected circle's outline whole (#1841/#1842). A wireframe
/// (or sketch) view draws every edge, hidden ones included, so the smooth outline can go
/// down in one pass. Every other style hides what the solid covers, and a rim is no
/// exception — those views get the rim as the visible arcs in [`styled_view_geometry`]'s
/// segments instead.
pub fn view_strokes_whole_circles(view: &DrawingView) -> bool {
    view.sketch.is_some() || view.style == crate::model::DrawingViewStyle::Wireframe
}

/// Whether a projected 2D segment lies on one of the projected circles (#313), so it's drawn as
/// part of the smooth circle/edge-on line instead of a straight stroke or dimension.
pub fn projected_segment_on_circle(a: glam::Vec2, b: glam::Vec2, pcs: &[ProjectedCircle]) -> bool {
    pcs.iter().any(|pc| match pc {
        ProjectedCircle::Round { center, radius } => {
            let tol = radius * 0.08 + 1e-2;
            ((a - *center).length() - radius).abs() < tol
                && ((b - *center).length() - radius).abs() < tol
        }
        ProjectedCircle::EdgeOn { a: la, b: lb } => {
            let d = *lb - *la;
            let len2 = d.length_squared().max(1e-6);
            let tol = d.length() * 0.08 + 1e-2;
            let on = |p: glam::Vec2| {
                let t = ((p - *la).dot(d) / len2).clamp(0.0, 1.0);
                (p - (*la + d * t)).length() < tol
            };
            on(a) && on(b)
        }
        ProjectedCircle::Angled { center, major, minor } => {
            // Map into ellipse space: a point on the ellipse lands on the unit circle. The
            // approximate distance scales the radial miss by the smaller semi-axis, which is
            // plenty for the few-percent tolerance here.
            let m = glam::Mat2::from_cols(*major, *minor).inverse();
            let on = |p: glam::Vec2| {
                let q = m * (p - *center);
                let semi = major.length().min(minor.length());
                ((q.length() - 1.0) * semi).abs() < major.length() * 0.08 + 1e-2
            };
            on(a) && on(b)
        }
    })
}

/// PDF points per millimetre (1 pt = 1/72 in): exports are sized in points so the PDF page
/// physically matches the drawing's configured mm page (#298).
pub const PT_PER_MM: f32 = 72.0 / 25.4;
/// Default placed view card size as a fraction of the page (#297/#1207) — kept public so the
/// editor, export, and scripting agree. Per-view sizes live on each view's `size_x`/`size_y`;
/// this is only the historical default.
pub const CELL_FRAC: f32 = 0.42;
/// The magnification a zoom loupe draws at (#1846): the ratio of its two circles, so growing
/// the magnified circle magnifies rather than showing more of the part.
pub fn loupe_zoom(loupe: &crate::model::DrawingLoupe) -> f32 {
    if loupe.radius.abs() < 1e-6 {
        return 1.0;
    }
    loupe.to_radius / loupe.radius
}

/// A loupe's two centres, in the view's projected millimetres (#1846).
pub fn loupe_centers(loupe: &crate::model::DrawingLoupe) -> (glam::Vec2, glam::Vec2) {
    (
        glam::Vec2::new(loupe.at.0, loupe.at.1),
        glam::Vec2::new(loupe.to.0, loupe.to.1),
    )
}

/// The line joining a loupe's circles (#1846): the centre line trimmed back to each rim, so
/// it touches the edges rather than running through them. `None` when the circles overlap and
/// there is no gap to bridge.
pub fn loupe_connector(loupe: &crate::model::DrawingLoupe) -> Option<(glam::Vec2, glam::Vec2)> {
    let (c1, c2) = loupe_centers(loupe);
    let span = c2 - c1;
    let gap = span.length() - loupe.radius.abs() - loupe.to_radius.abs();
    if gap <= 1e-4 {
        return None;
    }
    let dir = span.normalize();
    Some((c1 + dir * loupe.radius.abs(), c2 - dir * loupe.to_radius.abs()))
}

/// The part of `(a, b)` inside the circle at `center` with radius `r`, or `None` when the
/// segment misses it entirely (#1846).
pub fn clip_segment_to_circle(
    a: glam::Vec2,
    b: glam::Vec2,
    center: glam::Vec2,
    r: f32,
) -> Option<(glam::Vec2, glam::Vec2)> {
    let d = b - a;
    let f = a - center;
    let aa = d.length_squared();
    if aa < 1e-12 {
        return ((a - center).length() <= r).then_some((a, b));
    }
    // |a + t·d − c|² = r², solved for the interval of `t` inside the circle.
    let bb = 2.0 * f.dot(d);
    let cc = f.length_squared() - r * r;
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let (t0, t1) = ((-bb - root) / (2.0 * aa), (-bb + root) / (2.0 * aa));
    let (lo, hi) = (t0.max(0.0), t1.min(1.0));
    (hi - lo > 1e-6).then(|| (a + d * lo, a + d * hi))
}

/// What a loupe's magnified circle draws inside itself (#1846): the view's own segments
/// clipped to the detail circle, then scaled by the zoom about that circle's centre and moved
/// onto the magnified one. Anything clear of the detail circle is dropped.
pub fn loupe_magnified_segments(
    loupe: &crate::model::DrawingLoupe,
    segments: &[(glam::Vec2, glam::Vec2)],
) -> Vec<(glam::Vec2, glam::Vec2)> {
    let (c1, c2) = loupe_centers(loupe);
    let zoom = loupe_zoom(loupe);
    let map = |p: glam::Vec2| c2 + (p - c1) * zoom;
    segments
        .iter()
        .filter_map(|(a, b)| clip_segment_to_circle(*a, *b, c1, loupe.radius.abs()))
        .map(|(a, b)| (map(a), map(b)))
        .collect()
}

/// Sutherland–Hodgman clip of a convex polygon to a circle (#1850), as a 48-gon inscribed
/// in it — a couple of thousandths of the radius under the true rim, which is well under a
/// stroke width at any page scale. Returns the (still convex) remainder, empty when the
/// polygon misses the circle.
pub fn clip_convex_to_circle(
    poly: &[glam::Vec2],
    center: glam::Vec2,
    r: f32,
) -> Vec<glam::Vec2> {
    const SIDES: usize = 48;
    let mut out: Vec<glam::Vec2> = poly.to_vec();
    let apothem = r * (std::f32::consts::PI / SIDES as f32).cos();
    for i in 0..SIDES {
        if out.len() < 3 {
            return Vec::new();
        }
        let ang = std::f32::consts::TAU * i as f32 / SIDES as f32;
        // Half-plane `n · (p − center) <= apothem`, one per side of the inscribed polygon.
        let n = glam::Vec2::new(ang.cos(), ang.sin());
        let inside = |p: glam::Vec2| n.dot(p - center) <= apothem;
        let cut = |a: glam::Vec2, b: glam::Vec2| {
            let (da, db) = (n.dot(a - center) - apothem, n.dot(b - center) - apothem);
            let t = da / (da - db);
            a + (b - a) * t
        };
        let mut next = Vec::with_capacity(out.len() + 1);
        for k in 0..out.len() {
            let (a, b) = (out[k], out[(k + 1) % out.len()]);
            match (inside(a), inside(b)) {
                (true, true) => next.push(b),
                (true, false) => next.push(cut(a, b)),
                (false, true) => {
                    next.push(cut(a, b));
                    next.push(b);
                }
                (false, false) => {}
            }
        }
        out = next;
    }
    if out.len() < 3 {
        return Vec::new();
    }
    out
}

/// One filled patch a loupe redraws inside its magnified circle (#1850): a convex polygon
/// ready to fan-triangulate, carrying the same `tint`/`shade` the view's own fill does so
/// every renderer maps it through the formula it already has.
pub struct LoupeFill {
    pub points: Vec<glam::Vec2>,
    pub tint: [u8; 3],
    pub shade: f32,
}

/// One shading mark a loupe redraws (#1850) — a colored-pencil scribble or a watercolor
/// pass, clipped and magnified like everything else it shows.
pub struct LoupeStroke {
    pub a: glam::Vec2,
    pub b: glam::Vec2,
    pub tint: [u8; 3],
    pub shade: f32,
    pub width: f32,
    pub on_sheet: bool,
}

/// What a loupe paints under its edges, in back-to-front order (#1850): the styled view's
/// own fills and shading marks, clipped to the detail circle and magnified.
pub enum LoupeMark {
    Fill(LoupeFill),
    Stroke(LoupeStroke),
}

/// The styled view's painted marks as the loupe redraws them (#1850).
pub fn loupe_magnified_marks(
    loupe: &crate::model::DrawingLoupe,
    styled: &StyledViewGeometry,
) -> Vec<LoupeMark> {
    let (c1, c2) = loupe_centers(loupe);
    let zoom = loupe_zoom(loupe);
    let r = loupe.radius.abs();
    let map = |p: glam::Vec2| c2 + (p - c1) * zoom;
    let mut out = Vec::new();
    for mark in styled.painted() {
        match mark {
            PaintedMark::Fill(face) => {
                for tri in &face.tris {
                    let clipped = clip_convex_to_circle(tri, c1, r);
                    if clipped.len() < 3 {
                        continue;
                    }
                    out.push(LoupeMark::Fill(LoupeFill {
                        points: clipped.into_iter().map(map).collect(),
                        tint: face.tint,
                        shade: face.shade,
                    }));
                }
            }
            PaintedMark::Stroke(stroke) => {
                if let Some((a, b)) = clip_segment_to_circle(stroke.a, stroke.b, c1, r) {
                    out.push(LoupeMark::Stroke(LoupeStroke {
                        a: map(a),
                        b: map(b),
                        tint: stroke.tint,
                        shade: stroke.shade,
                        // A mark measured on the sheet is magnified with what it covers.
                        width: if stroke.on_sheet { stroke.width * zoom } else { stroke.width },
                        on_sheet: stroke.on_sheet,
                    }));
                }
            }
        }
    }
    out
}

/// Everything a renderer draws for one zoom loupe (#1846), in the view's projected
/// millimetres. Built once by [`loupe_drawing`] so the editor sheet and the SVG/PDF export
/// put down the same thing.
pub struct LoupeDrawing {
    /// The detail circle over the geometry: centre and radius.
    pub detail: (glam::Vec2, f32),
    /// The magnified circle.
    pub magnified: (glam::Vec2, f32),
    /// The rim-to-rim line joining them, absent when the circles overlap.
    pub connector: Option<(glam::Vec2, glam::Vec2)>,
    /// The view's edges as the magnified circle redraws them.
    pub content: Vec<(glam::Vec2, glam::Vec2)>,
    /// The view's section hatch, likewise — kept apart so it strokes thinner, as it does on
    /// the view itself.
    pub hatch: Vec<(glam::Vec2, glam::Vec2)>,
    /// What goes down *under* the edges (#1850): the styled view's fills and shading marks,
    /// clipped and magnified, back to front. Empty for a style that paints nothing.
    pub marks: Vec<LoupeMark>,
}

/// What to draw for one zoom loupe (#1846): its two circles, the line joining their rims, and
/// the view's own lines redrawn magnified inside the big one. `lines` is the view's stroked
/// geometry ([`view_stroked_lines`]) and `hatch` its section hatch.
pub fn loupe_drawing(
    loupe: &crate::model::DrawingLoupe,
    lines: &[(glam::Vec2, glam::Vec2)],
    hatch: &[(glam::Vec2, glam::Vec2)],
    styled: Option<&StyledViewGeometry>,
) -> LoupeDrawing {
    let (c1, c2) = loupe_centers(loupe);
    LoupeDrawing {
        detail: (c1, loupe.radius.abs()),
        magnified: (c2, loupe.to_radius.abs()),
        connector: loupe_connector(loupe),
        content: loupe_magnified_segments(loupe, lines),
        hatch: loupe_magnified_segments(loupe, hatch),
        marks: styled.map(|s| loupe_magnified_marks(loupe, s)).unwrap_or_default(),
    }
}

/// Stroke width for a loupe's circles and the line joining them (#1846): thinner than the
/// model outline, so the loupe reads as an annotation over the drawing rather than part of
/// the part. The geometry it magnifies strokes at the usual [`MODEL_STROKE`].
pub const LOUPE_STROKE: f32 = MODEL_STROKE * 0.5;

/// The rim band of a loupe circle that **resizes** it, in screen pixels (#1846/#1851).
///
/// Inside the band the circle moves; on it, it resizes. Proportional so a big circle keeps a
/// band you can hit without aiming, with a floor so a small one still has one at all — and
/// shared by the hit test and the paint, so what a selected loupe *shows* as its grab zone is
/// exactly the zone that grabs.
pub fn loupe_resize_band_px(radius_px: f32) -> f32 {
    (radius_px * 0.3).max(6.0)
}

/// Radius of the dot at a selected loupe circle's centre — the handle that **moves** it
/// (#1851). Small enough to leave the magnified detail readable around it.
pub fn loupe_move_handle_px(radius_px: f32) -> f32 {
    (radius_px * 0.12).clamp(3.0, 7.0)
}

/// Padding inside a view card between its border and the projected geometry.
pub const CELL_PAD: f32 = 12.0;
/// Screen-pixel half-size of a selected view's corner resize grip (#1207).
pub const VIEW_RESIZE_HANDLE_RADIUS_PX: f32 = 6.0;

/// Stroke width for the model's projected edges and detected circles (#327). Kept clearly
/// heavier than the dimension/extension lines so the part outline reads as the primary geometry.
pub const MODEL_STROKE: f32 = 1.6;
/// Stroke width for dimension lines, their extension lines, and diameter lines (#327) — thinner
/// than [`MODEL_STROKE`] so annotations sit visually beneath the model outline.
pub const DIM_STROKE: f32 = 0.6;

/// Section-hatch stroke width in a drawing (#1784): half the model edges', so the hatch
/// reads as a fill texture on the cut faces rather than lines competing with the outline.
pub const HATCH_STROKE: f32 = MODEL_STROKE * 0.5;

/// Width of one colored-pencil scribble on the page (#1840). A section hatch is a texture and
/// draws thin; a scribble is the color itself, and laid down at hatch weight the face stayed
/// nearly bare paper. The viewport's pencil lays a mark heavier than its own outline — this is
/// the page's version of that.
pub const SCRIBBLE_STROKE: f32 = MODEL_STROKE * 0.9;
/// Width of one side-of-the-lead pass on the page (#1840), as a multiple of the pitch between
/// passes: the flat of the pencil covers a band, where its point draws a line, and the bands
/// have to overlap or the tone reads as stripes.
pub const SIDE_STROKE_OF_PITCH: f32 = 1.7;
/// How far from the ground toward the full laid-on tone one pass at full pressure reaches.
/// A pencil grazes the paper: the color is the body's, but a long way from saturated.
pub const SIDE_STRENGTH: f32 = 0.42;
/// …and how many of them cross the view, corner to corner (#1840). The number, not a spacing
/// in millimetres, is what makes a hand's fill read the same on a 6 mm part and a 600 mm one.
pub const SCRIBBLE_LINES_ACROSS: f32 = 80.0;

/// Combined solid mesh of every body a view projects (#1190/#1191). Empty/`None` for sketch
/// views or when no body still has geometry.
/// The cutting planes a view is sectioned by (#1689): those of the cross-section view it was
/// imported from, or none for an ordinary projection.
pub fn drawing_view_cuts<'a>(
    doc: &'a Document,
    view: &DrawingView,
) -> &'a [crate::model::CrossSectionCut] {
    view.cross_section
        .and_then(|key| doc.cross_sections.get(key))
        .map(|v| v.cuts.as_slice())
        .unwrap_or(&[])
}

/// The bodies a view actually projects (#1854).
///
/// The stored keys are what the view was made from; each resolves to whatever live body
/// replaced it, so a projection keeps up when the part gains a feature (extruding onto a
/// body consumes it and produces a new one) instead of drawing the part as it used to be.
pub fn drawing_view_bodies(doc: &Document, view: &DrawingView) -> Vec<crate::model::BodyKey> {
    let mut out: Vec<crate::model::BodyKey> = Vec::new();
    for &bi in &view.bodies {
        for live in crate::model::live_successor_bodies(doc, bi) {
            if !out.contains(&live) {
                out.push(live);
            }
        }
    }
    out
}

/// One body's mesh as this view shows it (#1689): cut by the view's cross-section planes
/// when it has any — each only where its cut/exclude scope takes this body (#1769) — and
/// whole otherwise.
fn drawing_view_body_mesh(
    doc: &Document,
    view: &DrawingView,
    body: crate::model::BodyKey,
) -> Option<crate::extrude::SolidMesh> {
    match drawing_view_cuts(doc, view) {
        [] => crate::extrude::body_solid_mesh(doc, body),
        cuts => crate::extrude::sectioned_body_mesh(doc, body, cuts),
    }
}

pub fn drawing_view_solid_mesh(
    doc: &Document,
    view: &DrawingView,
) -> Option<crate::extrude::SolidMesh> {
    if view.sketch.is_some() {
        return None;
    }
    let mut mesh = crate::extrude::SolidMesh::default();
    for bi in drawing_view_bodies(doc, view) {
        if let Some(solid) = drawing_view_body_mesh(doc, view, bi) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// The view's bodies as separate meshes, each with the color it should paint in (#1807):
/// its material's color for the `Colorful` style, white — a colorless tint — for the grey
/// `Shaded` one. Ordering matches [`drawing_view_solid_mesh`], which merges the same meshes.
fn drawing_view_body_meshes(
    doc: &Document,
    view: &DrawingView,
) -> Vec<(crate::extrude::SolidMesh, [u8; 3])> {
    if view.sketch.is_some() {
        return Vec::new();
    }
    // Colored pencil keeps each body's own color too (#1821) — that is what makes it
    // *colored* pencil rather than the grey one.
    let colorful = view.style == crate::model::DrawingViewStyle::Colorful
        || view.style.is_hand_colored();
    drawing_view_bodies(doc, view)
        .into_iter()
        .filter_map(|bi| {
            let solid = drawing_view_body_mesh(doc, view, bi)?;
            let tint = match (colorful, doc.bodies.get(bi)) {
                (true, Some(body)) => {
                    let c = crate::gpu_viewport::body_material_fill(doc, body);
                    [c.r(), c.g(), c.b()]
                }
                _ => [255, 255, 255],
            };
            Some((solid, tint))
        })
        .collect()
}

/// Caption source label for a view: sketch name, single body name, component name when all
/// bodies belong to one component (#1190), otherwise a short multi-body summary (#1191).
pub fn drawing_view_source_label(doc: &Document, view: &DrawingView) -> String {
    use crate::hierarchy::HierarchyNode;
    use crate::names::node_label;
    if let Some(si) = view.sketch {
        return node_label(doc, HierarchyNode::Sketch(si));
    }
    match drawing_view_bodies(doc, view).as_slice() {
        [] => "Projection".to_string(),
        [bi] => node_label(doc, HierarchyNode::Body(*bi)),
        bodies => {
            // Prefer the component name when every body is owned by the same component (or a
            // nested one under it) — the usual "add whole component" case (#1190).
            if let Some(label) = shared_component_label(doc, bodies) {
                return label;
            }
            if bodies.len() <= 3 {
                bodies
                    .iter()
                    .map(|bi| node_label(doc, HierarchyNode::Body(*bi)))
                    .collect::<Vec<_>>()
                    .join(" + ")
            } else {
                format!("{} bodies", bodies.len())
            }
        }
    }
}

/// When every body is owned by the same component (possibly nested under it), that
/// component's label; otherwise `None`.
fn shared_component_label(doc: &Document, bodies: &[crate::model::BodyKey]) -> Option<String> {
    use crate::hierarchy::{owning_component, HierarchyNode, SceneElement};
    use crate::names::node_label;
    let mut owners = bodies.iter().map(|&bi| {
        owning_component(doc, &SceneElement::Body(bi))
    });
    let first = owners.next()??;
    if owners.all(|o| o == Some(first)) {
        Some(node_label(doc, HierarchyNode::Component(first)))
    } else {
        None
    }
}

/// The world-space feature edges a drawing view projects (#278): each body's solid-mesh unique
/// edges, or — when the view's `sketch` is set — that sketch's line/circle geometry. Shared by
/// the editor pane and the SVG/PDF export so both draw the same thing.
pub fn drawing_view_world_edges(doc: &Document, view: &DrawingView) -> Vec<(Vec3, Vec3)> {
    if let Some(si) = view.sketch {
        let mut edges = Vec::new();
        for line in doc.lines.values().filter(|l| l.sketch == si) {
            if let Some(pts) = crate::face::line_world_polyline(doc, line) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        for circle in doc.circles.values().filter(|c| c.sketch == si) {
            if let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 48) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        edges
    } else {
        // Crease/feature edges only — the view-dependent silhouette (#319) is added later, in
        // the stroke geometry, so it doesn't interfere with circle detection (#313). Multi-body
        // views union each body's creases (#1190/#1191).
        let mut edges = Vec::new();
        for bi in drawing_view_bodies(doc, view) {
            if let Some(mesh) = drawing_view_body_mesh(doc, view, bi) {
                edges.extend(crate::gpu_viewport::solid_mesh_unique_edges(&mesh));
            }
        }
        edges
    }
}

/// The hatch lines a sectioned view draws on the faces its planes opened (#1689), in world
/// space, ready to project with the rest of the view's edges.
pub fn section_hatch_world_segments(doc: &Document, view: &DrawingView) -> Vec<(Vec3, Vec3)> {
    let cuts = drawing_view_cuts(doc, view);
    if cuts.is_empty() {
        return Vec::new();
    }
    let Some(mesh) = drawing_view_solid_mesh(doc, view) else {
        return Vec::new();
    };
    cuts.iter()
        .flat_map(|cut| {
            crate::extrude::section_hatch_segments(
                &mesh,
                cut,
                crate::extrude::SECTION_HATCH_SPACING_MM,
            )
        })
        .collect()
}

/// The view-dependent silhouette edges of a body view (#319): a cylinder's straight sides and
/// other smooth-surface outlines that aren't crease edges. Empty for sketch views. Multi-body
/// views use the combined mesh so silhouettes account for the whole assembly (#1190/#1191).
pub fn drawing_view_silhouette_edges(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> Vec<(Vec3, Vec3)> {
    if view.sketch.is_some() {
        return Vec::new();
    }
    let Some(mesh) = drawing_view_solid_mesh(doc, view) else {
        return Vec::new();
    };
    let (right, up) = resolved_view_axes(views, view);
    crate::gpu_viewport::solid_mesh_silhouette_edges(&mesh, right.cross(up))
}

/// Quantized world vertices of this view, mapped to the body they sit on (#1714).
/// First body wins at a coincident vertex.
pub fn drawing_view_vertex_bodies(
    doc: &Document,
    view: &DrawingView,
) -> std::collections::HashMap<[i32; 3], crate::model::BodyKey> {
    let mut map = std::collections::HashMap::new();
    for bi in drawing_view_bodies(doc, view) {
        let Some(mesh) = drawing_view_body_mesh(doc, view, bi) else {
            continue;
        };
        for tri in &mesh.triangles {
            for p in tri {
                map.entry(crate::hierarchy::quantize_body_point(*p)).or_insert(bi);
            }
        }
    }
    map
}

/// The body of this view that owns both endpoints of a world edge, if any (#1714).
pub fn drawing_view_edge_body(
    vertex_bodies: &std::collections::HashMap<[i32; 3], crate::model::BodyKey>,
    a: Vec3,
    b: Vec3,
) -> Option<crate::model::BodyKey> {
    let qa = crate::hierarchy::quantize_body_point(a);
    let qb = crate::hierarchy::quantize_body_point(b);
    let ba = vertex_bodies.get(&qa)?;
    let bb = vertex_bodies.get(&qb)?;
    (ba == bb).then_some(*ba)
}

/// The edges a view can dimension (#334): its crease/feature edges plus the view-dependent
/// silhouette edges (a cylinder's straight sides), so the **length** of a smooth extrusion — which
/// has no crease edge down its side — can be dimensioned like any straight edge. Silhouette edges
/// are deduped against the crease set by quantized endpoints. Circle detection deliberately stays
/// on the crease-only [`drawing_view_world_edges`] (#319), so this is used only for dimensioning.
pub fn drawing_view_dimensionable_edges(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> Vec<(Vec3, Vec3)> {
    let mut edges = drawing_view_world_edges(doc, view);
    let mut seen: std::collections::HashSet<crate::model::DrawingEdgeKey> = edges
        .iter()
        .map(|(a, b)| {
            crate::model::normalized_edge_key(
                crate::hierarchy::quantize_body_point(*a),
                crate::hierarchy::quantize_body_point(*b),
            )
        })
        .collect();
    for (a, b) in drawing_view_silhouette_edges(doc, views, view) {
        let key = crate::model::normalized_edge_key(
            crate::hierarchy::quantize_body_point(a),
            crate::hierarchy::quantize_body_point(b),
        );
        if seen.insert(key) {
            edges.push((a, b));
        }
    }
    // One straight line, one dimension (#1644).
    merge_collinear_runs(&edges)
}

/// A curve's tessellation turns by less than this at every joint; a real corner turns by
/// more. Between the two sits a band of ambiguity no threshold resolves — a coarse facet
/// chain reads as corners, a very shallow kink reads as a curve — and these values split the
/// difference the way technical drawings draw: facets from the kernel tessellation turn by a
/// few degrees at most, while feature corners are sharp.
const PICK_SHARP_TURN_COS: f32 = 0.906_307_8; // cos 25°
/// Two facets this parallel are the same straight run; anything more is the curve turning.
const PICK_STRAIGHT_RUN_COS: f32 = 0.999_998_5; // within ~0.1°

/// The *logical* pick geometry of a view (#1780/#1781/#1785): [`merge_collinear_runs`]
/// makes each straight model edge one edge; what is left that turns is a curve's
/// tessellation — those facets (and their faux vertices) are rendering artifacts, not line
/// geometry to pick or dimension, so a turning chain of ≥ 3 segments comes back as one
/// curve polyline (a whole-curve pick, #1785) instead of line picks. Corners (sharp turns)
/// and short 1–2 segment chains — two real edges meeting at a shallow angle — survive as
/// untouched line picks.
///
/// `project` gives a segment's view projection; the turn test runs on it, because that is
/// what the reader of the drawing sees.
fn logical_pick_geometry(
    world_edges: &[(Vec3, Vec3)],
    project: &impl Fn(Vec3) -> glam::Vec2,
) -> (Vec<(Vec3, Vec3)>, Vec<Vec<Vec3>>) {
    let world_edges = merge_collinear_runs(world_edges);
    let projected: Vec<glam::Vec2> =
        world_edges.iter().flat_map(|(a, b)| [project(*a), project(*b)]).collect();
    assert_eq!(projected.len(), world_edges.len() * 2, "2 projected points per edge");
    // Weld segment endpoints by quantized world position, so chains follow the geometry
    // rather than the projection (an edge-on curve projects straight but still turns).
    let mut node_of: std::collections::HashMap<[i32; 3], usize> = Default::default();
    let mut nodes: Vec<Vec3> = Vec::new();
    let mut node_proj: Vec<glam::Vec2> = Vec::new();
    let node = |p: Vec3,
                proj: glam::Vec2,
                node_of: &mut std::collections::HashMap<[i32; 3], usize>,
                nodes: &mut Vec<Vec3>,
                node_proj: &mut Vec<glam::Vec2>|
     -> usize {
        *node_of.entry(crate::hierarchy::quantize_body_point(p)).or_insert_with(|| {
            nodes.push(p);
            node_proj.push(proj);
            nodes.len() - 1
        })
    };
    let ends: Vec<(usize, usize)> = world_edges
        .iter()
        .enumerate()
        .map(|(i, (a, b))| {
            (
                node(*a, projected[2 * i], &mut node_of, &mut nodes, &mut node_proj),
                node(*b, projected[2 * i + 1], &mut node_of, &mut nodes, &mut node_proj),
            )
        })
        .collect();
    let mut adjacency: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];
    for (i, &(na, nb)) in ends.iter().enumerate() {
        adjacency[na].push((i, nb));
        adjacency[nb].push((i, na));
    }
    let dir = |from: usize, to: usize| -> glam::Vec2 {
        (node_proj[to] - node_proj[from]).normalize_or_zero()
    };
    // Walk every unvisited segment out in both directions through degree-2 joints, stopping
    // at junctions and sharp turns, to collect the maximal facet chains.
    let mut visited = vec![false; world_edges.len()];
    let mut out = Vec::new();
    let mut curves: Vec<Vec<Vec3>> = Vec::new();
    for i in 0..world_edges.len() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let mut chain = vec![i];
        // The chain's nodes in walk order: [na, nb, then right-walk appends, left-walk
        // prepends] — the curve polyline when the chain turns out to be a curve.
        let mut walk: Vec<usize> = vec![ends[i].0, ends[i].1];
        for &(first, first_dir) in &[(ends[i].1, 1usize), (ends[i].0, 0usize)] {
            let mut at = first;
            // Direction of travel into `at`: along segment `i`, toward `at`.
            let mut incoming = if first_dir == 1 {
                dir(ends[i].0, ends[i].1)
            } else {
                dir(ends[i].1, ends[i].0)
            };
            loop {
                let others: Vec<(usize, usize)> =
                    adjacency[at].iter().copied().filter(|(s, _)| !visited[*s]).collect();
                if others.len() != 1 {
                    break;
                }
                let (next, beyond) = others[0];
                let outgoing = dir(at, beyond);
                if incoming.dot(outgoing) < PICK_SHARP_TURN_COS {
                    break;
                }
                visited[next] = true;
                chain.push(next);
                if first_dir == 1 {
                    walk.push(beyond);
                } else {
                    walk.insert(0, beyond);
                }
                incoming = outgoing;
                at = beyond;
            }
        }
        // Classify the chain by how it projects. A run of ≥ 3 facets is a curve's
        // tessellation: when it turns anywhere on the page it is one curve — not line
        // picks, but a whole-curve pick of its own (#1785); when it projects dead straight
        // (a curve seen edge-on) it reads as one line, so it merges to one edge spanning
        // its extremes. Short chains stay — two segments can be two real edges meeting at
        // a shallow angle, and a lone segment is geometry in its own right.
        if chain.len() >= 3 {
            let straight = chain.windows(2).all(|w| {
                let (a, b) = (w[0], w[1]);
                let (shared, a_other) = if ends[a].0 == ends[b].0 || ends[a].0 == ends[b].1 {
                    (ends[a].0, ends[a].1)
                } else {
                    (ends[a].1, ends[a].0)
                };
                let b_other = if ends[b].0 == shared { ends[b].1 } else { ends[b].0 };
                dir(a_other, shared).dot(dir(shared, b_other)) >= PICK_STRAIGHT_RUN_COS
            });
            if straight {
                // The chain's extreme nodes along its own projected direction.
                let axis = dir(ends[chain[0]].0, ends[chain[0]].1);
                let (mut lo, mut hi) = (ends[chain[0]].0, ends[chain[0]].0);
                let (mut lo_t, mut hi_t) = (f32::MAX, f32::MIN);
                for &s in &chain {
                    for n in [ends[s].0, ends[s].1] {
                        let t = node_proj[n].dot(axis);
                        if t < lo_t {
                            lo_t = t;
                            lo = n;
                        }
                        if t > hi_t {
                            hi_t = t;
                            hi = n;
                        }
                    }
                }
                if lo != hi {
                    out.push((nodes[lo], nodes[hi]));
                }
            } else {
                // The curve's world points in walk order; a closed cycle (the walk arrived
                // back at its own start) already ends where it began.
                curves.push(walk.iter().map(|&n| nodes[n]).collect());
            }
        } else {
            for &s in &chain {
                out.push(world_edges[s]);
            }
        }
    }
    let lines = collapse_overlapping_projected_spans(out, project);
    (lines, curves)
}

/// The *logical* pick edges of a view (#1780/#1781): see [`logical_pick_geometry`].
pub fn logical_pick_edges(
    world_edges: &[(Vec3, Vec3)],
    project: &impl Fn(Vec3) -> glam::Vec2,
) -> Vec<(Vec3, Vec3)> {
    logical_pick_geometry(world_edges, project).0
}

/// The *logical* curve picks of a view (#1785): the tessellation chains that turn — a cut
/// ellipse, a fillet run — each as one ordered world polyline. A curve toggles and
/// dimensions as a whole; its facets are never picks (#1781).
pub fn logical_pick_curves(
    world_edges: &[(Vec3, Vec3)],
    project: &impl Fn(Vec3) -> glam::Vec2,
) -> Vec<Vec<Vec3>> {
    logical_pick_geometry(world_edges, project).1
}

/// A curve dimension's stored identity (#1785): the polyline rotated to start at its
/// lexicographically smallest point and, if the reversed walk sorts earlier, reversed — so
/// the same curve picked from either end toggles the same entry.
pub fn canonical_curve_key(points: &[[i32; 3]]) -> Vec<[i32; 3]> {
    let n = points.len();
    let mut best = points.to_vec();
    for reversed in [false, true] {
        let seq: Vec<[i32; 3]> = if reversed {
            points.iter().rev().copied().collect()
        } else {
            points.to_vec()
        };
        for start in 0..n {
            let rotated: Vec<[i32; 3]> = (0..n).map(|i| seq[(start + i) % n]).collect();
            if rotated < best {
                best = rotated;
            }
        }
    }
    best
}

/// A curve chain's world length (#1785): the sum of its tessellation chords — the polyline
/// is the kernel's own tessellation, fine enough to read as the arc itself.
pub fn curve_chain_length(world: &[Vec3]) -> f32 {
    world.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

/// Collapse groups of collinear, overlapping projected spans into one edge per stretch,
/// keeping the world endpoints that reach farthest along the line.
fn collapse_overlapping_projected_spans(
    edges: Vec<(Vec3, Vec3)>,
    project: &impl Fn(Vec3) -> glam::Vec2,
) -> Vec<(Vec3, Vec3)> {
    const COLLINEAR_DOT: f32 = 1.0 - 1e-4;
    const OFFSET_TOL: f32 = 0.05; // mm on the page
    const GAP_TOL: f32 = 0.05; // mm on the page
    let pts: Vec<[glam::Vec2; 2]> = edges
        .iter()
        .map(|(a, b)| [project(*a), project(*b)])
        .collect();
    let n = edges.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        let mut j = i;
        while parent[j] != j {
            let next = parent[j];
            parent[j] = r;
            j = next;
        }
        r
    }
    let unit = |a: glam::Vec2, b: glam::Vec2| (b - a).normalize_or_zero();
    for i in 0..n {
        for j in (i + 1)..n {
            let (di, dj) = (unit(pts[i][0], pts[i][1]), unit(pts[j][0], pts[j][1]));
            if di.dot(dj).abs() < COLLINEAR_DOT || di.length_squared() < 0.5 {
                continue;
            }
            // Same infinite line: j's endpoints close to i's line.
            let offset = ((pts[j][0] - pts[i][0]).perp_dot(di)).abs();
            if offset > OFFSET_TOL {
                continue;
            }
            // Overlapping or nearly touching intervals along i's direction.
            let ti = (
                (pts[i][0] - pts[i][0]).dot(di),
                (pts[i][1] - pts[i][0]).dot(di),
            );
            let (ta, tb) = (
                (pts[j][0] - pts[i][0]).dot(di),
                (pts[j][1] - pts[i][0]).dot(di),
            );
            let (i_lo, i_hi) = (ti.0.min(ti.1), ti.0.max(ti.1));
            let (j_lo, j_hi) = (ta.min(tb), ta.max(tb));
            if j_lo > i_hi + GAP_TOL || i_lo > j_hi + GAP_TOL {
                continue;
            }
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        groups.entry(find(&mut parent, i)).or_default().push(i);
    }
    let mut out = Vec::new();
    for members in groups.values() {
        if members.len() == 1 {
            out.push(edges[members[0]]);
            continue;
        }
        // Union interval along the first member's direction, carrying the world points
        // that sit at the two extremes.
        let base = members[0];
        let dir = unit(pts[base][0], pts[base][1]);
        let (mut lo_p, mut hi_p) = (edges[base].0, edges[base].1);
        let (mut lo_t, mut hi_t) = (f32::MAX, f32::MIN);
        for &m in members {
            for p in [edges[m].0, edges[m].1] {
                let t = project(p).dot(dir);
                if t < lo_t {
                    lo_t = t;
                    lo_p = p;
                }
                if t > hi_t {
                    hi_t = t;
                    hi_p = p;
                }
            }
        }
        if lo_p != hi_p {
            out.push((lo_p, hi_p));
        }
    }
    out
}

/// Join every run of touching, collinear edges into one (#1644). A straight line on a body is
/// broken into a segment per face that meets it, and dimensioning one 20 mm piece of an 80 mm
/// edge is not what anyone is after — so the dimension surface sees the whole run.
///
/// Edges are grouped by the infinite 3D line they lie on and merged along it; a gap between two
/// stretches of the same line keeps them apart, and the merged endpoints are the original mesh
/// points, so the length stays exact.
pub fn merge_collinear_runs(edges: &[(Vec3, Vec3)]) -> Vec<(Vec3, Vec3)> {
    // Quantized line identity: canonical direction plus the foot of the perpendicular from the
    // origin, so every edge of one line lands in the same bucket whichever way it is drawn.
    type LineKey = ([i32; 3], [i32; 3]);
    let quant_dir = |d: Vec3| {
        [
            (d.x * 10_000.0).round() as i32,
            (d.y * 10_000.0).round() as i32,
            (d.z * 10_000.0).round() as i32,
        ]
    };
    // Each entry: the line, and its edges as intervals `(t_min, p_min, t_max, p_max)` along it.
    let mut groups: Vec<(LineKey, Vec<(f32, Vec3, f32, Vec3)>)> = Vec::new();
    let mut out: Vec<(Vec3, Vec3)> = Vec::new();
    for &(a, b) in edges {
        let d = b - a;
        if d.length_squared() < 1e-12 {
            out.push((a, b));
            continue;
        }
        let dir = d.normalize();
        // Canonical direction: flip so the first significant component is positive.
        let flip = if dir.x.abs() > 1e-6 {
            dir.x < 0.0
        } else if dir.y.abs() > 1e-6 {
            dir.y < 0.0
        } else {
            dir.z < 0.0
        };
        let dir = if flip { -dir } else { dir };
        let foot = a - dir * a.dot(dir);
        let key: LineKey = (quant_dir(dir), crate::hierarchy::quantize_body_point(foot));
        let (ta, tb) = (a.dot(dir), b.dot(dir));
        let interval = if ta <= tb { (ta, a, tb, b) } else { (tb, b, ta, a) };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(interval),
            None => groups.push((key, vec![interval])),
        }
    }
    for (_, mut intervals) in groups {
        intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut run = intervals[0];
        for &(t0, p0, t1, p1) in &intervals[1..] {
            // Touching (or overlapping) segments continue the run; a gap ends it.
            if t0 <= run.2 + 1e-3 {
                if t1 > run.2 {
                    run.2 = t1;
                    run.3 = p1;
                }
            } else {
                out.push((run.1, run.3));
                run = (t0, p0, t1, p1);
            }
        }
        out.push((run.1, run.3));
    }
    out
}

/// How close (screen px) a click has to be to a projected corner for a free dimension's point
/// to snap onto it (#1645), so "corner to corner" measures exactly.
pub const POINT_DIM_SNAP_PX: f32 = 10.0;

/// What a free point-to-point dimension measures (#1645), in the view's projected millimetres:
/// the straight distance, or the separation along one page axis.
pub fn point_dim_value(dim: &crate::model::DrawingPointDim) -> f32 {
    use crate::model::PointDimAxis as A;
    let (dx, dy) = (dim.b.0 - dim.a.0, dim.b.1 - dim.a.1);
    match dim.axis {
        A::Direct => (dx * dx + dy * dy).sqrt(),
        A::Horizontal => dx.abs(),
        A::Vertical => dy.abs(),
    }
}

/// Where a free point-to-point dimension's line runs, and which way its extension lines
/// leave the picked points (#1645).
///
/// The dimension line sits `gap` beyond whichever point is further out along the outward
/// normal, so both extension lines are visible whatever the two points' offsets. For a Direct
/// dimension the two are level with each other and this is the ordinary parallel dimension;
/// for Horizontal/Vertical it is the axis line with a longer extension from the nearer point.
pub fn point_dim_line(
    dim: &crate::model::DrawingPointDim,
    gap: f32,
) -> (glam::Vec2, glam::Vec2, glam::Vec2) {
    use crate::model::PointDimAxis as A;
    let a = glam::Vec2::new(dim.a.0, dim.a.1);
    let b = glam::Vec2::new(dim.b.0, dim.b.1);
    let axis = match dim.axis {
        A::Direct => (b - a).normalize_or(glam::Vec2::X),
        A::Horizontal => glam::Vec2::X,
        A::Vertical => glam::Vec2::Y,
    };
    // Outward normal: away from the pair's own midpoint side, biased down/left so a fresh
    // dimension lands clear of the geometry between the points.
    let mut out = glam::Vec2::new(-axis.y, axis.x);
    if out.dot(b - a) < 0.0 {
        out = -out;
    }
    if (dim.axis == A::Horizontal && out.y > 0.0) || (dim.axis == A::Vertical && out.x > 0.0) {
        out = -out;
    }
    let reach = a.dot(out).max(b.dot(out)) + gap + dim.offset;
    let pa = a + out * (reach - a.dot(out));
    let pb = b + out * (reach - b.dot(out));
    (pa, pb, out)
}

/// The architectural dimension-line geometry for one edge (#294), all in the view's projected
/// 2D mm space. `a`/`b` are the edge endpoints; `outward` is the unit perpendicular pointing
/// away from the geometry centroid; `offset` is how far out along `outward` the dimension line
/// sits. Both the editor and the exports build their strokes from this so they never drift.
pub struct DimLineGeometry {
    /// The two extension lines (edge endpoint → just past the dimension line).
    pub extensions: [(glam::Vec2, glam::Vec2); 2],
    /// The dimension line itself (endpoint to endpoint, parallel to the edge).
    pub line: (glam::Vec2, glam::Vec2),
    /// Two arrowhead triangles (three points each) at the dimension line's ends.
    pub arrows: [[glam::Vec2; 3]; 2],
}

/// Build [`DimLineGeometry`] for an edge from `a` to `b`, offset `outward * offset` from it.
/// `arrow` is the arrowhead length in the same units, so callers can size features to the
/// drawing (a proportional fraction of the projected extent keeps them readable at any scale).
pub fn dimension_line_geometry(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    offset: f32,
    arrow: f32,
) -> DimLineGeometry {
    let da = a + outward * offset;
    let db = b + outward * offset;
    let along = (db - da).normalize_or_zero();
    // Arrowheads point outward from the line centre toward each end.
    let head = |tip: glam::Vec2, dir: glam::Vec2| {
        let base = tip - dir * arrow;
        let side = glam::Vec2::new(-dir.y, dir.x) * (arrow * 0.4);
        [tip, base + side, base - side]
    };
    DimLineGeometry {
        // Extension lines start a hair off the edge and overshoot the dimension line a touch.
        extensions: [
            (a + outward * (arrow * 0.4), da + outward * (arrow * 0.7)),
            (b + outward * (arrow * 0.4), db + outward * (arrow * 0.7)),
        ],
        line: (da, db),
        arrows: [head(da, -along), head(db, along)],
    }
}

/// Whether a projected point sits inside a loupe's detail circle, so a dimension end can
/// close with an arrow there. An end outside means the measured edge continues past what
/// the loupe shows (#1913).
pub fn loupe_contains_point(p: glam::Vec2, center: glam::Vec2, r: f32) -> bool {
    (p - center).length_squared() <= r * r + 1e-3
}

/// `true` at each end of projected edge `(a, b)` that lies inside the detail circle.
pub fn loupe_dim_closed_ends(
    a: glam::Vec2,
    b: glam::Vec2,
    center: glam::Vec2,
    r: f32,
) -> [bool; 2] {
    [loupe_contains_point(a, center, r), loupe_contains_point(b, center, r)]
}

/// An architectural dimension drawn on a zoom loupe (#1849/#1913). Same layout as
/// [`DimLineGeometry`], except a measured end that is **not** inside the detail circle
/// has no arrow and no extension line: ISO 129 keeps arrows only at shown feature ends,
/// and the open end finishes in dashes so the cropped bit is not read as the whole
/// measurement.
pub struct LoupeDimGeometry {
    pub extensions: Vec<(glam::Vec2, glam::Vec2)>,
    /// Full dimension line (for the length label), from the visible clipped segment.
    pub line: (glam::Vec2, glam::Vec2),
    /// Solid interior of the dimension line. `None` when the whole stroke is dashed.
    pub solid: Option<(glam::Vec2, glam::Vec2)>,
    /// Ends that continue past the loupe — stroke these dashed, with no arrow.
    pub dashes: Vec<(glam::Vec2, glam::Vec2)>,
    pub arrows: Vec<[glam::Vec2; 3]>,
}

/// Build [`LoupeDimGeometry`] for the magnified clipped edge `(a, b)`. `closed` is
/// [`loupe_dim_closed_ends`] on the **unclipped** projected edge against the detail circle.
pub fn loupe_dimension_geometry(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    offset: f32,
    arrow: f32,
    closed: [bool; 2],
) -> LoupeDimGeometry {
    let full = dimension_line_geometry(a, b, outward, offset, arrow);
    if closed == [true, true] {
        return LoupeDimGeometry {
            extensions: full.extensions.to_vec(),
            line: full.line,
            solid: Some(full.line),
            dashes: Vec::new(),
            arrows: full.arrows.to_vec(),
        };
    }
    let (da, db) = full.line;
    let along = (db - da).normalize_or_zero();
    let len = (db - da).length();
    // Long enough to read as "this continues", short enough to leave a solid middle
    // for the label. A tiny cropped segment dashes almost to the remaining arrow.
    let dash_span = if len < 1e-4 {
        0.0
    } else {
        (arrow * 2.5).clamp(0.0, len * 0.4)
    };
    let mut extensions = Vec::new();
    let mut arrows = Vec::new();
    let mut dashes = Vec::new();
    let mut solid_a = da;
    let mut solid_b = db;
    if closed[0] {
        extensions.push(full.extensions[0]);
        arrows.push(full.arrows[0]);
    } else if dash_span > 1e-4 {
        dashes.push((da, da + along * dash_span));
        solid_a = da + along * dash_span;
    }
    if closed[1] {
        extensions.push(full.extensions[1]);
        arrows.push(full.arrows[1]);
    } else if dash_span > 1e-4 {
        dashes.push((db - along * dash_span, db));
        solid_b = db - along * dash_span;
    }
    let solid = ((solid_b - solid_a).dot(along) > 1e-4).then_some((solid_a, solid_b));
    LoupeDimGeometry {
        extensions,
        line: full.line,
        solid,
        dashes,
        arrows,
    }
}

/// The drawn form of an angle dimension (#1652): an arc centred on the corner the two edges
/// make, sweeping from one edge to the other, with an arrowhead at each end and the degree
/// label just outside it. Everything is in the view's projected 2D mm space — the angle is
/// the one the arc actually spans on the page, so label and drawing always agree.
pub struct AngleDimGeometry {
    /// Where the two edges meet, produced if they don't touch.
    pub center: glam::Vec2,
    pub radius: f32,
    /// Arc bounds in radians, swept counter-clockwise from `start` to `end`.
    pub start: f32,
    pub end: f32,
    pub degrees: f32,
    /// Where the `NN°` label sits, just outside the arc's midpoint.
    pub label: glam::Vec2,
    /// Extension lines producing an edge back to the corner, for edges that stop short.
    pub extensions: Vec<(glam::Vec2, glam::Vec2)>,
    /// Two arrowhead triangles (three points each), one at each end of the arc.
    pub arrows: [[glam::Vec2; 3]; 2],
}

impl AngleDimGeometry {
    fn at(&self, angle: f32) -> glam::Vec2 {
        self.center + glam::Vec2::new(angle.cos(), angle.sin()) * self.radius
    }

    /// The arc as a polyline, fine enough to read as a curve at any drawing scale.
    pub fn arc_points(&self) -> Vec<glam::Vec2> {
        let steps = (((self.end - self.start).abs() / 0.15).ceil() as usize).clamp(8, 96);
        (0..=steps)
            .map(|i| self.at(self.start + (self.end - self.start) * (i as f32 / steps as f32)))
            .collect()
    }
}

/// Build [`AngleDimGeometry`] for the angle between two projected edges. `arrow` is the
/// arrowhead length in the same units. `None` when either edge is degenerate or the two are
/// parallel, since parallel edges never make a corner to measure.
pub fn angle_dim_geometry(
    e1: (glam::Vec2, glam::Vec2),
    e2: (glam::Vec2, glam::Vec2),
    arrow: f32,
) -> Option<AngleDimGeometry> {
    let (v1, v2) = (e1.1 - e1.0, e2.1 - e2.0);
    let (l1, l2) = (v1.length(), v2.length());
    if l1 < 1e-4 || l2 < 1e-4 {
        return None;
    }
    let (u1, u2) = (v1 / l1, v2 / l2);
    let cross = u1.perp_dot(u2);
    if cross.abs() < 1e-4 {
        return None; // parallel: no corner
    }
    let center = e1.0 + u1 * ((e2.0 - e1.0).perp_dot(u2) / cross);
    // The arc opens toward the ends that are actually drawn, so it lands on the edges rather
    // than on the empty produced side of the corner.
    let ends = |e: (glam::Vec2, glam::Vec2)| {
        if (e.0 - center).length_squared() <= (e.1 - center).length_squared() {
            (e.0, e.1)
        } else {
            (e.1, e.0)
        }
    };
    let ((near1, far1), (near2, far2)) = (ends(e1), ends(e2));
    let (d1, d2) = (
        (far1 - center).normalize_or_zero(),
        (far2 - center).normalize_or_zero(),
    );
    if d1 == glam::Vec2::ZERO || d2 == glam::Vec2::ZERO {
        return None;
    }
    // Small enough to stay well inside the shorter edge, but never smaller than its arrowheads.
    let reach = (far1 - center).length().min((far2 - center).length());
    let radius = (reach * 0.45).max(arrow * 2.0);
    let (a1, a2) = (d1.y.atan2(d1.x), d2.y.atan2(d2.x));
    let mut sweep = a2 - a1;
    while sweep <= -std::f32::consts::PI {
        sweep += std::f32::consts::TAU;
    }
    while sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    }
    let (start, end) = if sweep >= 0.0 { (a1, a1 + sweep) } else { (a1 + sweep, a1) };
    // An edge that stops short of the corner is produced back to it, as on a drawing board.
    let extensions = [(near1, d1), (near2, d2)]
        .into_iter()
        .filter(|(n, _)| (*n - center).length() > 1e-3)
        .map(|(n, _)| (n, center))
        .collect();
    let head = |angle: f32, dir: f32| {
        let tip = center + glam::Vec2::new(angle.cos(), angle.sin()) * radius;
        let tangent = glam::Vec2::new(-angle.sin(), angle.cos()) * dir;
        let base = tip - tangent * arrow;
        let side = glam::Vec2::new(-tangent.y, tangent.x) * (arrow * 0.4);
        [tip, base + side, base - side]
    };
    let mid = (start + end) * 0.5;
    Some(AngleDimGeometry {
        center,
        radius,
        start,
        end,
        degrees: sweep.abs().to_degrees(),
        label: center + glam::Vec2::new(mid.cos(), mid.sin()) * (radius + arrow * 1.6),
        extensions,
        arrows: [head(start, -1.0), head(end, 1.0)],
    })
}

/// Plan per-dimension **extra offsets** (beyond the default gap) so dimension lines and their
/// number labels don't overlap each other (#321): parallel dimensions whose lines would land at
/// the same distance and whose spans overlap are pushed out onto successive "tiers", the way CAD
/// stacks parallel dimensions. Input is one `(a, b, outward)` per dimension in projected mm;
/// output is the extra offset for each, in the same order. Greedy interval coloring per
/// parallel group, longest-span dimensions taking the innermost tier.
pub fn plan_dimension_tiers(dims: &[(glam::Vec2, glam::Vec2, glam::Vec2)], gap: f32) -> Vec<f32> {
    let n = dims.len();
    // Per-dimension: line direction, span [s0,s1] along it, and the signed distance of the
    // dimension line from the origin along `outward` (its "height", so parallel lines at the
    // same height on the same side are the ones that can collide).
    struct Info {
        dir: glam::Vec2,
        outward: glam::Vec2,
        s0: f32,
        s1: f32,
        height: f32,
        len: f32,
    }
    let info: Vec<Info> = dims
        .iter()
        .map(|&(a, b, outward)| {
            let seg = b - a;
            let len = seg.length().max(1e-6);
            let dir = seg / len;
            let s0 = a.dot(dir);
            let s1 = b.dot(dir);
            let (s0, s1) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
            // Dimension line sits at the edge midpoint pushed out by the default gap.
            let mid = (a + b) * 0.5 + outward * gap;
            Info { dir, outward, s0, s1, height: mid.dot(outward), len }
        })
        .collect();

    // Process longest first so big datums stay innermost; assign the lowest free tier.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| info[j].len.total_cmp(&info[i].len));
    let mut tier = vec![0usize; n];
    let mut placed: Vec<usize> = Vec::new(); // indices already assigned
    for &i in &order {
        let mut t = 0;
        'search: loop {
            for &j in &placed {
                if tier[j] != t {
                    continue;
                }
                // Same tier: collide if parallel, same side, near the same height, spans overlap.
                let parallel = info[i].dir.dot(info[j].dir).abs() > 0.99;
                let same_side = info[i].outward.dot(info[j].outward) > 0.9;
                let same_height = (info[i].height - info[j].height).abs() < gap * 0.5;
                let overlap = info[i].s0 < info[j].s1 - 1e-3 && info[j].s0 < info[i].s1 - 1e-3;
                if parallel && same_side && same_height && overlap {
                    t += 1;
                    continue 'search;
                }
            }
            break;
        }
        tier[i] = t;
        placed.push(i);
    }
    // Each tier steps out by ~1.4 gaps so a label on the inner line clears the outer one.
    tier.iter().map(|&t| t as f32 * gap * 1.4).collect()
}

/// The rotation (radians, clockwise in screen space) that makes a label along direction `dir`
/// always read **left-to-right or bottom-to-top** (#322): the angle is normalized into
/// `[-90°, 90°)`, so a downward vertical reads upward (−90°) rather than top-to-bottom, and a
/// down-to-the-right slope reads top-left → bottom-right.
pub fn readable_text_angle(dir: glam::Vec2) -> f32 {
    let mut angle = dir.y.atan2(dir.x);
    while angle >= std::f32::consts::FRAC_PI_2 {
        angle -= std::f32::consts::PI;
    }
    while angle < -std::f32::consts::FRAC_PI_2 {
        angle += std::f32::consts::PI;
    }
    angle
}

/// The outward unit perpendicular for an edge's dimension line: the side of the edge facing
/// away from the geometry centroid `center` (#294), so labels sit outside the part.
pub fn dimension_outward(a: glam::Vec2, b: glam::Vec2, center: glam::Vec2) -> glam::Vec2 {
    let seg = b - a;
    let mut perp = glam::Vec2::new(-seg.y, seg.x).normalize_or_zero();
    if perp == glam::Vec2::ZERO {
        perp = glam::Vec2::new(0.0, -1.0);
    }
    let mid = (a + b) * 0.5;
    if perp.dot(mid - center) < 0.0 {
        perp = -perp;
    }
    perp
}

/// Projected 2D geometry for a drawing view under its display style (#301), shared by the
/// editor pane and the SVG/PDF export.
pub struct StyledViewGeometry {
    /// Back-to-front shaded faces — `Shaded` only. Each is one coplanar run of triangles so
    /// a renderer can paint it as a single seamless surface (#1651); painting the triangles
    /// one at a time leaves the tessellation's diagonals showing as hairline seams.
    pub faces: Vec<ShadedFace>,
    /// The edge segments to stroke: every feature edge for `Wireframe`; only the visible
    /// runs (hidden lines removed) for `Visible`/`Shaded`.
    pub segments: Vec<(glam::Vec2, glam::Vec2)>,
    /// A sectioned view's hatch lines (#1689), kept apart from the edges so they can stroke
    /// thinner than them (#1784) — the hatch is a fill texture, not geometry.
    pub hatch: Vec<(glam::Vec2, glam::Vec2)>,
    /// Colored-pencil shading (#1821): the strokes laid across each face to give it its tone,
    /// and the shadows the solids drop on one another. Empty for every other style.
    pub shading: Vec<ShadingStroke>,
    /// Whether these fills are a colored-pencil *ground* tone rather than a shaded surface
    /// (#1825): the body's own color, one value on every side, meant to sit a long way toward
    /// the paper so the scribble over it is what reads. Which way "toward the paper" goes is
    /// the renderer's to decide — white on the print, the sheet's own dark on the editor —
    /// so the tint travels unmixed and each surface maps it.
    pub scribbled: bool,
    /// The color to stroke the edges in (#1821), or `None` for the usual ink. A colored
    /// pencil draws its outlines in a deepened version of what it filled with, the way the
    /// viewport's mode does — but only when the view shows one color: edges come from the
    /// merged mesh, so a two-color assembly has no one answer, and ink is the honest one.
    pub stroke_tint: Option<[u8; 3]>,
}

/// One colored-pencil stroke laid on a face (#1821). Carries a `tint` and a `shade` like
/// [`ShadedFace`], so the sheet and the print map it with the formula they already use for
/// fills — a stroke is just a darker patch of the same color.
pub struct ShadingStroke {
    pub a: glam::Vec2,
    pub b: glam::Vec2,
    pub tint: [u8; 3],
    pub shade: f32,
    /// Which of [`StyledViewGeometry::faces`] this mark was laid on (#1840). It has to be
    /// painted with that face rather than after all of them: a plate's scribble laid down
    /// last lands on top of the block standing on the plate.
    pub over: usize,
    /// Stroke width. In the same device units the other strokes use, unless `on_sheet`
    /// (#1829/#1840). A pencil scribble is a line; a wash lays broader marks, and its drying
    /// rim broader still.
    pub width: f32,
    /// Whether `width` is in the view's own millimetres rather than device units (#1840), so
    /// a renderer scales it with the view. The side of a pencil covers a *band of the
    /// drawing*: at a fixed device width the passes pile up on top of each other when the
    /// view is small and leave stripes when it is large.
    pub on_sheet: bool,
}

/// One thing a renderer puts on the page, in [`StyledViewGeometry::painted`] order (#1840).
pub enum PaintedMark<'a> {
    /// A face's fill.
    Fill(&'a ShadedFace),
    /// A mark laid on the face that came before it.
    Stroke(&'a ShadingStroke),
}

impl StyledViewGeometry {
    /// The fills and the marks laid on them, in the order they go on the page (#1840): each
    /// face, then its own strokes. A renderer must not paint every fill and *then* every
    /// mark — a face's marks have to go down before whatever stands in front of that face
    /// covers it, or a plate's scribble lands on top of the block standing on the plate.
    pub fn painted(&self) -> Vec<PaintedMark<'_>> {
        let mut by_face: Vec<Vec<&ShadingStroke>> = vec![Vec::new(); self.faces.len()];
        for stroke in &self.shading {
            if let Some(bucket) = by_face.get_mut(stroke.over) {
                bucket.push(stroke);
            }
        }
        let mut out = Vec::with_capacity(self.faces.len() + self.shading.len());
        for (i, face) in self.faces.iter().enumerate() {
            out.push(PaintedMark::Fill(face));
            out.extend(by_face[i].iter().map(|s| PaintedMark::Stroke(s)));
        }
        out
    }
}

/// One coplanar run of a shaded view's front faces, with the grey it's painted in
/// (0..1, 1 = white).
pub struct ShadedFace {
    /// Projected 2D triangles, all sharing one plane of the solid.
    pub tris: Vec<[glam::Vec2; 3]>,
    pub shade: f32,
    /// The color `shade` scales (#1807). White for `Shaded`, which is how it stays grey;
    /// the body's own material color for `Colorful`.
    pub tint: [u8; 3],
    /// The world plane this flat lies in (#1820): outward normal, and `n · p` for any point
    /// on it. The paint order is a depth sort over these, so keeping them lets a consumer —
    /// and the regression test — recover which surface really is in front at a given point.
    pub plane: (Vec3, f32),
}

/// Back-to-front paint order for a set of coplanar flats (#1820).
///
/// A depth sort alone only orders flats that don't overlap on the page: one flat spans a range
/// of depths, so a big face's farthest point can sit behind a small face the big one actually
/// covers, and the small face paints over it. This takes the depth-sorted order as a starting
/// point and repairs it — for every overlapping pair it samples where the two meet, asks each
/// flat's own plane how deep it is there, and records "this one must be painted first". A
/// topological sort then honours those constraints, falling back on the depth order for pairs
/// that never meet and for any cycle (two flats that genuinely pass through each other, which
/// no single order can draw correctly anyway).
///
/// `flats` arrives in the fallback order; the returned indices point back into it.
fn painter_order(flats: &[ShadedFace], right: Vec3, up: Vec3, toward: Vec3) -> Vec<usize> {
    let n = flats.len();
    if n < 2 {
        return (0..n).collect();
    }
    let bbox = |tris: &[[glam::Vec2; 3]]| {
        let mut lo = glam::Vec2::splat(f32::INFINITY);
        let mut hi = glam::Vec2::splat(f32::NEG_INFINITY);
        for t in tris {
            for p in t {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        (lo, hi)
    };
    let boxes: Vec<(glam::Vec2, glam::Vec2)> = flats.iter().map(|f| bbox(&f.tris)).collect();
    let inside = |tris: &[[glam::Vec2; 3]], p: glam::Vec2| {
        tris.iter().any(|t| {
            let area2 = (t[1] - t[0]).perp_dot(t[2] - t[0]);
            if area2.abs() < 1e-9 {
                return false;
            }
            let w0 = (t[1] - p).perp_dot(t[2] - p) / area2;
            let w1 = (t[2] - p).perp_dot(t[0] - p) / area2;
            let w2 = 1.0 - w0 - w1;
            w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
        })
    };
    // How deep this flat's plane is under a point on the page. The view basis is orthonormal,
    // so a page point `(u, v)` is the world ray `u·right + v·up + d·toward`; solving the plane
    // equation for `d` gives the depth. Front-facing flats have `n · toward > 0`.
    let depth_at = |(nrm, c): (Vec3, f32), p: glam::Vec2| {
        let denom = nrm.dot(toward);
        (denom.abs() > 1e-6)
            .then(|| (c - p.x * nrm.dot(right) - p.y * nrm.dot(up)) / denom)
    };
    /// Sample grid across the overlap of two flats' bounds. Coarse on purpose: it only has to
    /// find *a* point the two share, not measure the overlap. Vertices and centroids of each
    /// flat fill in what a regular grid misses — a thin isometric sliver of a big cut face
    /// behind a smaller body in front (#1908).
    const GRID: usize = 6;
    // `before[i]` = the flats that must be painted before `i`.
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let (lo, hi) = (boxes[i].0.max(boxes[j].0), boxes[i].1.min(boxes[j].1));
            if lo.x > hi.x || lo.y > hi.y {
                continue; // never meet on the page
            }
            // The sample where the two are furthest apart in depth decides: a shared edge
            // (where they meet exactly) says nothing about which is in front.
            let mut best = 0.0f32;
            let mut front = i;
            let consider = |p: glam::Vec2, best: &mut f32, front: &mut usize| {
                if !inside(&flats[i].tris, p) || !inside(&flats[j].tris, p) {
                    return;
                }
                let (Some(di), Some(dj)) =
                    (depth_at(flats[i].plane, p), depth_at(flats[j].plane, p))
                else {
                    return;
                };
                if (di - dj).abs() > *best {
                    *best = (di - dj).abs();
                    *front = if di > dj { i } else { j };
                }
            };
            for gy in 0..=GRID {
                for gx in 0..=GRID {
                    consider(
                        glam::Vec2::new(
                            lo.x + (hi.x - lo.x) * gx as f32 / GRID as f32,
                            lo.y + (hi.y - lo.y) * gy as f32 / GRID as f32,
                        ),
                        &mut best,
                        &mut front,
                    );
                }
            }
            for flat in [i, j] {
                for t in &flats[flat].tris {
                    consider((t[0] + t[1] + t[2]) / 3.0, &mut best, &mut front);
                    for p in t {
                        consider(*p, &mut best, &mut front);
                    }
                }
            }
            if best <= 0.0 {
                continue; // touching, or no shared sample — leave them to the depth order
            }
            let back = if front == i { j } else { i };
            edges[back].push(front);
            indegree[front] += 1;
        }
    }
    // Kahn's algorithm, always taking the lowest-numbered ready flat so the depth order breaks
    // every tie — the result is deterministic, and identical to the depth sort when nothing
    // overlaps.
    let mut out = Vec::with_capacity(n);
    let mut done = vec![false; n];
    for _ in 0..n {
        let Some(next) = (0..n).find(|&i| !done[i] && indegree[i] == 0) else {
            break; // a cycle: interpenetrating flats, which no order draws correctly
        };
        done[next] = true;
        out.push(next);
        for &to in &edges[next] {
            indegree[to] -= 1;
        }
    }
    out.extend((0..n).filter(|&i| !done[i]));
    out
}

/// One triangle of a view's solid, projected onto the page with a depth at each corner.
struct ProjTri {
    p: [glam::Vec2; 3],
    d: [f32; 3],
    /// Twice the signed area of the projected triangle; ~0 = edge-on, skipped.
    area2: f32,
}

/// How far outside a projected triangle a point may sit and still count as inside, in
/// barycentric units (#1713) — enough to close the seam between two triangles of one
/// flat, far too little to reach across a real gap.
const BARY_TOL: f32 = 1e-5;

/// How many places along an edge visibility is sampled before the run boundaries are honed
/// (#1841). The samples find every stretch the solid hides; the bisection below pins where
/// each one starts and ends.
const OCCLUSION_SAMPLES: usize = 32;
/// Bisection steps run at each visibility change, halving the interval each time: 2⁻²⁰ of an
/// edge is far finer than any page can show.
const OCCLUSION_REFINE_STEPS: usize = 20;

/// A drawing view's own solid, projected, as a test for whether a point of the model is
/// hidden behind it — the hidden-line removal every style but Wireframe does (#1713).
pub struct ViewOcclusion {
    tris: Vec<ProjTri>,
    right: Vec3,
    up: Vec3,
    /// Depth grows toward the viewer along this axis.
    toward: Vec3,
    /// Depth slack, scaled to the model, so a face doesn't hide the edge lying on it.
    eps: f32,
}

impl ViewOcclusion {
    /// The occluder for `view`, or `None` when the view has no solid (a sketch view, or
    /// bodies that don't mesh). Test-only: the renderers build theirs inside
    /// [`styled_view_geometry`], from the mesh they already have.
    #[cfg(test)]
    pub fn for_view(doc: &Document, views: &[DrawingView], view: &DrawingView) -> Option<Self> {
        let (right, up) = resolved_view_axes(views, view);
        Self::from_mesh(&drawing_view_solid_mesh(doc, view)?, right, up)
    }

    fn from_mesh(mesh: &crate::extrude::SolidMesh, right: Vec3, up: Vec3) -> Option<Self> {
        let toward = right.cross(up);
        let (lo, hi) = mesh.bounds()?;
        let eps = (hi - lo).length().max(1e-3) * 2e-3;
        let tris = mesh
            .triangles
            .iter()
            .map(|t| {
                let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
                let p = [project(t[0]), project(t[1]), project(t[2])];
                let area2 = (p[1] - p[0]).perp_dot(p[2] - p[0]);
                ProjTri { p, d: [t[0].dot(toward), t[1].dot(toward), t[2].dot(toward)], area2 }
            })
            .filter(|t| t.area2.abs() > 1e-6)
            .collect();
        Some(ViewOcclusion { tris, right, up, toward, eps })
    }

    /// Where a world point lands on the page.
    pub fn project(&self, p: Vec3) -> glam::Vec2 {
        glam::Vec2::new(p.dot(self.right), p.dot(self.up))
    }

    /// Whether some face of the solid is strictly in front of the world point `p`.
    pub fn hides(&self, p: Vec3) -> bool {
        self.hides_page(self.project(p), p.dot(self.toward))
    }

    fn hides_page(&self, point: glam::Vec2, depth: f32) -> bool {
        self.tris.iter().any(|t| {
            // Barycentric coordinates of `point` in the projected triangle.
            let w0 = (t.p[1] - point).perp_dot(t.p[2] - point) / t.area2;
            let w1 = (t.p[2] - point).perp_dot(t.p[0] - point) / t.area2;
            let w2 = 1.0 - w0 - w1;
            // A point on the seam between two triangles of the same flat belongs to both;
            // float error can put it a hair outside *both*, leaving a crack the hidden edge
            // shows through — a stub of a hidden line poking out of a solid (#1713). A sliver
            // of tolerance closes the seam.
            if w0 < -BARY_TOL || w1 < -BARY_TOL || w2 < -BARY_TOL {
                return false;
            }
            w0 * t.d[0] + w1 * t.d[1] + w2 * t.d[2] > depth + self.eps
        })
    }

    /// The stretches of the world segment `a`–`b` the solid leaves visible, as `t` ranges.
    ///
    /// Sampling alone puts each boundary on a thirty-second of the edge, which reads as a
    /// line overrunning the block that hides it (#1841); each change of visibility between
    /// two samples is bisected down to where the solid's outline really crosses.
    pub fn visible_runs(&self, a: Vec3, b: Vec3) -> Vec<(f32, f32)> {
        let visible = |t: f32| {
            let p = a.lerp(b, t);
            !self.hides(p)
        };
        let refine = |lo: f32, hi: f32, lo_visible: bool| {
            let (mut lo, mut hi) = (lo, hi);
            for _ in 0..OCCLUSION_REFINE_STEPS {
                let mid = (lo + hi) * 0.5;
                if visible(mid) == lo_visible {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) * 0.5
        };
        let mut runs = Vec::new();
        let mut run_start: Option<f32> = None;
        let mut prev: Option<(f32, bool)> = None;
        for i in 0..OCCLUSION_SAMPLES {
            let t = (i as f32 + 0.5) / OCCLUSION_SAMPLES as f32;
            let vis = visible(t);
            match (vis, run_start) {
                // The first sample stands for the edge's start: nothing before it to bisect.
                (true, None) => {
                    run_start = Some(match prev {
                        Some((pt, pv)) => refine(pt, t, pv),
                        None => 0.0,
                    })
                }
                (false, Some(s)) => {
                    let end = match prev {
                        Some((pt, pv)) => refine(pt, t, pv),
                        None => 0.0,
                    };
                    runs.push((s, end));
                    run_start = None;
                }
                _ => {}
            }
            prev = Some((t, vis));
        }
        if let Some(s) = run_start {
            runs.push((s, 1.0));
        }
        runs
    }
}

/// Project a view's geometry under its display style (#301). Sketch views have no solid to
/// occlude or shade, so they always render as plain wireframe.
pub fn styled_view_geometry(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> StyledViewGeometry {
    use crate::model::DrawingViewStyle;
    let (right, up) = resolved_view_axes(views, view);
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    // Crease edges plus the view-dependent silhouette (#319) so smooth-surface outlines (a
    // cylinder's straight sides) are stroked; circle detection/dimensioning use crease edges
    // only, so the silhouette here doesn't affect them.
    let crease_edges = drawing_view_world_edges(doc, view);
    let mut edges = crease_edges.clone();
    edges.extend(drawing_view_silhouette_edges(doc, views, view));
    // A sectioned view hatches the faces its planes opened (#1689). The hatch travels in its
    // own field rather than with the stroked edges (#1784) — it draws thinner — and apart
    // from `drawing_view_world_edges`, so dimensioning and circle detection still see only
    // real geometry. Hidden-line styles still run it through the same occlusion pass as
    // the edges (#1908): otherwise the hash marks (and the cut they sit on) read through
    // a body standing in front of the cut.
    // A pencil view's cut is drawn by the same hand as the rest of it (#1827): a perfectly
    // ruled hatch in the middle of a hand-drawn view reads as a machine drawing pasted in.
    // The wobble is held well under the hatch spacing, or the lines cross and the face fills
    // in solid. Clip in world space first, then wobble the surviving runs.
    let hatch_world = section_hatch_world_segments(doc, view);
    let hatch_from = |segments: &[(Vec3, Vec3)]| -> Vec<(glam::Vec2, glam::Vec2)> {
        let ruled: Vec<(glam::Vec2, glam::Vec2)> =
            segments.iter().map(|(a, b)| (project(*a), project(*b))).collect();
        if view.style.is_pencil() {
            let wobble = crate::extrude::SECTION_HATCH_SPACING_MM
                * crate::pencil::RULED_WOBBLE_OF_SPACING;
            ruled
                .iter()
                .flat_map(|(a, b)| {
                    let points = crate::pencil::stroke_2d_within(*a, *b, 0, wobble);
                    points
                        .windows(2)
                        .map(|w| (w[0], w[1]))
                        .collect::<Vec<_>>()
                })
                .collect()
        } else {
            ruled
        }
    };
    let mut hatch = hatch_from(&hatch_world);
    let wireframe = || StyledViewGeometry {
        faces: Vec::new(),
        segments: edges.iter().map(|(a, b)| (project(*a), project(*b))).collect(),
        hatch: hatch.clone(),
        shading: Vec::new(),
        scribbled: false,
        stroke_tint: None,
    };
    if view.sketch.is_some() || view.style == DrawingViewStyle::Wireframe {
        return wireframe();
    }
    // Both shaded styles fill faces; the pencil style draws the same visible edges as
    // `Visible`, by hand (#1809).
    let shades_faces = matches!(view.style, DrawingViewStyle::Shaded | DrawingViewStyle::Colorful)
        || view.style.is_hand_colored();
    // Per body, so `Colorful` can keep each one's material color (#1807); the merged mesh
    // is what the occlusion test and the grey styles work from, exactly as before.
    let bodies = drawing_view_body_meshes(doc, view);
    let Some(mesh) = drawing_view_solid_mesh(doc, view) else {
        return wireframe();
    };
    // Depth grows toward the viewer along the view's out-of-page axis.
    let toward = right.cross(up);
    let Some(occlusion) = ViewOcclusion::from_mesh(&mesh, right, up) else {
        return wireframe();
    };

    // A detected circle (#313) is drawn as one smooth outline rather than the tessellated
    // polygon the mesh carries. The renderers used to stroke that outline whole, over
    // whatever stood in front of it — so a bored plate showed the far rim of its hole in
    // full (#1841/#1842). The rim is refined here instead, in world space, and goes through
    // the hidden-line pass with every other edge; the renderers leave the whole outline to
    // the styles that draw hidden lines anyway (see [`view_strokes_whole_circles`]).
    for c in classify_world_circles(&crease_edges) {
        edges.retain(|(a, b)| !world_segment_on_circle(*a, *b, &c));
        let pts = world_circle_points(&c, SMOOTH_CIRCLE_SEGMENTS);
        edges.extend(pts.windows(2).map(|w| (w[0], w[1])));
    }

    // Keep the visible run of each edge (hidden-line removal).
    let mut segments: Vec<(glam::Vec2, glam::Vec2)> = Vec::new();
    for (a, b) in &edges {
        for (from, to) in occlusion.visible_runs(*a, *b) {
            segments.push((project(a.lerp(*b, from)), project(a.lerp(*b, to))));
        }
    }
    // Same pass for the hatch (#1908). It is a fill texture, not geometry, but it still
    // sits in the world on the cut face — a body in front of that face has to hide it.
    let visible_hatch: Vec<(Vec3, Vec3)> = hatch_world
        .iter()
        .flat_map(|(a, b)| {
            occlusion
                .visible_runs(*a, *b)
                .into_iter()
                .map(|(from, to)| (a.lerp(*b, from), a.lerp(*b, to)))
        })
        .collect();
    hatch = hatch_from(&visible_hatch);

    // Shaded fills: front faces painted back-to-front, greyed by how squarely they face a
    // fixed key light up-and-left of the viewer. Coplanar triangles are gathered into one
    // face (#1651) so a renderer paints each flat as a single surface — drawn one by one
    // they leave the tessellation's diagonals showing between them.
    let mut fills = Vec::new();
    if shades_faces {
        let light = (toward * 1.2 - right * 0.35 + up * 0.55).normalize();
        // Plane key: the outward normal and the plane's distance from the origin, both
        // quantized so a tessellator's per-triangle rounding still lands on one flat.
        // The plane key carries the tint, so two bodies that happen to share a plane still
        // paint as two faces in their own colors (#1807).
        struct Flat {
            key: ([i32; 4], [u8; 3]),
            /// The flat's farthest point along `toward` — the initial (and tie-break) order.
            depth: f32,
            shade: f32,
            tris: Vec<[glam::Vec2; 3]>,
            /// World plane: outward normal and `n · p`.
            plane: (Vec3, f32),
        }
        let mut planes: Vec<Flat> = Vec::new();
        for (body_mesh, tint) in &bodies {
            for t in &body_mesh.triangles {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                if n == Vec3::ZERO || n.dot(toward) <= 0.0 {
                    continue; // back or degenerate face
                }
                let q = |v: f32| (v * 1000.0).round() as i32;
                let key = ([q(n.x), q(n.y), q(n.z), q(n.dot(t[0]) * 0.1)], *tint);
                // Colored pencil takes one value on every side (#1825): its ground tone is
                // the body's own color taken toward the paper, not a lit surface, so the key
                // light plays no part in it.
                let shade = if view.style.is_hand_colored() {
                    1.0
                } else {
                    0.62 + 0.33 * n.dot(light).max(0.0)
                };
                let depth = (t[0] + t[1] + t[2]).dot(toward) / 3.0;
                let tri = [project(t[0]), project(t[1]), project(t[2])];
                match planes.iter_mut().find(|f| f.key == key) {
                    Some(f) => {
                        f.depth = f.depth.min(depth);
                        f.tris.push(tri);
                    }
                    None => planes.push(Flat {
                        key,
                        depth,
                        shade,
                        tris: vec![tri],
                        plane: (n, n.dot(t[0])),
                    }),
                }
            }
        }
        // Farthest flat first. One depth per flat only orders flats that don't overlap on the
        // page: a big face spans a range of depths, so its farthest point can sit behind a
        // small face it actually covers — that is how a bar's shaded side leaked through onto
        // the top of the solid it grows out of (#1820). So sort by depth for a deterministic
        // starting order, then reorder every overlapping *pair* by which one is really in
        // front where they meet.
        planes.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let mut faces: Vec<ShadedFace> = planes
            .into_iter()
            .map(|f| ShadedFace { tris: f.tris, shade: f.shade, tint: f.key.1, plane: f.plane })
            .collect();
        let order = painter_order(&faces, right, up, toward);
        let mut slots: Vec<Option<ShadedFace>> = faces.drain(..).map(Some).collect();
        fills = order.into_iter().filter_map(|i| slots[i].take()).collect();
    }

    // Loose pencil (#1809): the same visible edges, drawn the way a hand draws them —
    // overshooting each corner, bowing along the way, and gone over twice. The wobble is
    // keyed to the segment's own endpoints, so a view redraws identically every time.
    if view.style.is_pencil() {
        let mut drawn = Vec::with_capacity(segments.len() * crate::pencil::PENCIL_PASSES * 5);
        for (a, b) in &segments {
            for pass in 0..crate::pencil::PENCIL_PASSES {
                let points = crate::pencil::stroke_2d(*a, *b, pass);
                drawn.extend(points.windows(2).map(|w| (w[0], w[1])));
            }
        }
        segments = drawn;
    }

    // Colored pencil (#1821/#1825): the same hand as the viewport's mode — the color
    // *scribbled* across each flat, run a little past its outline and broken by gaps of bare
    // paper, plus the shadows the solids drop on one another. One density and one tone on
    // every side, whichever way it faces: a colored pencil drawing gets its form from its
    // outlines, exactly as the plain pencil style does.
    let mut shading = Vec::new();
    if view.style.is_hand_colored() {
        let light = (toward * 1.2 - right * 0.35 + up * 0.55).normalize();
        // Everything the light can reach, for the shadows the flats receive.
        let casters: Vec<[Vec3; 3]> = mesh
            .triangles
            .iter()
            .filter(|t| (t[1] - t[0]).cross(t[2] - t[0]).dot(light) > 0.0)
            .copied()
            .collect();
        // Hatching happens flat on the page: the flats are already projected there, and a
        // drawing's stroke spacing belongs to the sheet rather than to the model. A view is
        // scaled to fit its card, so a spacing fixed in millimetres of the *part* lays a
        // dense scribble on a small one and a few stray lines across a big one (#1840) —
        // take it from how big the view is instead, and a hand covers every part the same.
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
        for p in fills.iter().flat_map(|f| f.tris.iter()).flat_map(|t| t.iter()) {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let across = if lo.is_finite() && hi.is_finite() { (hi - lo).length() } else { 100.0 };
        let spacing = |mm: f32| {
            (across / SCRIBBLE_LINES_ACROSS * (mm / crate::pencil::PENCIL_SCRIBBLE_SPACING_MM))
                .clamp(0.3, 6.0)
        };
        let page = crate::pencil::HatchFrame { origin: Vec3::ZERO, u: Vec3::X, v: Vec3::Y };
        let lift = |tris: &[[glam::Vec2; 3]]| -> Vec<[Vec3; 3]> {
            tris.iter()
                .map(|t| std::array::from_fn(|i| Vec3::new(t[i].x, t[i].y, 0.0)))
                .collect()
        };
        for (fi, face) in fills.iter().enumerate() {
            let (n, c) = face.plane;
            // Read off the body's own color before the fills are lightened below.
            let body = eframe::egui::Color32::from_rgb(face.tint[0], face.tint[1], face.tint[2]);
            let laid_on = crate::pencil::shading_tone(body);
            let stroke_tint = [laid_on.r(), laid_on.g(), laid_on.b()];
            // A stable turn per flat, so two faces meeting at an edge are not scribbled in
            // lockstep — keyed to the plane, so a view redraws identically every time.
            let mut h = 0x811C_9DC5u32;
            for v in [n.x, n.y, n.z, c] {
                h = (h ^ (v * 1000.0).round() as i32 as u32).wrapping_mul(0x0100_0193);
            }
            let turn = (h >> 8) as f32 / (1 << 24) as f32 * std::f32::consts::PI;
            let flat = lift(&face.tris);
            let wash = view.style == DrawingViewStyle::Watercolor;
            let push = |out: &mut Vec<ShadingStroke>,
                        segments: Vec<(Vec3, Vec3)>,
                        tint: [u8; 3],
                        shade: f32,
                        width: f32,
                        broken: bool| {
                for (a, b) in segments {
                    // A pencil lifts and re-lands; a brush carries the mark right through.
                    let pieces: Vec<(Vec3, Vec3)> = if broken {
                        crate::pencil::scribble(a, b, 0)
                    } else {
                        crate::pencil::pooling(a, b, 0)
                    };
                    for (from, to) in pieces {
                        let points = crate::pencil::stroke_inside(from, to, 0);
                        out.extend(points.windows(2).map(|w| ShadingStroke {
                            a: glam::Vec2::new(w[0].x, w[0].y),
                            b: glam::Vec2::new(w[1].x, w[1].y),
                            tint,
                            shade,
                            width,
                            on_sheet: false,
                            over: fi,
                        }));
                    }
                }
            };
            if wash {
                // Pooling within the wash, and the rim it dried to against every edge.
                let pool = crate::pencil::wash_pool_tone(body);
                push(
                    &mut shading,
                    crate::pencil::hatch_in_frame(
                        &page,
                        crate::pencil::WASH_POOL_SPACING_MM,
                        crate::pencil::PENCIL_HATCH_ANGLE_RAD + turn,
                        &flat,
                        None,
                    ),
                    [pool.r(), pool.g(), pool.b()],
                    1.0,
                    WASH_POOL_STROKE,
                    false,
                );
                let rim = crate::pencil::wash_edge_tone(body);
                for (a, b) in flat_boundary_edges_2d(&face.tris) {
                    for pass in 0..crate::pencil::WASH_EDGE_PASSES {
                        let points = crate::pencil::stroke(
                            Vec3::new(a.x, a.y, 0.0),
                            Vec3::new(b.x, b.y, 0.0),
                            pass,
                        );
                        shading.extend(points.windows(2).map(|w| ShadingStroke {
                            a: glam::Vec2::new(w[0].x, w[0].y),
                            b: glam::Vec2::new(w[1].x, w[1].y),
                            tint: [rim.r(), rim.g(), rim.b()],
                            shade: 1.0,
                            width: WASH_EDGE_STROKE,
                            on_sheet: false,
                            over: fi,
                        }));
                    }
                }
            } else {
                // Colored pencil lays its color in with the **side** of the lead (#1840),
                // not its point: broad passes across the flat, each at its own pressure,
                // one straight mark apiece. A print cannot stack translucent passes the way
                // the viewport does, so pressure shows as a paler tone rather than a
                // lighter touch — `pressed` mixes it back toward the ground it lies on.
                let ground = crate::pencil::scribble_ground(body);
                let pitch = spacing(crate::pencil::PENCIL_SIDE_SPACING_MM);
                for (a, b) in crate::pencil::hatch_in_frame(
                    &page,
                    pitch,
                    crate::pencil::PENCIL_HATCH_ANGLE_RAD + turn,
                    &flat,
                    None,
                ) {
                    for (from, to, pressure) in crate::pencil::side_shading(a, b, 0) {
                        // Opaque marks cannot build tone by lying over one another the way
                        // the viewport's translucent passes do, so the pressure has to be in
                        // the color: each pass is the laid-on tone let back toward the ground.
                        let tone =
                            crate::pencil::pressed(laid_on, ground, pressure * SIDE_STRENGTH);
                        shading.push(ShadingStroke {
                            a: glam::Vec2::new(from.x, from.y),
                            b: glam::Vec2::new(to.x, to.y),
                            tint: [tone.r(), tone.g(), tone.b()],
                            shade: 1.0,
                            width: pitch * SIDE_STROKE_OF_PITCH,
                            on_sheet: true,
                            over: fi,
                        });
                    }
                }
            }
            // What stands between this flat and the light, dropped onto its plane and clipped
            // to the flat — the drawings-page half of the viewport's cast shadows (#1818).
            let facing = n.dot(light);
            if facing > 0.2 {
                let cast: Vec<[Vec3; 3]> = casters
                    .iter()
                    .filter_map(|t| {
                        let above = t.map(|p| n.dot(p) - c);
                        (above.iter().any(|d| *d >= SHADOW_CASTER_MIN_MM)).then(|| {
                            let on: [Vec3; 3] =
                                std::array::from_fn(|i| t[i] - light * (above[i].max(0.0) / facing));
                            std::array::from_fn(|i| {
                                let p = project(on[i]);
                                Vec3::new(p.x, p.y, 0.0)
                            })
                        })
                    })
                    .collect();
                push(
                    &mut shading,
                    crate::pencil::hatch_in_frame(
                        &page,
                        spacing(crate::pencil::PENCIL_CAST_SPACING_MM),
                        crate::pencil::PENCIL_HATCH_ANGLE_RAD
                            + turn
                            + crate::pencil::PENCIL_CAST_TURN_RAD,
                        &cast,
                        Some(flat.as_slice()),
                    ),
                    stroke_tint,
                    PENCIL_SHADOW_SHADE,
                    SCRIBBLE_STROKE,
                    true,
                );
            }
        }
        // The ground under whatever is laid on it. One value on every side (#1825): the light
        // term goes entirely. Each style mixes its own — a colored pencil only grazes the
        // paper, a wash covers — and the tint carries the result, so both renderers read one
        // print color rather than each re-deriving it (#1829).
        for face in &mut fills {
            let body = eframe::egui::Color32::from_rgb(face.tint[0], face.tint[1], face.tint[2]);
            let ground = if view.style == DrawingViewStyle::Watercolor {
                crate::pencil::wash_tone(body)
            } else {
                crate::pencil::scribble_ground(body)
            };
            face.tint = [ground.r(), ground.g(), ground.b()];
            face.shade = 1.0;
        }
    }

    // A colored pencil draws its outline in a deepened version of what it filled with    // A colored pencil draws its outline in a deepened version of what it filled with
    // (#1812/#1821) — but only when there is one color to deepen.
    let stroke_tint = (view.style == DrawingViewStyle::ColorPencil)
        .then(|| {
            let mut tints = bodies.iter().map(|(_, t)| *t);
            let first = tints.next()?;
            tints.all(|t| t == first).then(|| {
                let deep = crate::pencil::color_tones(eframe::egui::Color32::from_rgb(
                    first[0], first[1], first[2],
                ))
                .1;
                [deep.r(), deep.g(), deep.b()]
            })
        })
        .flatten();

    StyledViewGeometry {
        faces: fills,
        segments,
        hatch,
        shading,
        scribbled: view.style.is_hand_colored(),
        stroke_tint,
    }
}

/// Stroke widths for the watercolor wash on the page (#1829), in the same device units the
/// hatch uses: the pooling lays a broad mark, and the rim it dries to a broader one.
const WASH_POOL_STROKE: f32 = HATCH_STROKE * 3.5;
const WASH_EDGE_STROKE: f32 = HATCH_STROKE * 2.0;

/// The boundary of one projected flat (#1829): the edges only one of its triangles owns, which
/// is the outline a wash gathers against as it dries. The 2D twin of the viewport's
/// `flat_boundary_edges`, on the page rather than in the world.
fn flat_boundary_edges_2d(tris: &[[glam::Vec2; 3]]) -> Vec<(glam::Vec2, glam::Vec2)> {
    let key = |p: glam::Vec2| [(p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64];
    let mut seen: std::collections::HashMap<([i64; 2], [i64; 2]), (glam::Vec2, glam::Vec2, u32)> =
        std::collections::HashMap::new();
    for tri in tris {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            let (ka, kb) = (key(a), key(b));
            let k = if ka <= kb { (ka, kb) } else { (kb, ka) };
            seen.entry(k).or_insert((a, b, 0)).2 += 1;
        }
    }
    let mut out: Vec<(glam::Vec2, glam::Vec2)> =
        seen.into_values().filter(|(_, _, n)| *n == 1).map(|(a, b, _)| (a, b)).collect();
    // Deterministic: the map hands them back in an arbitrary order, and a wobble keyed to the
    // endpoints must be laid down in a stable order for the page to redraw identically.
    out.sort_by(|x, y| {
        (x.0.x, x.0.y, x.1.x, x.1.y)
            .partial_cmp(&(y.0.x, y.0.y, y.1.x, y.1.y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// How much darker a colored-pencil cast shadow is than the scribble it lies among (#1821).
/// It rides the same `tint`/`shade` pair the fills use, so the sheet and the print map it with
/// the formula they already have.
const PENCIL_SHADOW_SHADE: f32 = 0.62;
/// How far above a flat a triangle must stand to shadow it — the contact face itself must not.
const SHADOW_CASTER_MIN_MM: f32 = 0.05;

/// An 8-bit RGB paint.
#[derive(Clone, Copy, PartialEq)]
struct Rgb(u8, u8, u8);
const BLACK: Rgb = Rgb(0, 0, 0);
const WHITE: Rgb = Rgb(255, 255, 255);

/// Horizontal text alignment relative to the given `x`.
#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
    End,
}

/// A 2D vector-drawing sink in top-left (SVG-style) coordinates: `y` grows downward and text
/// `y` is the baseline. The PDF backend flips to bottom-left internally. Both backends render
/// the same [`render_drawing`] output.
trait Canvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32);
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32);
    /// A dashed line — aligned-view projection lines (#377). Backends override; the default
    /// falls back to a solid stroke.
    fn line_dashed(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.line(x1, y1, x2, y2, color, width);
    }
    /// A filled polygon (shaded view faces, #301).
    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb);
    /// A stroked (unfilled) circle outline — a smooth detected curve (#313).
    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32);
    /// Letter the text that follows by hand, or back in the drawing's usual sans (#1830).
    /// A pencil view's caption and dimensions are set in the bundled Klee One; every other
    /// style keeps the sans, and a backend that cannot switch face ignores this.
    fn set_hand_lettered(&mut self, hand_lettered: bool) {
        let _ = hand_lettered;
    }
    /// Text is always black in a drawing; `size` is the font size in px.
    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str);
    /// Text rotated `angle` radians clockwise about `(x, y)` — dimension labels running along
    /// their dimension line (#314). `(x, y)` is the **visual centre** of the glyphs, matching
    /// the editor's galley-centred `TextShape`, so a label offset off its dimension line
    /// stays off it in the export too (#1350). Backends override; the default draws it unrotated.
    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        let _ = angle;
        self.text(x, y, size, anchor, content);
    }
}

/// Approximate rendered width (device units) of a dimension label in the drawing's Helvetica —
/// ~0.55 em per glyph, matching the PDF backend's centring estimate (#314).
pub fn text_device_width(size: f32, content: &str) -> f32 {
    0.55 * size * content.chars().count() as f32
}

/// Fraction of em from the alphabetic baseline up to the visual centre of capital
/// letters in Helvetica / typical sans (used so PDF/SVG `text_rot` matches the editor's
/// galley-centred labels, #1350).
pub const DIM_LABEL_MID_EM: f32 = 0.35;

/// Where and how to draw a dimension's label (#314): `(pos, angle_radians)`. If the text fits
/// along the dimension line it runs centred along it (rotated, kept upright); otherwise it's
/// placed just beyond the line's far end, horizontal, so it never overlaps the arrows.
/// Everything is in device units (screen px for the editor, points for export).
pub fn dimension_label_layout(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    text_w: f32,
    text_h: f32,
    gap: f32,
) -> (glam::Vec2, f32) {
    let along = b - a;
    let len = along.length();
    let dir = if len > 1e-3 { along / len } else { glam::Vec2::new(1.0, 0.0) };
    let mid = (a + b) * 0.5;
    // The returned point is the label's visual *centre*, so half a glyph has to come out of
    // the gap before the text clears the stroke (#1716). Without it a `gap` of 5 pt put an
    // 11 pt label's lower edge about a point off the line -- close enough to read as sitting
    // on it, worst of all where the label runs alongside the stroke for its whole width.
    let clear = gap + text_h * DIM_LABEL_MID_EM;
    if text_w + gap <= len {
        (mid + outward * clear, readable_text_angle(dir))
    } else {
        // Too short: sit horizontally just past the far end, on the outward side.
        (b + dir * (text_w * 0.5 + gap) + outward * clear, 0.0)
    }
}

/// The page size (width, height) in PDF points for a drawing — its configured mm page (#298),
/// landscape US-Letter by default — or `None` if the index is missing/deleted.
fn page_dims(doc: &Document, index: crate::model::DrawingKey) -> Option<(f32, f32)> {
    let drawing = doc.drawings.get(index)?;
    Some((
        drawing.page_width_mm * PT_PER_MM,
        drawing.page_height_mm * PT_PER_MM,
    ))
}

/// Draw a whole drawing into `canvas`, WYSIWYG with the editor (#297): each view is a card
/// centred at its `pos_x`/`pos_y` page fraction, sized like the editor's cards, on the
/// drawing's configured page. The title sits in the top margin.
fn render_drawing<C: Canvas>(
    doc: &Document,
    index: crate::model::DrawingKey,
    canvas: &mut C,
) -> Option<()> {
    let drawing = doc.drawings.get(index)?;
    let (width, height) = page_dims(doc, index)?;
    let unit = doc.default_length_unit;

    canvas.rect(0.0, 0.0, width, height, Some(WHITE), None, 0.0);
    // The title is a normal, deletable text annotation created with the drawing (#335), rendered
    // in the annotation loop below just like any other note — the export no longer stamps its own
    // title into the top margin (that never appeared in the WYSIWYG editor).

    for (vi, view) in drawing.views.iter().enumerate() {
        // Aligned children (#296) resolve their shared axis to the parent's.
        let (px, py) = resolved_view_pos(doc, index, vi);
        let (sx, sy) = view_size_frac(view);
        let cell_w = width * sx;
        let cell_h = height * sy;
        let cell_x = px * width - cell_w * 0.5;
        let cell_y = py * height - cell_h * 0.5;
        // No card border in exports (#337): the grey rectangle is an editor-only affordance for
        // selecting/dragging a view; a printed drawing shows just the projection and its caption.
        let source = drawing_view_source_label(doc, view);
        // An aligned child inherits its parent's scale (#296/#300).
        let scale_text = resolved_view_scale(doc, index, vi);
        let scale_suffix = scale_text
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        // A pencil view letters its own text by hand (#1830) — caption and dimensions both.
        // A technical caption in the usual clean sans undoes the drawn look under it.
        canvas.set_hand_lettered(view.style.is_pencil());
        // The caption is toggleable, positionable, and its text overridable (#372); custom
        // templates interpolate {expr} fields (#338), same as the editor.
        if !view.label_hidden {
            let label = match &view.label_text {
                Some(t) => crate::value::interpolate_text(t, doc),
                None => format!("{source} — {}{scale_suffix}", view.orientation.label()),
            };
            use crate::model::DrawingLabelPos as LP;
            let (lx, ly, anchor) = match view.label_pos {
                LP::TopLeft => (cell_x + CELL_PAD, cell_y + 20.0, Anchor::Start),
                LP::TopCenter => (cell_x + cell_w * 0.5, cell_y + 20.0, Anchor::Middle),
                LP::TopRight => (cell_x + cell_w - CELL_PAD, cell_y + 20.0, Anchor::End),
                LP::BottomLeft => (cell_x + CELL_PAD, cell_y + cell_h - 8.0, Anchor::Start),
                LP::BottomCenter => {
                    (cell_x + cell_w * 0.5, cell_y + cell_h - 8.0, Anchor::Middle)
                }
                LP::BottomRight => {
                    (cell_x + cell_w - CELL_PAD, cell_y + cell_h - 8.0, Anchor::End)
                }
            };
            canvas.text(lx, ly, 11.0, anchor, &label);
        }
        render_view_geometry(
            canvas,
            doc,
            &drawing.views,
            view,
            vi,
            scale_text.as_deref(),
            cell_x,
            cell_y,
            cell_w,
            cell_h,
            unit,
        );
        canvas.set_hand_lettered(false);
    }

    // Aligned projection lines (#377): dashed, lightweight lines connecting each toggled
    // aligned child's silhouette extremes to its base view's, each endpoint mapped through
    // its own view's transform so the lines land exactly on the rendered geometry.
    for (vi, view) in drawing.views.iter().enumerate() {
        if !view.align_lines {
            continue;
        }
        let Some(lines) = aligned_projection_lines(doc, &drawing.views, vi) else {
            continue;
        };
        let to_screen_for = |v: usize| {
            let (px, py) = resolved_view_pos(doc, index, v);
            let (sx, sy) = drawing
                .views
                .get(v)
                .map(view_size_frac)
                .unwrap_or((CELL_FRAC, CELL_FRAC));
            let cell_w = width * sx;
            let cell_h = height * sy;
            let cell_x = px * width - cell_w * 0.5;
            let cell_y = py * height - cell_h * 0.5;
            let scale_text = resolved_view_scale(doc, index, v);
            let (scale, bbox_center, area_center) = export_view_transform(
                doc,
                &drawing.views,
                v,
                scale_text.as_deref(),
                cell_x,
                cell_y,
                cell_w,
                cell_h,
            );
            move |p: glam::Vec2| {
                let d = (p - bbox_center) * scale;
                glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
            }
        };
        let parent_ts = to_screen_for(view.aligned_parent.unwrap_or(vi));
        let child_ts = to_screen_for(vi);
        for (ppt, cpt) in lines {
            let a = parent_ts(ppt);
            let b = child_ts(cpt);
            canvas.line_dashed(a.x, a.y, b.x, b.y, Rgb(110, 110, 110), 0.6);
        }
    }

    // Free text annotations (#312): wrapped to their box, positioned by page fraction.
    for ann in drawing.annotations.values() {
        let font = (ann.size_frac * height).clamp(4.0, 400.0);
        let x = ann.pos_x * width;
        let y = ann.pos_y * height + font; // baseline of the first line
        let wrap = ann.wrap_frac.map(|w| (w * width).max(font));
        let line_h = font * 1.25;
        // Substitute {expr} variable fields against the document's parameters (#338).
        let rendered = crate::value::interpolate_text(&ann.text, doc);
        for (i, line) in wrap_text_lines(&rendered, font, wrap).iter().enumerate() {
            canvas.text(x, y + i as f32 * line_h, font, Anchor::Start, line);
        }
    }
    Some(())
}

/// Word-wrap `text` to `wrap_width` device units (`None` = no wrap), splitting on explicit
/// newlines too (#312). Uses the same ~0.55em glyph estimate as the PDF centring.
fn wrap_text_lines(text: &str, font: f32, wrap_width: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        match wrap_width {
            None => out.push(para.to_string()),
            Some(w) => {
                let mut line = String::new();
                for word in para.split(' ') {
                    let candidate = if line.is_empty() {
                        word.to_string()
                    } else {
                        format!("{line} {word}")
                    };
                    if !line.is_empty() && text_device_width(font, &candidate) > w {
                        out.push(std::mem::take(&mut line));
                        line = word.to_string();
                    } else {
                        line = candidate;
                    }
                }
                out.push(line);
            }
        }
    }
    out
}

/// The export-canvas transform a view's geometry renders through: `(scale, bbox_center,
/// area_center)`, where a projected point maps to
/// `area_center + ((p - bbox_center) * scale)` with y flipped. Shared by
/// [`render_view_geometry`] and the aligned projection-line pass (#377) so the dashed lines
/// land exactly on the rendered silhouettes.
#[allow(clippy::too_many_arguments)]
fn export_view_transform(
    doc: &Document,
    views: &[DrawingView],
    view_index: usize,
    scale_text: Option<&str>,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
) -> (f32, glam::Vec2, glam::Vec2) {
    // The caption strip takes the top of the card; geometry fits below it.
    let caption_h = 26.0;
    let area_w = cell_w - 2.0 * CELL_PAD;
    let area_h = cell_h - caption_h - 2.0 * CELL_PAD;
    // A set print scale (#300) draws at exactly `factor` page-mm per model-mm (points on the
    // export canvas); otherwise auto-fit to the card.
    let scale = match scale_text.and_then(crate::model::parse_drawing_scale) {
        Some(factor) => factor * PT_PER_MM,
        // Aligned children share their parent's auto-fit scale so edges line up (#364). Use
        // the scale-root's card area when sizes differ (#1207).
        None => {
            let root = aligned_scale_root(views, view_index);
            let (aw, ah) = if root == view_index {
                (area_w, area_h)
            } else if let Some(rv) = views.get(root) {
                let (sx, sy) = view_size_frac(rv);
                // Approximate page size from this view's cell + its own size fraction.
                let page_w = cell_w / view_size_frac(&views[view_index]).0.max(1e-6);
                let page_h = cell_h / view_size_frac(&views[view_index]).1.max(1e-6);
                let rw = page_w * sx - 2.0 * CELL_PAD;
                let rh = page_h * sy - caption_h - 2.0 * CELL_PAD;
                (rw.max(1e-3), rh.max(1e-3))
            } else {
                (area_w, area_h)
            };
            view_autofit_scale(doc, views, root, aw, ah, 0.9)
        }
    };
    // Aligned children align to their parent along the shared edge (#364), not just their card.
    let bbox_center = view_render_center(doc, views, view_index);
    let area_center =
        glam::Vec2::new(cell_x + cell_w * 0.5, cell_y + caption_h + CELL_PAD + area_h * 0.5);
    (scale, bbox_center, area_center)
}

#[allow(clippy::too_many_arguments)]
fn render_view_geometry<C: Canvas>(
    canvas: &mut C,
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
    view_index: usize,
    scale_text: Option<&str>,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    unit: crate::value::LengthUnit,
) {
    // Crease edges drive circle detection (#319); the dimensionable set also carries silhouette
    // edges so a smooth extrusion's length can be dimensioned (#334). Dimensions draw against
    // the *logical* edges (#1780/#1781) — the same ones picking offers — so a dimension made
    // on a merged straight run finds its line here, and a curve's facets offer nothing.
    let crease_edges = drawing_view_world_edges(doc, view);
    let raw_edges = drawing_view_dimensionable_edges(doc, views, view);
    let (right, up) = resolved_view_axes(views, view);
    let world_edges =
        logical_pick_edges(&raw_edges, &|p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up)));
    if world_edges.is_empty() {
        return;
    }
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let proj: Vec<(glam::Vec2, glam::Vec2)> = world_edges
        .iter()
        .map(|(a, b)| (project(*a), project(*b)))
        .collect();

    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (a, b) in &proj {
        min = min.min(*a).min(*b);
        max = max.max(*a).max(*b);
    }
    let extent = (max - min).max(glam::Vec2::splat(1e-3));
    let _ = extent;
    let (scale, bbox_center, area_center) =
        export_view_transform(doc, views, view_index, scale_text, cell_x, cell_y, cell_w, cell_h);
    // Model +up maps to screen -y (y grows downward).
    let to_screen = |p: glam::Vec2| {
        let d = (p - bbox_center) * scale;
        glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
    };

    // Detect tessellated circles (#313) in world space and project them for this view: round
    // when face-on, a foreshortened line when edge-on (#319). Their segments are drawn as the
    // smooth circle/line and dimensioned once (the diameter), not per short segment.
    let world_circles = classify_world_circles(&crease_edges);
    let pcircles: Vec<ProjectedCircle> = world_circles
        .iter()
        .map(|c| project_world_circle(c, right, up))
        .collect();

    // Strokes (and shaded fills) come from the view's display style (#301); the fit above
    // always uses the full wireframe bbox so switching styles never re-scales the view.
    let styled = styled_view_geometry(doc, views, view);
    // A hidden-line style's rims come through `styled.segments` as the arcs that survive
    // (#1841/#1842); only a wireframe view still strokes a detected circle whole.
    let whole_circles = view_strokes_whole_circles(view);
    // Each fill, then the marks laid on it (#1840): a face's colored-pencil scribble or its
    // wash has to go down before whatever stands in front of that face covers it.
    for mark in styled.painted() {
        match mark {
            PaintedMark::Fill(face) => {
                // `shade` scales the face's tint (#1807): white for the grey Shaded style, the
                // body's own material color for Colorful, and for the hand-colored styles the
                // ground tone the geometry already mixed toward the paper (#1825/#1829).
                let lit =
                    |c: u8| (c as f32 * face.shade.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8;
                let fill = Rgb(lit(face.tint[0]), lit(face.tint[1]), lit(face.tint[2]));
                for pts in &face.tris {
                    let s: Vec<(f32, f32)> = pts
                        .iter()
                        .map(|p| {
                            let sp = to_screen(*p);
                            (sp.x, sp.y)
                        })
                        .collect();
                    canvas.poly(&s, fill);
                }
            }
            // A stroke is a darker patch of the fill it lies on (#1821), so it takes the same
            // tint × shade the fills do.
            PaintedMark::Stroke(stroke) => {
                let lit = |c: u8| {
                    (c as f32 * stroke.shade.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8
                };
                let (sa, sb) = (to_screen(stroke.a), to_screen(stroke.b));
                // A mark measured on the sheet scales with the view (#1840); the rest are in
                // the page's own units already.
                let width = if stroke.on_sheet { stroke.width * scale } else { stroke.width };
                canvas.line(
                    sa.x,
                    sa.y,
                    sb.x,
                    sb.y,
                    Rgb(lit(stroke.tint[0]), lit(stroke.tint[1]), lit(stroke.tint[2])),
                    width,
                );
            }
        }
    }
    let ink = styled
        .stroke_tint
        .map(|t| Rgb(t[0], t[1], t[2]))
        .unwrap_or(BLACK);
    for (a, b) in &styled.segments {
        // In a wireframe view a segment lying on a detected circle is left to the smooth
        // outline stroked below; in every other style that outline *is* these segments,
        // hidden where the solid hides them (#1841/#1842).
        if whole_circles && projected_segment_on_circle(*a, *b, &pcircles) {
            continue;
        }
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, ink, MODEL_STROKE);
    }
    // Smooth detected circles (round) or their foreshortened diameter line (edge-on) — a
    // whole outline, so only for the views that draw hidden lines anyway.
    for pc in pcircles.iter().filter(|_| whole_circles) {
        match pc {
            ProjectedCircle::Round { center, radius } => {
                let sc = to_screen(*center);
                canvas.circle(sc.x, sc.y, radius * scale, BLACK, MODEL_STROKE);
            }
            ProjectedCircle::EdgeOn { a, b } => {
                let (sa, sb) = (to_screen(*a), to_screen(*b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
            }
            ProjectedCircle::Angled { center, major, minor } => {
                // The true ellipse, as a closed polyline (#1775).
                let pts = angled_circle_points(*center, *major, *minor, 48);
                for i in 0..pts.len() {
                    let (sa, sb) = (to_screen(pts[i]), to_screen(pts[(i + 1) % pts.len()]));
                    canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
                }
            }
        }
    }
    // The section hatch strokes thinner than the edges (#1784): a fill texture, not
    // geometry.
    for (a, b) in &styled.hatch {
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, HATCH_STROKE);
    }
    // Zoom loupes (#1846): the detail circle, the magnified one redrawing what it covers, and
    // the thin line joining their rims.
    let loupe_lines = view_stroked_lines(&styled, &pcircles, view);
    // A loupe may draw its detail in a style of its own (#1850): the restyled geometry is
    // worked out once per distinct style, not once per loupe.
    let mut restyled: Vec<(crate::model::DrawingViewStyle, StyledViewGeometry, Vec<(glam::Vec2, glam::Vec2)>)> =
        Vec::new();
    for loupe in &view.loupes {
        let own = loupe.style.filter(|s| *s != view.style);
        let (lines, sty_for_loupe) = match own {
            Some(style) => {
                if !restyled.iter().any(|(s, _, _)| *s == style) {
                    let mut alt = view.clone();
                    alt.style = style;
                    let sty = styled_view_geometry(doc, views, &alt);
                    let lines = view_stroked_lines(&sty, &pcircles, &alt);
                    restyled.push((style, sty, lines));
                }
                let (_, sty, l) =
                    restyled.iter().find(|(s, _, _)| *s == style).expect("just inserted");
                (l, sty)
            }
            None => (&loupe_lines, &styled),
        };
        let d = loupe_drawing(loupe, lines, &sty_for_loupe.hatch, Some(sty_for_loupe));
        // The fills and shading marks go down before the edges, as on the card itself.
        for mark in &d.marks {
            match mark {
                LoupeMark::Fill(fill) => {
                    let level = |c: u8| (c as f32 * fill.shade.clamp(0.0, 1.0)) as u8;
                    let color =
                        Rgb(level(fill.tint[0]), level(fill.tint[1]), level(fill.tint[2]));
                    let pts: Vec<(f32, f32)> = fill
                        .points
                        .iter()
                        .map(|p| {
                            let s = to_screen(*p);
                            (s.x, s.y)
                        })
                        .collect();
                    canvas.poly(&pts, color);
                }
                LoupeMark::Stroke(stroke) => {
                    let sh = stroke.shade.clamp(0.0, 1.0);
                    let tint: [u8; 3] =
                        std::array::from_fn(|i| (stroke.tint[i] as f32 * sh) as u8);
                    let (sa, sb) = (to_screen(stroke.a), to_screen(stroke.b));
                    let width = if stroke.on_sheet { stroke.width * scale } else { stroke.width };
                    canvas.line(sa.x, sa.y, sb.x, sb.y, Rgb(tint[0], tint[1], tint[2]), width);
                }
            }
        }
        for (a, b) in &d.content {
            let (sa, sb) = (to_screen(*a), to_screen(*b));
            canvas.line(sa.x, sa.y, sb.x, sb.y, ink, MODEL_STROKE);
        }
        for (a, b) in &d.hatch {
            let (sa, sb) = (to_screen(*a), to_screen(*b));
            canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, HATCH_STROKE);
        }
        // Dimensions the loupe carries (#1849/#1913): drawn against the magnified copy of
        // the edge, labelled with its real length. A cropped end dashes instead of closing
        // with an arrow.
        {
            let (c1, c2) = (d.detail.0, d.magnified.0);
            let zoom = loupe_zoom(loupe);
            let map = |p: glam::Vec2| c2 + (p - c1) * zoom;
            for (i, (a, b)) in proj.iter().enumerate() {
                let (wa, wb) = world_edges[i];
                if !loupe.dimensioned_edges.contains(&edge_key(wa, wb)) {
                    continue;
                }
                let Some((ca, cb)) = clip_segment_to_circle(*a, *b, c1, loupe.radius.abs())
                else {
                    continue;
                };
                let (ma, mb) = (map(ca), map(cb));
                if (mb - ma).length() < 1e-3 {
                    continue;
                }
                let outward = dimension_outward(ma, mb, c2);
                // Same sizes the card's own dimensions use (they are set further down, where
                // the view's dimensions are drawn; a loupe's are the same page distances).
                let diag = extent.length().max(1.0);
                let closed = loupe_dim_closed_ends(*a, *b, c1, loupe.radius.abs());
                let geom = loupe_dimension_geometry(
                    ma, mb, outward, diag * 0.05, diag * 0.025, closed,
                );
                let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
                    let (sp, sq) = (to_screen(p), to_screen(q));
                    canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
                };
                let stroke_dashed = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
                    let (sp, sq) = (to_screen(p), to_screen(q));
                    canvas.line_dashed(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
                };
                for (p, q) in geom.extensions {
                    stroke_line(canvas, p, q);
                }
                if let Some((p, q)) = geom.solid {
                    stroke_line(canvas, p, q);
                }
                for (p, q) in geom.dashes {
                    stroke_dashed(canvas, p, q);
                }
                for tri in geom.arrows {
                    let pts: Vec<(f32, f32)> = tri
                        .iter()
                        .map(|p| {
                            let s = to_screen(*p);
                            (s.x, s.y)
                        })
                        .collect();
                    canvas.poly(&pts, BLACK);
                }
                let label = crate::value::format_length_display_in((wa - wb).length(), unit);
                let (sa, sb) = (to_screen(geom.line.0), to_screen(geom.line.1));
                let out_screen =
                    (to_screen(geom.line.0 + outward) - to_screen(geom.line.0)).normalize();
                let (lp, ang) = dimension_label_layout(
                    sa,
                    sb,
                    out_screen,
                    text_device_width(11.0, &label),
                    11.0,
                    5.0,
                );
                canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
            }
        }
        for (c, r) in [d.detail, d.magnified] {
            let sc = to_screen(c);
            canvas.circle(sc.x, sc.y, r * scale, BLACK, LOUPE_STROKE);
        }
        if let Some((a, b)) = d.connector {
            let (sa, sb) = (to_screen(a), to_screen(b));
            canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, LOUPE_STROKE);
        }
    }
    // Length dimensions (#294): architectural dimension lines — extension lines, an offset
    // dimension line with arrowheads, and the measured length centred on it. Sizes are a
    // fraction of the projected extent so they read at any scale; a per-edge override
    // (dimension_offsets) pushes the line further out.
    let diag = extent.length().max(1.0);
    let default_gap = diag * 0.05;
    let arrow = diag * 0.025;
    // A single diameter dimension per detected circle (#313), replacing its segments' dims — but
    // only for circles whose diameter is shown (#342), so Show/Hide all controls them too.
    for (wc, pc) in world_circles.iter().zip(&pcircles) {
        if !view
            .dimensioned_circles
            .contains(&crate::hierarchy::quantize_body_point(wc.center))
        {
            continue;
        }
        let label = format!("Ø{}", crate::value::format_length_display_in(wc.radius * 2.0, unit));
        let circle_key = crate::hierarchy::quantize_body_point(wc.center);
        let extra = view
            .circle_dim_offsets
            .iter()
            .find(|(k, _)| *k == circle_key)
            .map(|(_, o)| *o)
            .unwrap_or(0.0);
        match pc {
            // Face-on (#397): a horizontal diameter line, the label offset off it by the
            // per-circle override (dragged up/down in the editor).
            ProjectedCircle::Round { center, radius } => {
                let dir = glam::Vec2::new(1.0, 0.0);
                let (a, b) = (*center - dir * *radius, *center + dir * *radius);
                let (sa, sb) = (to_screen(a), to_screen(b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, DIM_STROKE);
                let lp = to_screen(*center + glam::Vec2::new(0.0, extra));
                canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, 0.0);
            }
            // Edge-on (looks like a line, #320): a normal linear dimension — extension lines,
            // an offset dimension line with arrowheads, and the value running along it.
            ProjectedCircle::EdgeOn { a, b } => {
                let outward = dimension_outward(*a, *b, bbox_center);
                let geom = dimension_line_geometry(*a, *b, outward, default_gap + extra, arrow);
                let sl = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
                    let (sp, sq) = (to_screen(p), to_screen(q));
                    canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
                };
                for (p, q) in geom.extensions {
                    sl(canvas, p, q);
                }
                sl(canvas, geom.line.0, geom.line.1);
                for tri in geom.arrows {
                    let pts: Vec<(f32, f32)> =
                        tri.iter().map(|p| { let s = to_screen(*p); (s.x, s.y) }).collect();
                    canvas.poly(&pts, BLACK);
                }
                let (sla, slb) = (to_screen(geom.line.0), to_screen(geom.line.1));
                let out_screen =
                    (to_screen(geom.line.0 + outward) - to_screen(geom.line.0)).normalize_or_zero();
                let (lp, ang) = dimension_label_layout(sla, slb, out_screen, text_device_width(11.0, &label), 11.0, 5.0);
                canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
            }
            // Angled (#1775): the diameter line runs along the ellipse's major axis — the one
            // direction that still spans the true diameter — with the label offset along the
            // minor axis by the per-circle override.
            ProjectedCircle::Angled { center, major, minor } => {
                let (a, b) = (*center - *major, *center + *major);
                let (sa, sb) = (to_screen(a), to_screen(b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, DIM_STROKE);
                let lp = to_screen(*center + minor.normalize_or_zero() * extra);
                canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, 0.0);
            }
        }
    }
    for (i, (a, b)) in proj.iter().enumerate() {
        let (wa, wb) = world_edges[i];
        let key = edge_key(wa, wb);
        // An edge-on edge projects to a point — nothing meaningful to dimension here (#294) —
        // and circle segments are covered by the single diameter dimension above (#313).
        if !view.dimensioned_edges.contains(&key)
            || (*b - *a).length() < 1e-3
            || projected_segment_on_circle(*a, *b, &pcircles)
        {
            continue;
        }
        let outward = dimension_outward(*a, *b, bbox_center);
        let extra = view
            .dimension_offsets
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, o)| *o)
            .unwrap_or(0.0);
        let geom = dimension_line_geometry(*a, *b, outward, default_gap + extra, arrow);
        let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
            let (sp, sq) = (to_screen(p), to_screen(q));
            canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
        };
        for (p, q) in geom.extensions {
            stroke_line(canvas, p, q);
        }
        stroke_line(canvas, geom.line.0, geom.line.1);
        for tri in geom.arrows {
            let pts: Vec<(f32, f32)> = tri
                .iter()
                .map(|p| {
                    let s = to_screen(*p);
                    (s.x, s.y)
                })
                .collect();
            canvas.poly(&pts, BLACK);
        }
        // The label runs along the dimension line, or sits past its end if too short (#314).
        let label = crate::value::format_length_display_in((wa - wb).length(), unit);
        let (sa, sb) = (to_screen(geom.line.0), to_screen(geom.line.1));
        let out_screen = (to_screen(geom.line.0 + outward) - to_screen(geom.line.0)).normalize();
        let (lp, ang) = dimension_label_layout(
            sa,
            sb,
            out_screen,
            text_device_width(11.0, &label),
            11.0,
            5.0,
        );
        canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
    }

    // Curve length dimensions (#1785): the whole polyline strokes with the edges and its
    // measured length labels the middle, pushed outward from the view's centre.
    let curves = logical_pick_curves(&raw_edges, &project);
    for key in &view.dimensioned_curves {
        let matches = |chain: &Vec<Vec3>| {
            canonical_curve_key(
                &chain.iter().map(|p| crate::hierarchy::quantize_body_point(*p)).collect::<Vec<_>>(),
            ) == *key
        };
        let Some(chain) = curves.iter().find(|c| matches(c)) else {
            continue;
        };
        let pts: Vec<glam::Vec2> = chain.iter().map(|p| project(*p)).collect();
        for w in pts.windows(2) {
            if projected_segment_on_circle(w[0], w[1], &pcircles) {
                continue;
            }
            let (sa, sb) = (to_screen(w[0]), to_screen(w[1]));
            canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
        }
        let mid = pts[pts.len() / 2];
        let outward = {
            let v = mid - bbox_center;
            let n = v.normalize_or_zero();
            if n.length_squared() < 0.5 { glam::vec2(0.0, -1.0) } else { n }
        };
        let label = crate::value::format_length_display_in(curve_chain_length(chain), unit);
        let lp = to_screen(mid + outward * default_gap);
        canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, 0.0);
    }

    // Free point-to-point dimensions (#1645): two picked points, measured straight between
    // them or along one page axis.
    for dim in &view.point_dims {
        let (pa, pb, out) = point_dim_line(dim, default_gap);
        if (pb - pa).length() < 1e-3 {
            continue;
        }
        let a = glam::Vec2::new(dim.a.0, dim.a.1);
        let b = glam::Vec2::new(dim.b.0, dim.b.1);
        let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
            let (sp, sq) = (to_screen(p), to_screen(q));
            canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
        };
        // Extension lines run from each picked point out past the dimension line.
        for (p, q) in [(a, pa), (b, pb)] {
            stroke_line(canvas, p, q + (q - p).normalize_or_zero() * (arrow * 0.5));
        }
        stroke_line(canvas, pa, pb);
        let geom = dimension_line_geometry(pa, pb, out, 0.0, arrow);
        for tri in geom.arrows {
            let pts: Vec<(f32, f32)> =
                tri.iter().map(|p| { let s = to_screen(*p); (s.x, s.y) }).collect();
            canvas.poly(&pts, BLACK);
        }
        let label = crate::value::format_length_display_in(point_dim_value(dim), unit);
        let (sa, sb) = (to_screen(pa), to_screen(pb));
        let out_screen = (to_screen(pa + out) - to_screen(pa)).normalize_or_zero();
        let (lp, ang) =
            dimension_label_layout(sa, sb, out_screen, text_device_width(11.0, &label), 11.0, 5.0);
        canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
    }

    // Angle dimensions (#1652): an arc at the two edges' corner, spanning between them, with
    // arrowheads and the degree value just outside it.
    for (k1, k2) in &view.angle_dims {
        let edge = |k: &([i32; 3], [i32; 3])| (project(dequant(k.0)), project(dequant(k.1)));
        let Some(g) = angle_dim_geometry(edge(k1), edge(k2), arrow) else {
            continue;
        };
        let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
            let (sp, sq) = (to_screen(p), to_screen(q));
            canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
        };
        for (p, q) in &g.extensions {
            stroke_line(canvas, *p, *q);
        }
        let arc = g.arc_points();
        for pair in arc.windows(2) {
            stroke_line(canvas, pair[0], pair[1]);
        }
        for tri in g.arrows {
            let pts: Vec<(f32, f32)> =
                tri.iter().map(|p| { let s = to_screen(*p); (s.x, s.y) }).collect();
            canvas.poly(&pts, BLACK);
        }
        let sp = to_screen(g.label);
        canvas.text(sp.x, sp.y, 11.0, Anchor::Middle, &format!("{:.0}°", g.degrees));
    }
}

// ----- SVG backend -----

struct SvgCanvas {
    body: String,
    /// Whether the text that follows is a pencil view's, and so set in the hand-lettered face
    /// (#1830). SVG names the family; the viewer needs it installed, and falls back to the
    /// sans otherwise — embedding the whole face would add megabytes to every export.
    hand_lettered: bool,
}

impl SvgCanvas {
    fn font_family(&self) -> String {
        if self.hand_lettered {
            format!("'{}', sans-serif", crate::pencil::LABEL_FONT_FAMILY)
        } else {
            "sans-serif".to_string()
        }
    }
}

fn svg_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn svg_color(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

impl Canvas for SvgCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        let fill = fill.map(svg_color).unwrap_or_else(|| "none".to_string());
        let stroke_attr = match stroke {
            Some(c) => format!(" stroke=\"{}\" stroke-width=\"{stroke_w}\"", svg_color(c)),
            None => String::new(),
        };
        self.body.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{fill}\"{stroke_attr}/>\n"
        ));
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{}\" \
             stroke-width=\"{width}\"/>\n",
            svg_color(color)
        ));
    }

    fn line_dashed(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{}\" \
             stroke-width=\"{width}\" stroke-dasharray=\"4 3\"/>\n",
            svg_color(color)
        ));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        // Stroked with its own fill so adjacent shaded triangles don't show hairline seams.
        let points: Vec<String> = pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        self.body.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\"0.6\"/>\n",
            points.join(" "),
            fill = svg_color(fill)
        ));
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" stroke=\"{}\" \
             stroke-width=\"{width}\"/>\n",
            svg_color(color)
        ));
    }

    fn set_hand_lettered(&mut self, hand_lettered: bool) {
        self.hand_lettered = hand_lettered;
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        };
        let family = self.font_family();
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" font-size=\"{size}\" \
             fill=\"black\" text-anchor=\"{anchor}\">{}</text>\n",
            svg_esc(content)
        ));
    }

    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        };
        let deg = angle.to_degrees();
        let family = self.font_family();
        // `dominant-baseline="central"` makes `(x, y)` the visual centre, matching the
        // editor and the PDF backend (#1350). Captions still use `text()`, which stays
        // baseline-aligned.
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" font-size=\"{size}\" \
             fill=\"black\" text-anchor=\"{anchor}\" dominant-baseline=\"central\" \
             transform=\"rotate({deg:.2} {x:.1} {y:.1})\">{}</text>\n",
            svg_esc(content)
        ));
    }
}

/// Render one drawing to a self-contained black-on-white SVG document. `None` if the drawing
/// index is missing or deleted.
pub fn drawing_to_svg(doc: &Document, index: crate::model::DrawingKey) -> Option<String> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = SvgCanvas { body: String::new(), hand_lettered: false };
    render_drawing(doc, index, &mut canvas)?;
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">\n"
    ));
    s.push_str(&canvas.body);
    s.push_str("</svg>\n");
    Some(s)
}

// ----- PDF backend -----

/// Accumulates a PDF content stream. PDF space is bottom-left origin, so `y` is flipped from
/// the top-left [`Canvas`] coordinates using the page `height`.
struct PdfCanvas {
    ops: Vec<u8>,
    height: f32,
}

impl PdfCanvas {
    fn new(height: f32) -> Self {
        PdfCanvas { ops: Vec::new(), height }
    }
    fn push(&mut self, s: &str) {
        self.ops.extend_from_slice(s.as_bytes());
    }
    fn set_fill(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} rg\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
    fn set_stroke(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} RG\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
}

/// Escape a string into PDF WinAnsi document bytes (the font is Helvetica/WinAnsiEncoding).
fn pdf_text_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                out.push(ch as u8);
            }
            '⌀' => out.push(0xD8), // diameter → WinAnsi Ø (Latin O with stroke)
            'μ' => out.push(0xB5), // Greek mu → WinAnsi micro sign
            c if (c as u32) < 128 => out.push(c as u8),
            // WinAnsi is Latin-1 from U+00A0 up, so an accented letter, a degree sign or a
            // micro sign is simply its own byte — writing '?' there mangled every
            // non-English label (#1661).
            c if ('\u{a0}'..='\u{ff}').contains(&c) => out.push(c as u8),
            // The 0x80–0x9F slots WinAnsi fills with punctuation Latin-1 does not have.
            c => match winansi_special(c) {
                Some(b) => out.push(b),
                None => out.push(b'?'),
            },
        }
    }
    out
}

/// The WinAnsiEncoding characters in the 0x80–0x9F range, which Latin-1 leaves as control
/// codes — smart quotes, dashes, the ellipsis and friends (#1661).
fn winansi_special(c: char) -> Option<u8> {
    Some(match c {
        '\u{20ac}' => 0x80, // €
        '\u{201a}' => 0x82, // ‚
        '\u{0192}' => 0x83, // ƒ
        '\u{201e}' => 0x84, // „
        '\u{2026}' => 0x85, // …
        '\u{2020}' => 0x86, // †
        '\u{2021}' => 0x87, // ‡
        '\u{02c6}' => 0x88, // ˆ
        '\u{2030}' => 0x89, // ‰
        '\u{0160}' => 0x8a, // Š
        '\u{2039}' => 0x8b, // ‹
        '\u{0152}' => 0x8c, // Œ
        '\u{017d}' => 0x8e, // Ž
        '\u{2018}' => 0x91, // '
        '\u{2019}' => 0x92, // '
        '\u{201c}' => 0x93, // "
        '\u{201d}' => 0x94, // "
        '\u{2022}' => 0x95, // •
        '\u{2013}' => 0x96, // –
        '\u{2014}' => 0x97, // —
        '\u{02dc}' => 0x98, // ˜
        '\u{2122}' => 0x99, // ™
        '\u{0161}' => 0x9a, // š
        '\u{203a}' => 0x9b, // ›
        '\u{0153}' => 0x9c, // œ
        '\u{017e}' => 0x9e, // ž
        '\u{0178}' => 0x9f, // Ÿ
        _ => return None,
    })
}

impl Canvas for PdfCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        // Top-left (x, y) with height h → PDF bottom-left corner is (x, H - y - h).
        let py = self.height - y - h;
        self.push(&format!("{x:.2} {py:.2} {w:.2} {h:.2} re\n"));
        match (fill, stroke) {
            (Some(f), Some(s)) => {
                self.set_fill(f);
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nB\n"));
            }
            (Some(f), None) => {
                self.set_fill(f);
                self.push("f\n");
            }
            (None, Some(s)) => {
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nS\n"));
            }
            (None, None) => self.push("n\n"),
        }
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        let (py1, py2) = (self.height - y1, self.height - y2);
        self.set_stroke(color);
        self.push(&format!("{width:.2} w\n{x1:.2} {py1:.2} m {x2:.2} {py2:.2} l S\n"));
    }

    fn line_dashed(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        let (py1, py2) = (self.height - y1, self.height - y2);
        self.set_stroke(color);
        // `[4 3] 0 d` sets the dash pattern; `[] 0 d` restores solid strokes after.
        self.push(&format!(
            "[4 3] 0 d\n{width:.2} w\n{x1:.2} {py1:.2} m {x2:.2} {py2:.2} l S\n[] 0 d\n"
        ));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        let Some(((x0, y0), rest)) = pts.split_first() else {
            return;
        };
        // Fill *and* stroke with the same grey so adjacent shaded triangles don't show
        // hairline seams between them.
        self.set_fill(fill);
        self.set_stroke(fill);
        let mut path = format!("0.6 w\n{x0:.2} {:.2} m ", self.height - y0);
        for (x, y) in rest {
            path.push_str(&format!("{x:.2} {:.2} l ", self.height - y));
        }
        path.push_str("h b\n");
        self.push(&path);
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        // Four cubic Bézier arcs (kappa ≈ 0.5523) approximate a circle; y flips to PDF space.
        let k = 0.552_284_75 * r;
        let cy = self.height - cy;
        self.set_stroke(color);
        let mut path = format!("{width:.2} w\n{:.2} {cy:.2} m ", cx + r);
        // Right → top → left → bottom, counter-clockwise in PDF's y-up space.
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + r, cy + k, cx + k, cy + r, cx, cy + r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - k, cy + r, cx - r, cy + k, cx - r, cy));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - r, cy - k, cx - k, cy - r, cx, cy - r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + k, cy - r, cx + r, cy - k, cx + r, cy));
        path.push_str("S\n");
        self.push(&path);
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        // Helvetica averages ~0.5em per glyph; good enough to center dimension labels.
        let width = 0.5 * size * content.chars().count() as f32;
        let tx = match anchor {
            Anchor::Start => x,
            Anchor::Middle => x - width * 0.5,
            Anchor::End => x - width,
        };
        let py = self.height - y;
        self.set_fill(BLACK);
        self.push(&format!("BT /F1 {size:.2} Tf {tx:.2} {py:.2} Td ("));
        let bytes = pdf_text_bytes(content);
        self.ops.extend_from_slice(&bytes);
        self.push(") Tj ET\n");
    }

    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        // Rotate about the visual centre `(x, y)` via the text matrix. Screen angle is
        // clockwise (y-down); PDF is y-up, so negate. Shift half the text width along the
        // rotated baseline and 0.35em down from the cap-centre so the glyphs sit on the
        // same point the editor centres its galley on (#314/#1350).
        let width = 0.5 * size * content.chars().count() as f32;
        let half = match anchor {
            Anchor::Middle => width * 0.5,
            Anchor::Start => 0.0,
            Anchor::End => width,
        };
        let a = -angle;
        let (c, s) = (a.cos(), a.sin());
        let py = self.height - y;
        let v = DIM_LABEL_MID_EM * size;
        let tx = x - half * c + s * v;
        let ty = py - half * s - c * v;
        self.set_fill(BLACK);
        self.push(&format!(
            "BT /F1 {size:.2} Tf {c:.4} {s:.4} {:.4} {c:.4} {tx:.2} {ty:.2} Tm (",
            -s
        ));
        let bytes = pdf_text_bytes(content);
        self.ops.extend_from_slice(&bytes);
        self.push(") Tj ET\n");
    }
}

/// Render one drawing to a self-contained single-page PDF (black-on-white, Helvetica text).
/// `None` if the drawing index is missing or deleted.
pub fn drawing_to_pdf(doc: &Document, index: crate::model::DrawingKey) -> Option<Vec<u8>> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = PdfCanvas::new(height);
    render_drawing(doc, index, &mut canvas)?;
    Some(assemble_pdf(width, height, &canvas.ops))
}

/// Wrap a content stream in a minimal single-page PDF document (catalog, pages, one page with
/// a Helvetica font, the content stream), with a correct cross-reference table.
fn assemble_pdf(width: f32, height: f32, content: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, &mut offsets, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.2} {height:.2}] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // Content stream object (4).
    {
        offsets.push(out.len());
        out.extend_from_slice(b"4 0 obj\n");
        out.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        out.extend_from_slice(content);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }
    obj(
        &mut out,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );

    // Cross-reference table + trailer.
    let xref_pos = out.len();
    let count = offsets.len() + 1;
    out.extend_from_slice(format!("xref\n0 {count}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

#[cfg(test)]
mod loupe_tests {
    use super::*;
    use crate::model::DrawingLoupe;

    fn loupe() -> DrawingLoupe {
        DrawingLoupe {
            at: (10.0, 5.0),
            radius: 4.0,
            to: (60.0, 5.0),
            to_radius: 12.0,
            style: None,
            dimensioned_edges: Vec::new(),
        }
    }

    /// #1850: what a loupe paints under its edges is clipped to the detail circle and lands
    /// inside the magnified one — a shaded loupe on a wireframe view is the whole point of
    /// letting a loupe pick its own style.
    #[test]
    fn a_loupe_clips_and_magnifies_the_styled_fills() {
        let l = loupe();
        let (c1, c2) = loupe_centers(&l);
        // One big triangle across the detail circle, and one well clear of it.
        let styled = StyledViewGeometry {
            faces: vec![
                ShadedFace {
                    tris: vec![[
                        c1 + glam::Vec2::new(-40.0, -40.0),
                        c1 + glam::Vec2::new(40.0, -40.0),
                        c1 + glam::Vec2::new(0.0, 40.0),
                    ]],
                    shade: 0.6,
                    tint: [255, 255, 255],
                    plane: (Vec3::Z, 0.0),
                },
                ShadedFace {
                    tris: vec![[
                        c1 + glam::Vec2::new(500.0, 500.0),
                        c1 + glam::Vec2::new(520.0, 500.0),
                        c1 + glam::Vec2::new(510.0, 520.0),
                    ]],
                    shade: 0.3,
                    tint: [255, 255, 255],
                    plane: (Vec3::Z, 0.0),
                },
            ],
            segments: Vec::new(),
            hatch: Vec::new(),
            shading: Vec::new(),
            scribbled: false,
            stroke_tint: None,
        };
        let d = loupe_drawing(&l, &[], &[], Some(&styled));
        let fills: Vec<&LoupeFill> = d
            .marks
            .iter()
            .filter_map(|m| match m {
                LoupeMark::Fill(f) => Some(f),
                LoupeMark::Stroke(_) => None,
            })
            .collect();
        assert_eq!(fills.len(), 1, "the far triangle contributes nothing");
        let fill = fills[0];
        assert!((fill.shade - 0.6).abs() < 1e-6, "the fill keeps its own tone");
        let r = l.to_radius.abs();
        for p in &fill.points {
            let inside = (*p - c2).length();
            assert!(
                inside <= r + 1e-3,
                "every corner lands inside the magnified circle: {inside} > {r}"
            );
        }
        // It fills the circle rather than shrinking to a speck: a triangle spanning the
        // detail circle covers most of the magnified one.
        let far = fill
            .points
            .iter()
            .map(|p| (*p - c2).length())
            .fold(0.0_f32, f32::max);
        assert!(far > r * 0.8, "the clipped fill reaches the rim, got {far} of {r}");
    }

    /// #1850: a polygon clear of the circle clips away entirely, and one inside is untouched.
    #[test]
    fn clipping_to_a_circle_keeps_what_is_inside() {
        let c = glam::Vec2::new(5.0, 5.0);
        let inside = [
            glam::Vec2::new(4.0, 4.0),
            glam::Vec2::new(6.0, 4.0),
            glam::Vec2::new(5.0, 6.0),
        ];
        let kept = clip_convex_to_circle(&inside, c, 10.0);
        assert_eq!(kept.len(), 3, "a polygon well inside comes through whole");
        let outside = [
            glam::Vec2::new(90.0, 90.0),
            glam::Vec2::new(92.0, 90.0),
            glam::Vec2::new(91.0, 92.0),
        ];
        assert!(
            clip_convex_to_circle(&outside, c, 10.0).is_empty(),
            "one clear of the circle clips away entirely"
        );
    }

    /// #1846: the zoom is the ratio of the two circles, so growing the magnified circle
    /// magnifies rather than showing more.
    #[test]
    fn loupe_zoom_is_the_ratio_of_the_circles() {
        assert!((loupe_zoom(&loupe()) - 3.0).abs() < 1e-6);
    }

    /// #1846: the two circles are joined edge to edge, not centre to centre — the connector
    /// starts on the detail circle's rim and ends on the magnified one's.
    #[test]
    fn loupe_connector_runs_rim_to_rim() {
        let l = loupe();
        let (a, b) = loupe_connector(&l).expect("the circles are apart");
        let (c1, c2) = (glam::Vec2::new(l.at.0, l.at.1), glam::Vec2::new(l.to.0, l.to.1));
        assert!(((a - c1).length() - l.radius).abs() < 1e-4, "starts on the detail rim: {a:?}");
        assert!(((b - c2).length() - l.to_radius).abs() < 1e-4, "ends on the magnified rim: {b:?}");
        // And it runs along the line between the centres.
        let along = (c2 - c1).normalize();
        assert!((b - a).normalize().distance(along) < 1e-4, "runs centre to centre");
        // Overlapping circles have no gap to bridge.
        let touching = DrawingLoupe { to: (18.0, 5.0), ..l };
        assert!(loupe_connector(&touching).is_none(), "no connector when they overlap");
    }

    /// #1846: the magnified circle redraws what the detail circle covers, scaled by the zoom
    /// and re-centred — and nothing outside the detail circle comes along.
    #[test]
    fn loupe_magnifies_only_what_the_detail_circle_covers() {
        let l = loupe();
        let inside = (glam::Vec2::new(8.0, 5.0), glam::Vec2::new(12.0, 5.0));
        let outside = (glam::Vec2::new(40.0, 40.0), glam::Vec2::new(44.0, 44.0));
        // A segment that starts inside and leaves is clipped at the rim.
        let crossing = (glam::Vec2::new(10.0, 5.0), glam::Vec2::new(30.0, 5.0));
        let out = loupe_magnified_segments(&l, &[inside, outside, crossing]);
        assert_eq!(out.len(), 2, "the segment clear of the circle is dropped: {out:?}");

        let (a, b) = out[0];
        assert!(a.distance(glam::Vec2::new(54.0, 5.0)) < 1e-4, "left end 3x out from the centre: {a:?}");
        assert!(b.distance(glam::Vec2::new(66.0, 5.0)) < 1e-4, "right end: {b:?}");

        let c2 = glam::Vec2::new(l.to.0, l.to.1);
        for (a, b) in &out {
            assert!((*a - c2).length() <= l.to_radius + 1e-3, "stays inside the magnified circle");
            assert!((*b - c2).length() <= l.to_radius + 1e-3, "stays inside the magnified circle");
        }
        // The crossing segment is cut at the rim: it reaches the magnified circle's edge.
        let (_, end) = out[1];
        assert!(
            ((end - c2).length() - l.to_radius).abs() < 1e-3,
            "a segment leaving the detail circle stops on the magnified rim, got {end:?}"
        );
    }

    /// #1913: an edge wholly inside the detail circle still gets a complete dimension —
    /// arrows and extension lines at both ends, no dashes.
    #[test]
    fn loupe_dimension_stays_complete_when_the_edge_fits() {
        let c = glam::Vec2::new(10.0, 5.0);
        let a = glam::Vec2::new(8.0, 5.0);
        let b = glam::Vec2::new(12.0, 5.0);
        let closed = loupe_dim_closed_ends(a, b, c, 4.0);
        assert_eq!(closed, [true, true]);
        let g = loupe_dimension_geometry(a, b, glam::Vec2::new(0.0, 1.0), 2.0, 1.0, closed);
        assert_eq!(g.arrows.len(), 2, "both arrows");
        assert_eq!(g.extensions.len(), 2, "both witness lines");
        assert!(g.dashes.is_empty(), "no dashes on a complete measurement");
        assert_eq!(g.solid, Some(g.line));
    }

    /// #1913: an edge that crosses the circle is not a complete measurement of the cropped
    /// bit. ISO 129 keeps arrows only at shown feature ends; the open end finishes in dashes
    /// instead of an arrow.
    #[test]
    fn loupe_dimension_dashes_the_end_that_leaves_the_circle() {
        let c = glam::Vec2::new(10.0, 5.0);
        let r = 4.0;
        let a = glam::Vec2::new(10.0, 5.0); // inside
        let b = glam::Vec2::new(30.0, 5.0); // well outside
        let closed = loupe_dim_closed_ends(a, b, c, r);
        assert_eq!(closed, [true, false]);
        let (ca, cb) = clip_segment_to_circle(a, b, c, r).expect("it crosses");
        let g = loupe_dimension_geometry(ca, cb, glam::Vec2::new(0.0, 1.0), 2.0, 1.0, closed);
        assert_eq!(g.arrows.len(), 1, "arrow only at the end that's in the loupe");
        assert_eq!(g.extensions.len(), 1, "no witness line at the crop");
        assert_eq!(g.dashes.len(), 1, "the open end finishes in dashes");
        // The dash occupies the open end of the dimension line, not the closed one.
        let (da, db) = g.line;
        let (s, e) = g.dashes[0];
        assert!((e - db).length() < 1e-3, "dashes meet the open end: {e:?} vs {db:?}");
        assert!((s - da).length() > 1e-3, "dashes do not eat the closed end");
    }

    /// #1913: a long edge through the middle of the loupe continues both ways — dashes at
    /// both ends, no arrows, so the label is not read as the length of the visible bit.
    #[test]
    fn loupe_dimension_dashes_both_ends_when_the_edge_crosses_through() {
        let c = glam::Vec2::new(10.0, 5.0);
        let a = glam::Vec2::new(-20.0, 5.0);
        let b = glam::Vec2::new(40.0, 5.0);
        let closed = loupe_dim_closed_ends(a, b, c, 4.0);
        assert_eq!(closed, [false, false]);
        let (ca, cb) = clip_segment_to_circle(a, b, c, 4.0).expect("it crosses");
        let g = loupe_dimension_geometry(ca, cb, glam::Vec2::new(0.0, 1.0), 2.0, 1.0, closed);
        assert!(g.arrows.is_empty(), "no arrows: neither end is in the loupe");
        assert!(g.extensions.is_empty(), "no witness lines at the crop");
        assert_eq!(g.dashes.len(), 2, "both ends finish in dashes");
        assert!(g.solid.is_some(), "a solid middle remains for the label");
    }
}

#[cfg(test)]
mod tests {
    use crate::model::drawing_key_for_slot as dkey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use crate::model::{Drawing, DrawingView};

    /// #1780/#1781: a drawing view's pick edges are *logical* edges. Straight tessellation
    /// chains merge into the one edge they represent — so a cursor picks "the vertical line",
    /// not its pieces — and chains that turn (a curve's facets) drop out entirely: the
    /// divisions are a rendering artifact, and circles are picked whole by their own
    /// detection. Corners — where the direction genuinely changes — keep their endpoints.
    #[test]
    fn pick_edges_merge_straight_runs_and_drop_curve_facets() {
        let flat = |p: Vec3| glam::vec2(p.x, p.y);
        let run = |from: Vec3, to: Vec3, pieces: usize| -> Vec<(Vec3, Vec3)> {
            // `pieces` collinear segments from `from` to `to`, sharing endpoints.
            (0..pieces)
                .map(|i| {
                    let t0 = i as f32 / pieces as f32;
                    let t1 = (i + 1) as f32 / pieces as f32;
                    (
                        from + (to - from) * t0,
                        from + (to - from) * t1,
                    )
                })
                .collect()
        };
        let arc = |center: Vec3, radius: f32| -> Vec<(Vec3, Vec3)> {
            // A quarter-circle's facets: every joint turns.
            (0..24)
                .map(|i| {
                    let a0 = i as f32 / 24.0 * std::f32::consts::FRAC_PI_2;
                    let a1 = (i + 1) as f32 / 24.0 * std::f32::consts::FRAC_PI_2;
                    (
                        center + Vec3::new(radius * a0.cos(), radius * a0.sin(), 0.0),
                        center + Vec3::new(radius * a1.cos(), radius * a1.sin(), 0.0),
                    )
                })
                .collect()
        };
        // A straight run split into 5 pieces, an L corner of two whole edges, and a curved
        // arc — the Front view of a cut cylinder has exactly these.
        let mut edges = run(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 30.0), 5);
        edges.push((Vec3::new(0.0, 0.0, 30.0), Vec3::new(10.0, 0.0, 30.0)));
        edges.extend(arc(Vec3::new(10.0, 0.0, 30.0), 4.0));
        let logical = logical_pick_edges(&edges, &flat);
        // The straight run merges into one edge spanning its true ends, and the L's two
        // edges stay whole. The arc's facets are gone.
        assert_eq!(
            logical.len(),
            2,
            "one merged run + one corner edge, no arc facets: {logical:?}"
        );
        assert!(
            logical.iter().any(|(a, b)| {
                (a - Vec3::new(0.0, 0.0, 5.0)).length() < 1e-3
                    && (b - Vec3::new(0.0, 0.0, 30.0)).length() < 1e-3
            }),
            "the split run merges to its true endpoints: {logical:?}"
        );
        assert!(
            logical.iter().any(|(a, b)| {
                (a - Vec3::new(0.0, 0.0, 30.0)).length() < 1e-3
                    && (b - Vec3::new(10.0, 0.0, 30.0)).length() < 1e-3
            }),
            "the corner edge survives whole: {logical:?}"
        );
    }

    /// #1780: pick edges are order-independent — a run's pieces can arrive in any order and
    /// either direction, as mesh extraction produces them.
    #[test]
    fn pick_edges_merge_regardless_of_segment_order() {
        let flat = |p: Vec3| glam::vec2(p.x, p.y);
        let ends = [
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(3.0, 1.0, 0.0),
            Vec3::new(5.0, 1.0, 0.0),
        ];
        let mut edges = vec![
            (ends[1], ends[2]),
            (ends[0], ends[1]),
        ];
        let logical = logical_pick_edges(&edges, &flat);
        assert_eq!(logical.len(), 1, "one merged edge: {logical:?}");
        let (a, b) = logical[0];
        assert!((a - ends[0]).length() < 1e-3 && (b - ends[2]).length() < 1e-3);

        // A single piece is its own logical edge, unchanged.
        edges = vec![(ends[0], ends[1])];
        assert_eq!(logical_pick_edges(&edges, &flat).len(), 1);
    }

    /// #1780: a closed chain of turning facets — a full circle (or any closed curve) — drops
    /// out rather than coming back as pieces.
    #[test]
    fn pick_edges_drop_a_closed_chain_of_turning_facets() {
        let flat = |p: Vec3| glam::vec2(p.x, p.y);
        let edges: Vec<(Vec3, Vec3)> = (0..32)
            .map(|i| {
                let a0 = i as f32 / 32.0 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / 32.0 * std::f32::consts::TAU;
                (
                    Vec3::new(5.0 + a0.cos(), 5.0 + a0.sin(), 0.0),
                    Vec3::new(5.0 + a1.cos(), 5.0 + a1.sin(), 0.0),
                )
            })
            .collect();
        assert!(logical_pick_edges(&edges, &flat).is_empty());
    }

    /// #1780: a curve seen edge-on projects dead straight (a circle rim in a front view, a
    /// cut edge in a side view) — its facets read as one line, so they merge to a single
    /// edge spanning the curve's projected extremes instead of a heap of slivers.
    #[test]
    fn pick_edges_merge_an_edge_on_curve_to_one_spanning_edge() {
        let flat = |p: Vec3| glam::vec2(p.x, p.z);
        // A semicircular rim in the XY plane, seen edge-on from X: every facet projects
        // onto the same vertical line.
        let edges: Vec<(Vec3, Vec3)> = (0..24)
            .map(|i| {
                let a0 = i as f32 / 24.0 * std::f32::consts::PI;
                let a1 = (i + 1) as f32 / 24.0 * std::f32::consts::PI;
                (
                    Vec3::new(0.0, 5.0 + a0.cos() * 4.0, 5.0 + a0.sin() * 4.0),
                    Vec3::new(0.0, 5.0 + a1.cos() * 4.0, 5.0 + a1.sin() * 4.0),
                )
            })
            .collect();
        let logical = logical_pick_edges(&edges, &flat);
        assert_eq!(logical.len(), 1, "one spanning edge: {logical:?}");
        let (a, b) = logical[0];
        // The span runs between the arc's two extreme points on the page (either order).
        let (lo, hi) = if a.z <= b.z { (a.z, b.z) } else { (b.z, a.z) };
        assert!((lo - 5.0).abs() < 1e-3 && (hi - 9.0).abs() < 1e-3, "{a:?} – {b:?}");
    }

    /// #1652: an angle dimension draws as an arc between the two edges, centred on the corner
    /// they share, with an arrowhead at each end and the degree label just outside the arc.
    #[test]
    fn angle_dimension_arcs_between_the_two_edges() {
        let corner = glam::Vec2::new(2.0, 3.0);
        let g = angle_dim_geometry(
            (corner, corner + glam::Vec2::new(0.0, 10.0)),
            (corner, corner + glam::Vec2::new(6.0, 0.0)),
            0.5,
        )
        .expect("two edges meeting at a corner have an angle");
        assert!((g.degrees - 90.0).abs() < 1e-3, "square corner reads 90 degrees");
        assert!((g.center - corner).length() < 1e-4, "the arc sits on the shared corner");
        // The arc stays inside the shorter edge, and both of its ends land on the edges.
        assert!(g.radius > 0.0 && g.radius < 6.0);
        let pts = g.arc_points();
        let ends = [pts[0], *pts.last().unwrap()];
        assert!(
            (ends[0] - (corner + glam::Vec2::new(g.radius, 0.0))).length() < 1e-3
                || (ends[1] - (corner + glam::Vec2::new(g.radius, 0.0))).length() < 1e-3,
            "one arc end runs along the horizontal edge: {ends:?}"
        );
        assert!(
            (ends[0] - (corner + glam::Vec2::new(0.0, g.radius))).length() < 1e-3
                || (ends[1] - (corner + glam::Vec2::new(0.0, g.radius))).length() < 1e-3,
            "the other runs along the vertical edge: {ends:?}"
        );
        // Sampling the arc walks the sweep, every point at the radius from the corner.
        assert!(pts.len() >= 8);
        assert!(pts.iter().all(|p| ((*p - corner).length() - g.radius).abs() < 1e-3));
        // The label sits outside the arc, in the wedge between the two edges.
        let off = g.label - corner;
        assert!(off.length() > g.radius, "the label clears the arc");
        assert!(off.x > 0.0 && off.y > 0.0, "and stays in the measured wedge");
    }

    /// #1652: edges that only meet when produced still get an arc — at the crossing point of
    /// the two lines, opening toward the ends that are actually drawn.
    #[test]
    fn angle_dimension_uses_the_crossing_of_edges_that_do_not_touch() {
        let g = angle_dim_geometry(
            (glam::Vec2::new(4.0, 0.0), glam::Vec2::new(10.0, 0.0)),
            (glam::Vec2::new(0.0, 4.0), glam::Vec2::new(0.0, 10.0)),
            0.5,
        )
        .expect("the produced lines cross at the origin");
        assert!((g.center - glam::Vec2::ZERO).length() < 1e-3);
        assert!((g.degrees - 90.0).abs() < 1e-3);
        assert_eq!(g.extensions.len(), 2, "each edge is produced back to the corner");
    }

    /// #1652: parallel edges never meet, so there is nothing to arc between.
    #[test]
    fn angle_dimension_declines_parallel_edges() {
        assert!(angle_dim_geometry(
            (glam::Vec2::ZERO, glam::Vec2::new(10.0, 0.0)),
            (glam::Vec2::new(0.0, 5.0), glam::Vec2::new(10.0, 5.0)),
            0.5,
        )
        .is_none());
    }

    /// #1206: facing endpoints use the silhouette at each shared-axis extreme, not AABB corners.
    /// An L-shape's top-left AABB corner floats; the left extreme's top is the bar height.
    #[test]
    fn silhouette_facing_point_avoids_floating_aabb_corners() {
        // Horizontal bar (0..10)×(0..2) plus a stem on the right (8..10)×(2..10).
        let pts = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(0.0, 2.0),
            glam::Vec2::new(10.0, 2.0),
            glam::Vec2::new(10.0, 0.0),
            glam::Vec2::new(8.0, 2.0),
            glam::Vec2::new(8.0, 10.0),
            glam::Vec2::new(10.0, 10.0),
        ];
        // Left extreme, facing up (max y): bar top at y=2, not AABB top at y=10.
        let left_top = silhouette_facing_point(&pts, true, true, false).unwrap();
        assert!((left_top.x - 0.0).abs() < 1e-5 && (left_top.y - 2.0).abs() < 1e-5, "{left_top:?}");
        // Right extreme, facing up: stem top at y=10.
        let right_top = silhouette_facing_point(&pts, true, false, false).unwrap();
        assert!((right_top.x - 10.0).abs() < 1e-5 && (right_top.y - 10.0).abs() < 1e-5, "{right_top:?}");
        // Bottom extreme on shared Y, facing right (max x): right side of the bar.
        let bot_right = silhouette_facing_point(&pts, false, true, false).unwrap();
        assert!((bot_right.y - 0.0).abs() < 1e-5 && (bot_right.x - 10.0).abs() < 1e-5, "{bot_right:?}");
    }

    /// #1229: view card border + Delete ✕ only when selected or hovered (not idle).
    #[test]
    fn view_card_chrome_only_when_selected_or_hovered() {
        assert!(!view_card_chrome_active(false, false, false, false));
        assert!(view_card_chrome_active(true, false, false, false));
        assert!(view_card_chrome_active(false, true, false, false));
        assert!(view_card_chrome_active(false, false, true, false));
        assert!(view_card_chrome_active(false, false, false, true));
        assert!(view_card_chrome_active(true, true, true, true));
    }

    /// #1207: corner-grip hit testing and centre-fixed size math for view card resize.
    #[test]
    fn view_resize_handles_hit_and_size_from_drag() {
        let center = glam::Vec2::new(100.0, 80.0);
        let corners = view_card_corners(center, 40.0, 30.0);
        assert_eq!(corners[0], glam::Vec2::new(80.0, 65.0)); // TL
        assert_eq!(corners[2], glam::Vec2::new(120.0, 95.0)); // BR
        assert_eq!(
            view_resize_handle_hit(glam::Vec2::new(120.0, 95.0), &corners, 6.0),
            Some(2)
        );
        assert_eq!(
            view_resize_handle_hit(glam::Vec2::new(100.0, 80.0), &corners, 6.0),
            None
        );
        // Drag BR further out on a 200×100 page → size 0.4 × 0.6 (2*half/page).
        let (sx, sy) = view_size_from_corner_drag(
            center,
            glam::Vec2::new(140.0, 110.0),
            glam::Vec2::new(200.0, 100.0),
        );
        assert!((sx - 0.4).abs() < 1e-4 && (sy - 0.6).abs() < 1e-4, "got {sx}×{sy}");
    }

    /// #1207: size_x propagates across Above/Below, size_y across Left/Right.
    #[test]
    fn apply_view_size_propagates_linked_axes() {
        use crate::model::AlignDir;
        let base = DrawingView::from_bodies(vec![bkey(0)], DrawingOrientation::Top);
        let mut views = vec![
            base.clone(),
            DrawingView {
                aligned_parent: Some(0),
                aligned_dir: Some(AlignDir::Right),
                ..base.clone()
            },
            DrawingView {
                aligned_parent: Some(0),
                aligned_dir: Some(AlignDir::Below),
                ..base
            },
        ];
        apply_view_size(&mut views, 0, 0.2, 0.3);
        assert!((views[0].size_x - 0.2).abs() < 1e-4 && (views[0].size_y - 0.3).abs() < 1e-4);
        assert!((views[1].size_x - CELL_FRAC).abs() < 1e-4 && (views[1].size_y - 0.3).abs() < 1e-4);
        assert!((views[2].size_x - 0.2).abs() < 1e-4 && (views[2].size_y - CELL_FRAC).abs() < 1e-4);
    }

    /// #345: a free-angle orientation projects with its stored basis, so a free basis equal to a
    /// preset's reproduces that preset exactly (the convention `view_cube::free_basis` is chosen so
    /// a spun Front pose == the Front projection).
    #[test]
    fn free_orientation_uses_its_stored_basis() {
        use crate::model::DrawingOrientation as O;
        let (fr, fu) = view_axes(O::Front);
        let free = O::Free { right: fr.to_array(), up: fu.to_array() };
        let (r, u) = view_axes(free);
        assert!((r - fr).length() < 1e-5 && (u - fu).length() < 1e-5, "free == Front basis");
        // A denormalised / non-orthogonal stored basis is re-orthonormalised, not trusted blindly.
        let sloppy = O::Free { right: [2.0, 0.0, 0.0], up: [0.3, 0.0, 4.0] };
        let (r, u) = view_axes(sloppy);
        assert!((r.length() - 1.0).abs() < 1e-4 && (u.length() - 1.0).abs() < 1e-4);
        assert!(r.dot(u).abs() < 1e-4, "re-orthonormalised");
    }

    /// #1644: dimensioning "the line" must mean the whole line. A straight edge broken into a
    /// segment per face that lands on it merges back into one run; a real gap does not.
    #[test]
    fn collinear_edges_merge_into_one_run() {
        let p = |x: f32, z: f32| Vec3::new(x, 20.0, z);
        // One 80 mm vertical line, cut into three by the faces meeting it at z = 30 and 50.
        let run = merge_collinear_runs(&[
            (p(20.0, 0.0), p(20.0, 30.0)),
            (p(20.0, 30.0), p(20.0, 50.0)),
            (p(20.0, 50.0), p(20.0, 80.0)),
        ]);
        assert_eq!(run.len(), 1, "the three pieces are one line, got {run:?}");
        let (a, b) = run[0];
        assert!(((b - a).length() - 80.0).abs() < 1e-3, "run is 80 mm, got {}", (b - a).length());

        // Direction doesn't matter: the same line drawn every which way still merges.
        let mixed = merge_collinear_runs(&[
            (p(20.0, 30.0), p(20.0, 0.0)),
            (p(20.0, 50.0), p(20.0, 30.0)),
            (p(20.0, 50.0), p(20.0, 80.0)),
        ]);
        assert_eq!(mixed.len(), 1, "got {mixed:?}");

        // A gap in the middle is two lines, not one.
        let gapped = merge_collinear_runs(&[
            (p(20.0, 0.0), p(20.0, 30.0)),
            (p(20.0, 50.0), p(20.0, 80.0)),
        ]);
        assert_eq!(gapped.len(), 2, "a gap keeps them apart, got {gapped:?}");

        // Parallel lines a millimetre apart stay separate, and so do crossing edges.
        let others = merge_collinear_runs(&[
            (p(20.0, 0.0), p(20.0, 30.0)),
            (p(21.0, 0.0), p(21.0, 30.0)),
            (p(20.0, 30.0), p(40.0, 30.0)),
        ]);
        assert_eq!(others.len(), 3, "got {others:?}");
    }

    /// #1820: the shaded fills were painted in one depth order per flat, taken from the flat's
    /// *farthest* point. A big face spans a range of depths, so a small face it covers could
    /// sort in front of it — the bar's shaded side leaked through onto the top of the block it
    /// grows out of. Paint order must agree with what is actually in front where two flats meet.
    #[test]
    fn shaded_fills_paint_back_to_front_where_they_overlap() {
        let bytes = std::fs::read("tests/fixtures/issue_1820.json").expect("fixture");
        let doc = crate::storage::from_json_bytes(&bytes).expect("load");
        let d = doc.drawings.values().next().expect("the drawing");
        let iso = d
            .views
            .iter()
            .find(|v| v.style == crate::model::DrawingViewStyle::Shaded)
            .expect("the shaded three-quarter view");
        let (right, up) = resolved_view_axes(&d.views, iso);
        let toward = right.cross(up);
        let geo = styled_view_geometry(&doc, &d.views, iso);
        assert!(geo.faces.len() > 3, "the view shades several flats");

        let inside = |tris: &[[glam::Vec2; 3]], p: glam::Vec2| {
            tris.iter().any(|t| {
                let area2 = (t[1] - t[0]).perp_dot(t[2] - t[0]);
                if area2.abs() < 1e-9 {
                    return false;
                }
                let w0 = (t[1] - p).perp_dot(t[2] - p) / area2;
                let w1 = (t[2] - p).perp_dot(t[0] - p) / area2;
                let w2 = 1.0 - w0 - w1;
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            })
        };
        let depth_at = |(n, c): (Vec3, f32), p: glam::Vec2| {
            (c - p.x * n.dot(right) - p.y * n.dot(up)) / n.dot(toward)
        };

        // Sample the whole drawn area; at each point the *last* face painted over it is what
        // the reader sees, and it has to be the nearest surface there.
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::INFINITY), glam::Vec2::splat(f32::NEG_INFINITY));
        for f in &geo.faces {
            for t in &f.tris {
                for p in t {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
            }
        }
        let mut checked = 0;
        for gy in 0..80 {
            for gx in 0..80 {
                let p = glam::Vec2::new(
                    lo.x + (hi.x - lo.x) * (gx as f32 + 0.5) / 80.0,
                    lo.y + (hi.y - lo.y) * (gy as f32 + 0.5) / 80.0,
                );
                let covering: Vec<usize> = (0..geo.faces.len())
                    .filter(|&i| inside(&geo.faces[i].tris, p))
                    .collect();
                let Some(&painted) = covering.last() else {
                    continue;
                };
                checked += 1;
                let shown = depth_at(geo.faces[painted].plane, p);
                for &i in &covering {
                    let d = depth_at(geo.faces[i].plane, p);
                    assert!(
                        d <= shown + 1e-3,
                        "at {p:?} face {i} sits {:.3}mm in front of the face painted over it",
                        d - shown
                    );
                }
            }
        }
        assert!(checked > 500, "the sweep should land on the solid, hit {checked} points");
    }

    /// #1713: a hidden line poked a stub out of the solid in the shaded three-quarter view.
    /// The occluding face is two triangles; a point on the seam between them was a hair
    /// outside *both*, so the hidden-line test found a crack and let the edge through there.
    #[test]
    fn a_shaded_view_shows_no_stub_of_a_hidden_line() {
        let bytes = std::fs::read("tests/fixtures/issue_1713.json").expect("fixture");
        let doc = crate::storage::from_json_bytes(&bytes).expect("load");
        let d = doc.drawings.values().next().expect("the drawing");
        let iso = d
            .views
            .iter()
            .find(|v| v.style == crate::model::DrawingViewStyle::Shaded)
            .expect("the shaded three-quarter view");
        let (right, up) = resolved_view_axes(&d.views, iso);
        let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
        let mut edges = drawing_view_world_edges(&doc, iso);
        edges.extend(drawing_view_silhouette_edges(&doc, &d.views, iso));
        let geo = styled_view_geometry(&doc, &d.views, iso);
        assert!(!geo.segments.is_empty(), "the view strokes something");
        for (pa, pb) in &geo.segments {
            // Which edge is this a run of, and how much of it survived hidden-line removal?
            for (a, b) in &edges {
                let (qa, qb) = (project(*a), project(*b));
                let full = (qb - qa).length();
                if full < 1e-6 {
                    continue;
                }
                let off = |p: &glam::Vec2| {
                    let t = ((*p - qa).dot(qb - qa) / (full * full)).clamp(0.0, 1.0);
                    (*p - (qa + (qb - qa) * t)).length()
                };
                if off(pa).max(off(pb)) < 1e-3 {
                    let frac = (*pb - *pa).length() / full;
                    // The one real partial here is a third of its edge; the stubs were
                    // a sample or two long — 3 % and 6 %.
                    assert!(
                        frac > 0.2,
                        "a {:.0} % sliver of a {full:.1} mm edge is a hidden-line leak, \
                         not geometry: ({:.2},{:.2}) -> ({:.2},{:.2})",
                        frac * 100.0,
                        pa.x,
                        pa.y,
                        pb.x,
                        pb.y
                    );
                    break;
                }
            }
        }
    }

    /// #1644: end to end on the reporter's drawing — the front view's 80 mm vertical line is
    /// one dimensionable edge, not three 20/30 mm pieces.
    #[test]
    fn a_drawing_view_dimensions_a_whole_line() {
        let bytes = std::fs::read("tests/fixtures/issue_1644.json").expect("fixture");
        let doc = crate::storage::from_json_bytes(&bytes).expect("load");
        let d = doc.drawings.values().next().expect("the drawing");
        let front = &d.views[0];
        assert_eq!(front.orientation, DrawingOrientation::Front);
        let edges = drawing_view_dimensionable_edges(&doc, &d.views, front);
        let full = edges.iter().any(|(a, b)| {
            let (lo, hi) = if a.z <= b.z { (a, b) } else { (b, a) };
            (lo.x - 20.0).abs() < 1e-3
                && (lo.y - 20.0).abs() < 1e-3
                && lo.z.abs() < 1e-3
                && (hi.z - 80.0).abs() < 1e-3
        });
        assert!(full, "the x = 20 line should be dimensionable over its whole 80 mm");
        assert!(
            !edges.iter().any(|(a, b)| {
                (a.x - 20.0).abs() < 1e-3
                    && (a.y - 20.0).abs() < 1e-3
                    && (b.x - 20.0).abs() < 1e-3
                    && (b.y - 20.0).abs() < 1e-3
                    && ((*b - *a).length() - 20.0).abs() < 1e-3
            }),
            "and its 20 mm middle piece should no longer be separately dimensionable"
        );
    }

    /// #1645: a free point-to-point dimension measures the straight distance, or the
    /// separation along one page axis.
    #[test]
    fn a_point_dimension_measures_what_its_axis_says() {
        use crate::model::{DrawingPointDim, PointDimAxis as A};
        let dim = |axis| DrawingPointDim { a: (0.0, 0.0), b: (30.0, 40.0), axis, offset: 0.0 };
        assert!((point_dim_value(&dim(A::Direct)) - 50.0).abs() < 1e-4);
        assert!((point_dim_value(&dim(A::Horizontal)) - 30.0).abs() < 1e-4);
        assert!((point_dim_value(&dim(A::Vertical)) - 40.0).abs() < 1e-4);
        // Direction doesn't matter.
        let back = DrawingPointDim { a: (30.0, 40.0), b: (0.0, 0.0), axis: A::Horizontal, offset: 0.0 };
        assert!((point_dim_value(&back) - 30.0).abs() < 1e-4);
    }

    /// #1645: the dimension line runs along the measured axis and clears *both* picked points,
    /// so each extension line is visible however the two are offset.
    #[test]
    fn a_point_dimension_line_clears_both_points() {
        use crate::model::{DrawingPointDim, PointDimAxis as A};
        let gap = 6.0;
        let h = DrawingPointDim { a: (0.0, 0.0), b: (30.0, 40.0), axis: A::Horizontal, offset: 0.0 };
        let (pa, pb, _) = point_dim_line(&h, gap);
        assert!((pa.y - pb.y).abs() < 1e-4, "a horizontal dimension line is horizontal");
        assert!((pa.x - h.a.0).abs() < 1e-4 && (pb.x - h.b.0).abs() < 1e-4, "spans the two points");
        assert!(pa.y <= -gap + 1e-4, "and sits clear of the lower point, at {}", pa.y);

        let v = DrawingPointDim { a: (0.0, 0.0), b: (30.0, 40.0), axis: A::Vertical, offset: 0.0 };
        let (va, vb, _) = point_dim_line(&v, gap);
        assert!((va.x - vb.x).abs() < 1e-4, "a vertical dimension line is vertical");
        assert!((va.y - v.a.1).abs() < 1e-4 && (vb.y - v.b.1).abs() < 1e-4);

        // Direct: the two points are level with the line, which is parallel to a→b.
        let d = DrawingPointDim { a: (0.0, 0.0), b: (30.0, 40.0), axis: A::Direct, offset: 0.0 };
        let (da, db, out) = point_dim_line(&d, gap);
        assert!(((db - da).length() - 50.0).abs() < 1e-3, "spans the full 50 mm");
        assert!(out.dot(db - da).abs() < 1e-3, "outward is perpendicular to the line");
    }

    /// #1643: a drawing view and the navigation bear must agree on where the eye is. A view's
    /// `right × up` is the direction *out of the page toward the viewer*, which is exactly the
    /// bear's outward view direction for that pick. Top and Bottom were swapped, so picking the
    /// bear's bottom corner drew the view from above.
    #[test]
    fn face_views_look_from_where_the_bear_says() {
        use crate::camera::StandardView as S;
        use crate::model::DrawingOrientation as O;
        for (o, v) in [
            (O::Front, S::Front),
            (O::Back, S::Back),
            (O::Left, S::Left),
            (O::Right, S::Right),
            (O::Top, S::Top),
            (O::Bottom, S::Bottom),
        ] {
            let (r, u) = view_axes(o);
            let eye = r.cross(u);
            let bear = crate::view_cube::face_view_direction(v);
            assert!(
                (eye - bear).length() < 1e-4,
                "{o:?} looks from {eye:?}, the bear says {bear:?}"
            );
        }
    }

    /// #1643: and the same for every edge and corner view the bear offers — each is built from
    /// its faces, so a swapped Top/Bottom tilted half of them the wrong way.
    #[test]
    fn edge_and_corner_views_look_from_where_the_bear_says() {
        use crate::model::{CornerView, DrawingOrientation as O, EdgeView};
        use crate::view_cube::{CubeCornerId as CC, CubeEdgeId as CE};
        let corner_id = |c| match c {
            CornerView::FrontLeftBottom => CC::FrontLeftBottom,
            CornerView::FrontRightBottom => CC::FrontRightBottom,
            CornerView::BackRightBottom => CC::BackRightBottom,
            CornerView::BackLeftBottom => CC::BackLeftBottom,
            CornerView::FrontLeftTop => CC::FrontLeftTop,
            CornerView::FrontRightTop => CC::FrontRightTop,
            CornerView::BackRightTop => CC::BackRightTop,
            CornerView::BackLeftTop => CC::BackLeftTop,
        };
        for &c in CornerView::ALL {
            let (r, u) = view_axes(O::Corner(c));
            let eye = r.cross(u);
            let bear = crate::view_cube::corner_view_direction(corner_id(c));
            assert!(
                (eye - bear).length() < 1e-4,
                "corner {c:?} looks from {eye:?}, the bear says {bear:?}"
            );
        }
        let edge_id = |e| match e {
            EdgeView::FrontRight => CE::FrontRight,
            EdgeView::BackRight => CE::BackRight,
            EdgeView::BackLeft => CE::BackLeft,
            EdgeView::FrontLeft => CE::FrontLeft,
            EdgeView::FrontTop => CE::FrontTop,
            EdgeView::RightTop => CE::RightTop,
            EdgeView::BackTop => CE::BackTop,
            EdgeView::LeftTop => CE::LeftTop,
            EdgeView::FrontBottom => CE::FrontBottom,
            EdgeView::RightBottom => CE::RightBottom,
            EdgeView::BackBottom => CE::BackBottom,
            EdgeView::LeftBottom => CE::LeftBottom,
        };
        for &e in EdgeView::ALL {
            let (r, u) = view_axes(O::Edge(e));
            let eye = r.cross(u);
            let bear = crate::view_cube::edge_view_direction(edge_id(e));
            assert!(
                (eye - bear).length() < 1e-4,
                "edge {e:?} looks from {eye:?}, the bear says {bear:?}"
            );
        }
    }

    /// #1643: third-angle placement is what the unfolding must keep — the Top view goes *above*
    /// a Front base and the Bottom view below it, whichever way the axes are spelled.
    #[test]
    fn aligned_children_of_a_front_base_land_third_angle() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Above), Some(O::Top));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Below), Some(O::Bottom));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Right), Some(O::Right));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Left), Some(O::Left));
    }

    /// #351: an aligned child unfolds from its parent's basis for *any* base orientation, so a Top
    /// base yields Front below, Back above, and rotated Left/Right to the sides — all four
    /// directions offerable, and each rendered with the correct (possibly rotated) basis.
    #[test]
    fn aligned_children_unfold_for_a_top_base() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        // All four directions are offered from a Top base (not just Below).
        for dir in [AlignDir::Below, AlignDir::Above, AlignDir::Left, AlignDir::Right] {
            assert!(aligned_child_orientation(O::Top, dir).is_some(), "{dir:?} offered");
        }
        assert_eq!(aligned_child_orientation(O::Top, AlignDir::Below), Some(O::Front));

        // The rendered bases come from resolved_view_axes unfolding the Top parent (X, Y).
        let parent = DrawingView {
            cross_section: None,
            bodies: vec![bkey(0)], sketch: None, orientation: O::Top,
            dimensioned_edges: Vec::new(), angle_dims: Vec::new(), dimension_offsets: Vec::new(),
            dimensioned_circles: Vec::new(), dimensioned_curves: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(), loupes: Vec::new(), aligned_parent: None, aligned_dir: None,
            scale: None, style: Default::default(), pos_x: 0.5, pos_y: 0.5,
            size_x: CELL_FRAC, size_y: CELL_FRAC,
            align_lines: false,
label_hidden: false, label_pos: Default::default(), label_text: None,
        };
        let child = |dir| DrawingView {
            aligned_parent: Some(0), aligned_dir: Some(dir),
            // Children carry their default (auto-derived) orientation, so no #367 ring roll applies.
            orientation: aligned_child_orientation(O::Top, dir).unwrap(),
            ..parent.clone()
        };
        let views = |dir| vec![parent.clone(), child(dir)];
        // Top parent basis = (X, Y), eye direction = X×Y = +Z (#1643).
        let vb = views(AlignDir::Below);
        assert_eq!(resolved_view_axes(&vb, &vb[1]), (Vec3::X, Vec3::Z), "below → Front basis");
        let va = views(AlignDir::Above);
        assert_eq!(resolved_view_axes(&va, &va[1]), (Vec3::X, -Vec3::Z), "above → rotated Back");
        // The side children keep the parent's page up (world +Y), so they are the Left/Right
        // views rolled a quarter turn — not the half turn the swapped Top used to give (#1643).
        let vr = views(AlignDir::Right);
        assert_eq!(resolved_view_axes(&vr, &vr[1]), (-Vec3::Z, Vec3::Y), "right → rotated Right");
        let vl = views(AlignDir::Left);
        assert_eq!(resolved_view_axes(&vl, &vl[1]), (Vec3::Z, Vec3::Y), "left → rotated Left");
    }

    /// #332: an aligned child dragged to the side of a Front parent can be re-oriented to any of
    /// the four views that share the vertical axis (Front/Back/Left/Right), and one dragged above
    /// or below to the four sharing the horizontal axis (Front/Back/Top/Bottom).
    #[test]
    fn aligned_inline_orientations_stay_in_line() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        let side = aligned_inline_orientations(O::Front, AlignDir::Right);
        for o in [O::Back, O::Left, O::Right] {
            assert!(side.contains(&o), "{o:?} should be an in-line side view");
        }
        // The base's own orientation is excluded from the ring (#367) — a right view pointing
        // Front would just duplicate the base.
        assert!(!side.contains(&O::Front), "the base orientation is not offered");
        // The diagonal vertical-edge views (#339) share the vertical axis too, so they're in-line.
        use crate::model::EdgeView;
        for e in [EdgeView::FrontRight, EdgeView::BackRight, EdgeView::BackLeft, EdgeView::FrontLeft] {
            assert!(side.contains(&O::Edge(e)), "{e:?} should be an in-line diagonal");
        }
        assert!(!side.contains(&O::Top) && !side.contains(&O::Bottom));
        assert!(!side.contains(&O::Isometric));
        assert!(!side.contains(&O::Edge(EdgeView::FrontTop)), "tilted edges aren't in-line here");

        let stack = aligned_inline_orientations(O::Front, AlignDir::Below);
        for o in [O::Back, O::Top, O::Bottom] {
            assert!(stack.contains(&o), "{o:?} should be an in-line stacked view");
        }
        assert!(!stack.contains(&O::Front), "the base orientation is not offered");
        assert!(!stack.contains(&O::Left) && !stack.contains(&O::Right));
    }

    /// #367: an aligned child re-oriented to a ring member renders exactly that orientation while
    /// staying lined up. For an axis-aligned (Front) base the unfold is identity, so the rolled
    /// basis equals the chosen orientation's canonical basis for every ring member.
    #[test]
    fn aligned_child_ring_roll_renders_the_chosen_orientation() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        let parent = DrawingView {
            cross_section: None,
            bodies: vec![bkey(0)], sketch: None, orientation: O::Front,
            dimensioned_edges: Vec::new(), angle_dims: Vec::new(), dimension_offsets: Vec::new(),
            dimensioned_circles: Vec::new(), dimensioned_curves: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(), loupes: Vec::new(), aligned_parent: None, aligned_dir: None,
            scale: None, style: Default::default(), pos_x: 0.5, pos_y: 0.5,
            size_x: CELL_FRAC, size_y: CELL_FRAC,
            align_lines: false,
label_hidden: false, label_pos: Default::default(), label_text: None,
        };
        for dir in [AlignDir::Right, AlignDir::Left, AlignDir::Above, AlignDir::Below] {
            for o in aligned_inline_orientations(O::Front, dir) {
                let child = DrawingView {
                    aligned_parent: Some(0), aligned_dir: Some(dir), orientation: o, ..parent.clone()
                };
                let views = vec![parent.clone(), child];
                let (gr, gu) = resolved_view_axes(&views, &views[1]);
                let (wr, wu) = view_axes(o);
                assert!(
                    (gr - wr).length() < 1e-4 && (gu - wu).length() < 1e-4,
                    "{dir:?}/{o:?}: got ({gr:?},{gu:?}) want ({wr:?},{wu:?})"
                );
            }
        }
    }

    /// #339: every edge view projects with a valid orthonormal basis, and a vertical-edge view
    /// (Front-Right) is the 45° rotation of Front about the vertical axis.
    #[test]
    fn edge_view_bases_are_orthonormal() {
        use crate::model::{DrawingOrientation as O, EdgeView};
        for e in EdgeView::ALL {
            let (r, u) = view_axes(O::Edge(*e));
            assert!((r.length() - 1.0).abs() < 1e-4, "{e:?} right unit");
            assert!((u.length() - 1.0).abs() < 1e-4, "{e:?} up unit");
            assert!(r.dot(u).abs() < 1e-4, "{e:?} right ⟂ up");
        }
        // Front is (right=X, up=Z); Front-Right rotates 45° about Z → right=(X+Y)/√2, up=Z.
        let (r, u) = view_axes(O::Edge(EdgeView::FrontRight));
        let inv = 1.0 / 2.0_f32.sqrt();
        assert!((r - glam::Vec3::new(inv, inv, 0.0)).length() < 1e-4, "got {r:?}");
        assert!((u - glam::Vec3::Z).length() < 1e-4, "got {u:?}");
    }

    /// #344: every corner view projects with a valid orthonormal basis, and distinct corners give
    /// distinct views (not one fixed isometric).
    #[test]
    fn corner_view_bases_are_orthonormal_and_distinct() {
        use crate::model::{CornerView, DrawingOrientation as O};
        let mut outs = Vec::new();
        for c in CornerView::ALL {
            let (r, u) = view_axes(O::Corner(*c));
            assert!((r.length() - 1.0).abs() < 1e-4, "{c:?} right unit");
            assert!((u.length() - 1.0).abs() < 1e-4, "{c:?} up unit");
            assert!(r.dot(u).abs() < 1e-4, "{c:?} right ⟂ up");
            outs.push(r.cross(u));
        }
        // The eight corner view directions are all different.
        for i in 0..outs.len() {
            for j in (i + 1)..outs.len() {
                assert!((outs[i] - outs[j]).length() > 0.1, "corners {i},{j} share a view");
            }
        }
    }

    /// #314: a label that fits runs centred along the dimension line (angle matches, kept
    /// upright); one too wide sits past the far end, horizontal.
    #[test]
    fn dimension_label_runs_along_or_beside_the_line() {
        use std::f32::consts::FRAC_PI_2;
        let out = glam::Vec2::new(0.0, -1.0);
        // A long horizontal line: label fits, angle ~0, centred on the midpoint.
        let (pos, ang) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(100.0, 0.0),
            out,
            30.0,
            11.0,
            5.0,
        );
        assert!(ang.abs() < 1e-3, "horizontal line → horizontal label");
        assert!((pos.x - 50.0).abs() < 1e-3, "centred along the line");
        // A downward vertical line (screen y grows down): the label reads bottom-to-top
        // (angle −90°), never top-to-bottom (#322).
        let (_, ang_v) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(0.0, 100.0),
            glam::Vec2::new(1.0, 0.0),
            30.0,
            11.0,
            5.0,
        );
        assert!((ang_v + FRAC_PI_2).abs() < 1e-3, "downward vertical → reads bottom-to-top (−90°)");
        // The reverse direction reads the same way.
        assert!(
            (readable_text_angle(glam::Vec2::new(0.0, -1.0)) + FRAC_PI_2).abs() < 1e-3,
            "upward vertical also reads bottom-to-top"
        );
        // A down-to-the-right slope is allowed to read top-left → bottom-right (positive angle).
        assert!(readable_text_angle(glam::Vec2::new(1.0, 1.0)) > 0.0);
        // A short line: label can't fit, so it sits past the far end (x > line end), horizontal.
        let (pos_s, ang_s) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(4.0, 0.0),
            out,
            30.0,
            11.0,
            5.0,
        );
        assert!(ang_s.abs() < 1e-3, "short line → horizontal label");
        assert!(pos_s.x > 4.0, "label sits past the far end");
        // The returned point is the visual centre, offset `gap` plus half a glyph off the
        // line so a centred 11 pt box clears the stroke (#1350, #1716).
        assert!(
            (pos.y + 5.0 + 11.0 * DIM_LABEL_MID_EM).abs() < 1e-3,
            "fitting label's visual centre clears the line by gap plus half a glyph"
        );
    }

    /// #1716: the label clears the dimension stroke by the gap *plus* half a glyph, so a
    /// centred label never crowds the line -- vertical and slanted labels most of all, where
    /// the text runs alongside the stroke for its whole width.
    #[test]
    fn dimension_label_clears_the_stroke_by_more_than_half_a_glyph() {
        let size = 11.0;
        let gap = 5.0;
        for (a, b, out) in [
            (glam::Vec2::ZERO, glam::Vec2::new(100.0, 0.0), glam::Vec2::new(0.0, -1.0)),
            (glam::Vec2::ZERO, glam::Vec2::new(0.0, 100.0), glam::Vec2::new(1.0, 0.0)),
            (glam::Vec2::ZERO, glam::Vec2::new(70.0, 70.0), glam::Vec2::new(0.7, -0.7)),
        ] {
            let (pos, _) = dimension_label_layout(a, b, out.normalize(), 30.0, size, gap);
            let mid = (a + b) * 0.5;
            let off = (pos - mid).dot(out.normalize());
            assert!(
                off >= gap + size * DIM_LABEL_MID_EM,
                "label sits {off} off the line; needs gap plus half a glyph"
            );
        }
    }

    /// #1350: SVG `text_rot` treats `(x, y)` as the visual centre (matching the editor's
    /// galley-centred `TextShape`), not the alphabetic baseline — otherwise horizontal
    /// labels land on the dimension line in the PDF/SVG while sitting beside it on screen.
    #[test]
    fn svg_text_rot_centers_on_the_layout_point() {
        let mut c = SvgCanvas { body: String::new(), hand_lettered: false };
        c.text_rot(100.0, 50.0, 11.0, Anchor::Middle, "80.0 mm", 0.0);
        assert!(
            c.body.contains("dominant-baseline=\"central\""),
            "dimension labels must be vertically centred, got {}",
            c.body
        );
        assert!(
            c.body.contains("x=\"100.0\"") && c.body.contains("y=\"50.0\""),
            "layout point is the visual centre, got {}",
            c.body
        );
    }

    /// #1350: PDF `text_rot` shifts the baseline so capital glyphs centre on the layout
    /// point. For horizontal 11 pt Helvetica that's 0.35em below the given y (PDF y-up).
    #[test]
    fn pdf_text_rot_shifts_baseline_to_center_glyphs() {
        let mut c = PdfCanvas::new(200.0);
        c.text_rot(100.0, 50.0, 11.0, Anchor::Middle, "80.0 mm", 0.0);
        let s = String::from_utf8_lossy(&c.ops);
        // page_h - y - 0.35*size = 200 - 50 - 3.85 = 146.15
        assert!(
            s.contains("146.15"),
            "baseline should sit 0.35em below the layout point so glyphs centre on it, got {s}"
        );
    }

    /// #321: two parallel dimensions on the same side whose spans overlap land on different
    /// tiers (different offsets); a non-overlapping pair shares the innermost tier.
    #[test]
    fn overlapping_parallel_dimensions_get_staggered() {
        let out = glam::Vec2::new(0.0, -1.0);
        // Two horizontal dimensions on the same side, spans overlapping in x.
        let dims = vec![
            (glam::Vec2::new(0.0, 0.0), glam::Vec2::new(10.0, 0.0), out),
            (glam::Vec2::new(2.0, 0.0), glam::Vec2::new(8.0, 0.0), out),
        ];
        let offs = plan_dimension_tiers(&dims, 1.0);
        assert!(
            (offs[0] - offs[1]).abs() > 1e-3,
            "overlapping parallel dims should be on different tiers: {offs:?}"
        );
        // Two horizontal dims on the same side but non-overlapping spans → same tier (0).
        let dims2 = vec![
            (glam::Vec2::new(0.0, 0.0), glam::Vec2::new(10.0, 0.0), out),
            (glam::Vec2::new(20.0, 0.0), glam::Vec2::new(30.0, 0.0), out),
        ];
        let offs2 = plan_dimension_tiers(&dims2, 1.0);
        assert!((offs2[0]).abs() < 1e-4 && (offs2[1]).abs() < 1e-4, "non-overlapping share tier 0");
    }

    /// #313/#319: a tessellated circle in a plane is detected in 3D (centre/radius/normal); a
    /// run of straight edges is not. Projected face-on it's Round, edge-on it's a line.
    #[test]
    fn detects_a_world_circle_and_projects_it() {
        // A 32-gon of radius 10 in the XY plane (normal +Z), centred at (5, 3, 0).
        let n = 32;
        let c = Vec3::new(5.0, 3.0, 0.0);
        let r = 10.0;
        let pts: Vec<Vec3> = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                c + Vec3::new(a.cos(), a.sin(), 0.0) * r
            })
            .collect();
        let mut edges: Vec<(Vec3, Vec3)> = (0..n).map(|i| (pts[i], pts[(i + 1) % n])).collect();
        // Plus a separate straight square in a different place — not a circle.
        let sq = [
            Vec3::new(40.0, 0.0, 0.0),
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(50.0, 10.0, 0.0),
            Vec3::new(40.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            edges.push((sq[i], sq[(i + 1) % 4]));
        }
        let circles = classify_world_circles(&edges);
        assert_eq!(circles.len(), 1, "one circle (the 32-gon, not the square)");
        assert!((circles[0].radius - r).abs() < 0.3);
        // Looking down +Z (Top view: right=X, up=-Y) the circle faces us → Round.
        match project_world_circle(&circles[0], Vec3::X, -Vec3::Y) {
            ProjectedCircle::Round { radius, .. } => assert!((radius - r).abs() < 0.3),
            _ => panic!("face-on circle should project Round"),
        }
        // Looking along the plane (Front view: right=X, up=Z) it's edge-on → a line.
        match project_world_circle(&circles[0], Vec3::X, Vec3::Z) {
            ProjectedCircle::EdgeOn { a, b } => assert!(((a - b).length() - 2.0 * r).abs() < 0.5),
            _ => panic!("edge-on circle should project EdgeOn"),
        }
        // A 45° horizontal edge view (#369) is still edge-on for a Z-normal circle: it must
        // project to a full-width line, not a smaller floating round circle.
        let (right, up) = view_axes(DrawingOrientation::Edge(crate::model::EdgeView::FrontRight));
        match project_world_circle(&circles[0], right, up) {
            ProjectedCircle::EdgeOn { a, b } => {
                assert!(
                    ((a - b).length() - 2.0 * r).abs() < 0.5,
                    "the edge-on line spans the full diameter, got {}",
                    (a - b).length()
                );
                assert!(
                    (a.y - b.y).abs() < 1e-3,
                    "a horizontal cap circle projects to a horizontal line, got {a:?}..{b:?}"
                );
            }
            other => panic!("45° edge view of a flat circle should be EdgeOn, got {other:?}"),
        }
        // A corner view of the same flat circle (#1775, the reported bug) is angled: its
        // normal meets the view direction at ~54.7°, so the caps project to ellipses —
        // neither a full-radius circle nor an edge-on line.
        let (right, up) = view_axes(DrawingOrientation::Corner(
            crate::model::CornerView::FrontRightTop,
        ));
        match project_world_circle(&circles[0], right, up) {
            ProjectedCircle::Angled { major, minor, .. } => {
                assert!((major.length() - r).abs() < 0.3, "major semi-axis is the radius");
                let expect = r * right.cross(up).normalize().dot(circles[0].normal).abs();
                assert!(
                    (minor.length() - expect).abs() < 0.3,
                    "minor semi-axis foreshortens by |n·d|, got {} vs {}",
                    minor.length(),
                    expect
                );
            }
            other => panic!("a corner view should project the caps Angled, got {other:?}"),
        }
    }

    /// #1775: an angled (corner) view projects a circle to its true **ellipse** — major
    /// semi-axis `r` along the rim's horizon, minor `r·|n·d|` along the tilt — not a
    /// full-radius circle floating past the body's silhouette.
    #[test]
    fn an_angled_view_projects_a_circle_to_an_ellipse() {
        let s = 45f32.to_radians();
        // A Z-normal circle seen from a 45°-tilted direction (n·d = cos 45°).
        let n = Vec3::new(0.0, -s.sin(), s.cos());
        let c = WorldCircle { center: Vec3::ZERO, radius: 10.0, normal: n };
        match project_world_circle(&c, Vec3::X, Vec3::Y) {
            ProjectedCircle::Angled { major, minor, .. } => {
                assert!((major.length() - 10.0).abs() < 1e-3, "major semi-axis is r");
                assert!(
                    (minor.length() - 10.0 * s.cos()).abs() < 1e-3,
                    "minor semi-axis is r·|n·d|, got {}",
                    minor.length()
                );
                assert!(
                    (major.x * minor.x + major.y * minor.y).abs() < 1e-3,
                    "the axes are perpendicular"
                );
                // The major axis runs along the rim's horizon: horizontal here.
                assert!(major.y.abs() < 1e-3, "major axis along X, got {major:?}");
            }
            other => panic!("an angled circle should project Angled, got {other:?}"),
        }
        // Face-on stays Round and edge-on stays EdgeOn — the new case only covers between.
        let face = WorldCircle { center: Vec3::ZERO, radius: 10.0, normal: Vec3::Z };
        assert!(matches!(
            project_world_circle(&face, Vec3::X, Vec3::Y),
            ProjectedCircle::Round { .. }
        ));
        let edge = WorldCircle { center: Vec3::ZERO, radius: 10.0, normal: Vec3::Y };
        assert!(matches!(
            project_world_circle(&edge, Vec3::X, Vec3::Y),
            ProjectedCircle::EdgeOn { .. }
        ));
    }

    /// #1775: the ellipse helper traces a closed loop through the semi-axis endpoints, and
    /// rim segments of an angled circle lie on the projected ellipse — so they're covered by
    /// the single Ø dimension instead of being dimensioned as straight segments.
    #[test]
    fn angled_rim_segments_lie_on_the_projected_ellipse() {
        let s = 45f32.to_radians();
        let n = Vec3::new(0.0, -s.sin(), s.cos());
        // In-plane basis for the tilted circle.
        let u = Vec3::X;
        let v = n.cross(u).normalize();
        let c = WorldCircle { center: Vec3::ZERO, radius: 10.0, normal: n };
        let pc = project_world_circle(&c, Vec3::X, Vec3::Y);
        let ProjectedCircle::Angled { major, minor, .. } = &pc else {
            panic!("expected Angled");
        };
        // The loop passes through the semi-axis endpoints and closes.
        let pts = angled_circle_points(glam::Vec2::ZERO, *major, *minor, 32);
        assert_eq!(pts.len(), 32);
        assert!(pts[0].distance(*major) < 1e-3, "starts at the major end");
        assert!(pts[8].distance(*minor) < 1e-3, "quarter way round hits the minor end");
        // Two consecutive rim points 10° apart project onto the ellipse.
        let world = |deg: f32| {
            let t = deg.to_radians();
            u * (10.0 * t.cos()) + v * (10.0 * t.sin())
        };
        let proj = |p: Vec3| glam::Vec2::new(p.dot(Vec3::X), p.dot(Vec3::Y));
        let a = proj(world(0.0));
        let b = proj(world(10.0));
        assert!(
            projected_segment_on_circle(a, b, std::slice::from_ref(&pc)),
            "a rim chord lies on the projected ellipse"
        );
        assert!(
            !projected_segment_on_circle(
                a + glam::Vec2::new(0.0, 5.0),
                b + glam::Vec2::new(0.0, 5.0),
                std::slice::from_ref(&pc)
            ),
            "a chord pushed off the rim does not"
        );
    }

    /// #1225: a projection card's right-click menu no longer dumps every orientation — it offers
    /// **Create aligned view** (when the card can be a base) and **Remove**. Orientation is
    /// edited in the context pane.
    #[test]
    fn projection_card_context_menu_offers_aligned_view_not_every_orientation() {
        use DrawingOrientation as O;
        assert_eq!(
            projection_card_context_actions(O::Front),
            ["Create aligned view", "Remove"]
        );
        for o in [O::Back, O::Left, O::Right, O::Top, O::Bottom] {
            assert!(
                projection_card_context_actions(o).contains(&"Create aligned view"),
                "{o:?} should offer Create aligned view"
            );
        }
        // Iso / edge / corner / free cannot parent an aligned child — only Remove.
        for o in [
            O::Isometric,
            O::Edge(crate::model::EdgeView::FrontRight),
            O::Corner(crate::model::CornerView::FrontRightTop),
            O::Free {
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
            },
        ] {
            assert_eq!(
                projection_card_context_actions(o),
                ["Remove"],
                "{o:?} should not offer Create aligned view"
            );
        }
        // And the old dump of every orientation is gone.
        let labels = projection_card_context_actions(O::Front);
        for o in O::ALL {
            assert!(
                !labels.contains(&o.label()),
                "menu must not list orientation {}",
                o.label()
            );
        }
    }

    /// #296: a Front parent's aligned children follow the issue's mapping — down→Bottom,
    /// up→Top, right→Right, left→Left — and an isometric parent has no orthographic child.
    #[test]
    fn aligned_children_of_front_follow_the_screen_direction() {
        use crate::model::AlignDir;
        use DrawingOrientation as O;
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Below), Some(O::Bottom));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Above), Some(O::Top));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Right), Some(O::Right));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Left), Some(O::Left));
        // The four upright views (up = +Z) neighbour each other around the vertical axis, so
        // their left/right children are always canonical orthographic views.
        for parent in [O::Front, O::Back, O::Left, O::Right] {
            assert!(aligned_child_orientation(parent, AlignDir::Right).is_some(), "{parent:?}");
            assert!(aligned_child_orientation(parent, AlignDir::Left).is_some(), "{parent:?}");
        }
        // Directions whose unfolded view would need a rolled (non-canonical) up simply have no
        // aligned child, and an isometric parent never resolves — the tool just won't offer it.
        assert_eq!(aligned_child_orientation(O::Isometric, AlignDir::Below), None);
    }

    fn doc_with_drawing() -> Document {
        let mut doc = Document::default();
        doc.drawings.insert(Drawing {
            name: Some("Plate".to_string()),
            views: vec![DrawingView {
                cross_section: None,
                bodies: vec![bkey(0)],
                sketch: None,
                orientation: DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(), dimensioned_curves: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(), loupes: Vec::new(),
                aligned_parent: None,
                aligned_dir: None,
                scale: None,
                style: Default::default(),
                pos_x: 0.5,
                pos_y: 0.5,
                size_x: CELL_FRAC,
                size_y: CELL_FRAC,
                align_lines: false,
label_hidden: false,
                label_pos: Default::default(),
                label_text: None,
            }],
            // The title now renders as a normal text annotation, added with the drawing (#335),
            // not a baked-in export stamp — mirror that here.
            annotations: crate::arena::Arena::from_iter([crate::model::DrawingAnnotation {
                text: "Plate".to_string(),
                pos_x: 0.045,
                pos_y: 0.02,
                size_frac: 0.028,
                wrap_frac: None,
            }]),
            ..Default::default()
        });
        doc
    }

    #[test]
    fn svg_export_is_a_document() {
        let svg = drawing_to_svg(&doc_with_drawing(), dkey(0)).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Plate"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    /// #1661: WinAnsi covers Latin-1 and the 0x80–0x9F specials, so an accented name or a
    /// µ must land as its own byte rather than a '?'. Only what WinAnsi genuinely cannot
    /// represent falls back.
    #[test]
    fn pdf_text_keeps_what_winansi_can_represent() {
        assert_eq!(pdf_text_bytes("Caf\u{e9}"), b"Caf\xe9".to_vec());
        assert_eq!(pdf_text_bytes("na\u{ef}ve"), b"na\xefve".to_vec());
        assert_eq!(pdf_text_bytes("\u{fc}\u{f1}"), vec![0xFC, 0xF1]);
        assert_eq!(pdf_text_bytes("20\u{b5}m"), b"20\xb5m".to_vec());
        assert_eq!(pdf_text_bytes("90\u{b0}"), vec![b'9', b'0', 0xB0]);
        // 0x80–0x9F WinAnsi specials.
        assert_eq!(pdf_text_bytes("\u{2026}"), vec![0x85]);
        assert_eq!(pdf_text_bytes("\u{201c}x\u{201d}"), vec![0x93, b'x', 0x94]);
        assert_eq!(pdf_text_bytes("\u{2019}"), vec![0x92]);
        assert_eq!(pdf_text_bytes("\u{20ac}"), vec![0x80]);
        assert_eq!(pdf_text_bytes("\u{2014}\u{2013}"), vec![0x97, 0x96]);
        // Escapes still apply, and what WinAnsi cannot show still degrades to '?'.
        assert_eq!(pdf_text_bytes("(a)\\"), b"\\(a\\)\\\\".to_vec());
        assert_eq!(pdf_text_bytes("\u{4e2d}"), b"?".to_vec());
    }

    #[test]
    fn pdf_export_is_a_single_page_document() {
        let pdf = drawing_to_pdf(&doc_with_drawing(), dkey(0)).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"), "has a PDF header");
        assert!(pdf.ends_with(b"%%EOF\n"), "ends at EOF marker");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type /Catalog"));
        assert!(text.contains("/BaseFont /Helvetica"));
        assert!(text.contains("startxref"));
        // The title text is emitted into the content stream.
        assert!(text.contains("(Plate) Tj"));

        // Cross-reference integrity, checked against the RAW bytes (the content stream can
        // carry non-UTF-8 WinAnsi bytes, so string indices wouldn't match byte offsets):
        // startxref points at the `xref` table and every listed offset lands on its
        // `N 0 obj` — the easy thing to get wrong in a hand-rolled PDF.
        let start = parse_startxref(&pdf);
        assert_eq!(&pdf[start..start + 4], b"xref");
        let offsets = parse_xref_offsets(&pdf[start..]);
        assert_eq!(offsets.len(), 5, "five objects in the xref");
        for (i, off) in offsets.iter().enumerate() {
            let expect = format!("{} 0 obj", i + 1);
            assert!(
                pdf[*off..].starts_with(expect.as_bytes()),
                "xref offset {off} should point at '{expect}'"
            );
        }
    }

    /// The `startxref` byte offset from a PDF's trailer.
    fn parse_startxref(pdf: &[u8]) -> usize {
        let needle = b"startxref";
        let pos = pdf.windows(needle.len()).rposition(|w| w == needle).unwrap()
            + needle.len();
        let rest = &pdf[pos..];
        let digits: Vec<u8> = rest
            .iter()
            .skip_while(|b| b.is_ascii_whitespace())
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        String::from_utf8(digits).unwrap().parse().unwrap()
    }

    /// The object byte offsets listed in an xref table (the ` n ` entries, skipping the free
    /// object 0).
    fn parse_xref_offsets(table: &[u8]) -> Vec<usize> {
        String::from_utf8_lossy(table)
            .lines()
            .filter_map(|l| {
                let l = l.trim_end();
                (l.len() == 18 && l.ends_with(" n")).then(|| l[..10].parse().unwrap())
            })
            .collect()
    }

    /// #298: the exported page is the drawing's configured mm page in PDF points — the
    /// default is landscape US-Letter, 792 × 612 pt.
    #[test]
    fn pdf_page_matches_the_configured_page_size() {
        let doc = doc_with_drawing();
        let pdf = drawing_to_pdf(&doc, dkey(0)).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("/MediaBox [0 0 792.00 612.00]"),
            "default landscape-letter MediaBox, got: {}",
            text.lines().find(|l| l.contains("MediaBox")).unwrap_or("<none>")
        );

        let mut doc = doc;
        doc.drawings[dkey(0)].page_width_mm = 210.0; // portrait A4
        doc.drawings[dkey(0)].page_height_mm = 297.0;
        let pdf = drawing_to_pdf(&doc, dkey(0)).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let media = text.lines().find(|l| l.contains("MediaBox")).unwrap().to_string();
        assert!(
            media.contains("[0 0 595.") && media.contains(" 841."),
            "A4 MediaBox in points, got: {media}"
        );
    }

    /// #297: exports are WYSIWYG — a view's card lands at its `pos_x`/`pos_y` page fraction,
    /// so two views placed apart export apart (not into a fixed grid).
    #[test]
    fn svg_places_views_at_their_page_positions() {
        let mut doc = doc_with_drawing();
        let mut second = doc.drawings[dkey(0)].views[0].clone();
        doc.drawings[dkey(0)].views[0].pos_x = 0.25;
        doc.drawings[dkey(0)].views[0].pos_y = 0.3;
        second.pos_x = 0.75;
        second.pos_y = 0.7;
        doc.drawings[dkey(0)].views.push(second);
        let svg = drawing_to_svg(&doc, dkey(0)).unwrap();
        let (page_w, page_h) = page_dims(&doc, dkey(0)).unwrap();
        // Exports have no card border (#337); each view's caption text is placed at
        // (cell_x + CELL_PAD, cell_y + 20), so its position pins the card.
        let cell_w = page_w * CELL_FRAC;
        let cell_h = page_h * CELL_FRAC;
        for (px, py) in [(0.25f32, 0.3f32), (0.75, 0.7)] {
            let x = px * page_w - cell_w * 0.5 + CELL_PAD;
            let y = py * page_h - cell_h * 0.5 + 20.0;
            let needle = format!("<text x=\"{x:.1}\" y=\"{y:.1}\"");
            assert!(svg.contains(&needle), "expected a view caption at {needle}");
        }
    }

    #[test]
    fn missing_drawing_has_no_export() {
        let doc = Document::default();
        assert!(drawing_to_svg(&doc, dkey(0)).is_none());
        assert!(drawing_to_pdf(&doc, dkey(0)).is_none());
    }

    /// #376: a detected circle's normal must not depend on the edge order it was fed in —
    /// the sign picks which end of an edge-on diameter line the label hangs past, and the
    /// editor and export each classify from their own (differently ordered) edge pass.
    #[test]
    fn world_circle_normal_is_canonical_regardless_of_edge_order() {
        let n = 32;
        let c = Vec3::new(5.0, 3.0, 0.0);
        let pts: Vec<Vec3> = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                c + Vec3::new(a.cos(), a.sin(), 0.0) * 10.0
            })
            .collect();
        let forward: Vec<(Vec3, Vec3)> = (0..n).map(|i| (pts[i], pts[(i + 1) % n])).collect();
        // The same loop traversed backwards, starting elsewhere, with each edge reversed.
        let scrambled: Vec<(Vec3, Vec3)> = (0..n)
            .map(|i| {
                let j = (n + 7 - i) % n;
                (pts[(j + 1) % n], pts[j])
            })
            .collect();
        let a = classify_world_circles(&forward);
        let b = classify_world_circles(&scrambled);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert!(
            (a[0].normal - b[0].normal).length() < 1e-4,
            "same circle, same normal: {:?} vs {:?}",
            a[0].normal,
            b[0].normal
        );
        assert!(a[0].normal.z > 0.99, "canonical sign points +Z, got {:?}", a[0].normal);
    }

    /// #372: the caption label is toggleable, positionable, and its text overridable — a
    /// hidden label exports no caption, a custom template interpolates `{param}` fields
    /// (#338), and a bottom-right label anchors at the card's far corner.
    #[test]
    fn svg_view_label_hides_moves_and_customizes() {
        let mut doc = doc_with_drawing();
        let auto_caption = "Body 0 — Front";
        let svg = drawing_to_svg(&doc, dkey(0)).unwrap();
        assert!(svg.contains(auto_caption), "default: the automatic caption exports");

        doc.drawings[dkey(0)].views[0].label_hidden = true;
        let svg = drawing_to_svg(&doc, dkey(0)).unwrap();
        assert!(!svg.contains(auto_caption), "hidden: no caption in the export");

        doc.drawings[dkey(0)].views[0].label_hidden = false;
        doc.parameters.insert(crate::model::Parameter {
            name: "w".to_string(),
            expression: "40mm".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.drawings[dkey(0)].views[0].label_text = Some("Width {w}".to_string());
        doc.drawings[dkey(0)].views[0].label_pos = crate::model::DrawingLabelPos::BottomRight;
        let svg = drawing_to_svg(&doc, dkey(0)).unwrap();
        assert!(!svg.contains(auto_caption), "a custom template replaces the auto caption");
        assert!(
            svg.contains("Width 40 mm"),
            "custom template interpolates {{param}} fields"
        );
        // Bottom-right: anchored at (cell_x + cell_w - CELL_PAD, cell_y + cell_h - 8).
        let (page_w, page_h) = page_dims(&doc, dkey(0)).unwrap();
        let view = &doc.drawings[dkey(0)].views[0];
        let x = view.pos_x * page_w + page_w * CELL_FRAC * 0.5 - CELL_PAD;
        let y = view.pos_y * page_h + page_h * CELL_FRAC * 0.5 - 8.0;
        let needle = format!("<text x=\"{x:.1}\" y=\"{y:.1}\"");
        assert!(
            svg.contains(&needle) && svg.contains("text-anchor=\"end\""),
            "bottom-right label anchors at the card corner ({needle})"
        );
    }

    /// #1784: a sectioned view's hatch strokes separately from its edges — the hatch is a
    /// fill texture, not geometry, and draws at the thinner [`HATCH_STROKE`].
    #[test]
    fn section_hatch_is_stroked_separately_from_edges() {
        let mut state = crate::actions::AppState::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.origin = [0.0, 0.0, 0.0];
        shape.width = "40".into();
        shape.depth = "40".into();
        shape.height = "40".into();
        state.apply(crate::actions::Action::CreateShape { shape });
        state.apply(crate::actions::Action::CreateCrossSection { name: None });
        let section = state.doc.cross_sections.keys().next().expect("the view");
        state.apply(crate::actions::Action::AddCrossSectionCut {
            view: Some(section),
            cut: crate::model::CrossSectionCut {
                origin: glam::Vec3::new(0.0, 0.0, 20.0),
                normal: -glam::Vec3::Z,
                ..Default::default()
            },
        });
        state.apply(crate::actions::Action::CreateDrawing { name: None });
        let drawing = state.doc.drawings.keys().next().expect("the drawing");
        state.apply(crate::actions::Action::AddDrawingView {
            drawing,
            bodies: vec![bkey(0)],
            orientation: crate::model::DrawingOrientation::Top,
        });
        state.apply(crate::actions::Action::SetDrawingViewCrossSection {
            drawing,
            view: 0,
            cross_section: Some(section),
        });
        let views = state.doc.drawings[drawing].views.clone();
        let geo = styled_view_geometry(&state.doc, &views, &views[0]);
        assert!(!geo.hatch.is_empty(), "the cut face is hatched");
        assert!(!geo.segments.is_empty(), "the edges still stroke");
        for h in &geo.hatch {
            assert!(
                !geo.segments.contains(h),
                "hatch segment {h:?} must not ride with the edges"
            );
        }
        assert!(HATCH_STROKE < MODEL_STROKE, "the hatch draws thinner than edges");
    }

    /// Two cuboids and a Colorful isometric drawing: the back one is sectioned, the front
    /// one is not. Used by [#1908] — hatch and cut-face fill must not show through the
    /// occluding body.
    fn colorful_section_occluded_by_a_front_body() -> (crate::actions::AppState, crate::model::DrawingKey)
    {
        let mut state = crate::actions::AppState::default();
        let mut back = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        // Origin is the base centre: the block occupies x,y ∈ [-20, 20], z ∈ [0, 80].
        back.origin = [0.0, 0.0, 0.0];
        back.width = "40".into();
        back.depth = "40".into();
        back.height = "80".into();
        state.apply(crate::actions::Action::CreateShape { shape: back });
        // Sit the uncut body on the isometric toward-camera ray from the cut-face centre,
        // with a gap so the solids don't interpenetrate.
        let mut front = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        front.origin = [15.0, 15.0, 45.0];
        front.width = "20".into();
        front.depth = "20".into();
        front.height = "20".into();
        state.apply(crate::actions::Action::CreateShape { shape: front });
        let material = |state: &crate::actions::AppState, name: &str| {
            state
                .doc
                .materials
                .iter()
                .find(|(_, m)| m.name == name)
                .map(|(k, _)| k)
                .unwrap_or_else(|| panic!("built-in material {name}"))
        };
        let blue = material(&state, "Blue");
        let red = material(&state, "Red");
        state.apply(crate::actions::Action::SetBodyMaterial {
            body: bkey(0),
            material: Some(blue),
        });
        state.apply(crate::actions::Action::SetBodyMaterial {
            body: bkey(1),
            material: Some(red),
        });
        state.apply(crate::actions::Action::CreateCrossSection { name: None });
        let section = state.doc.cross_sections.keys().next().expect("the view");
        state.apply(crate::actions::Action::AddCrossSectionCut {
            view: Some(section),
            cut: crate::model::CrossSectionCut {
                origin: glam::Vec3::ZERO,
                normal: -glam::Vec3::X,
                cut_bodies: Some(vec![bkey(0)]),
                ..Default::default()
            },
        });
        state.apply(crate::actions::Action::CreateDrawing { name: None });
        let drawing = state.doc.drawings.keys().next().expect("the drawing");
        state.apply(crate::actions::Action::AddDrawingView {
            drawing,
            bodies: vec![bkey(0), bkey(1)],
            orientation: crate::model::DrawingOrientation::Isometric,
        });
        state.apply(crate::actions::Action::SetDrawingViewCrossSection {
            drawing,
            view: 0,
            cross_section: Some(section),
        });
        state.apply(crate::actions::Action::SetDrawingViewStyle {
            drawing,
            view: 0,
            style: crate::model::DrawingViewStyle::Colorful,
        });
        (state, drawing)
    }

    fn point_on_segment_2d(a: glam::Vec2, b: glam::Vec2, p: glam::Vec2, tol: f32) -> bool {
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 < 1e-12 {
            return (a - p).length() < tol;
        }
        let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
        (a + ab * t - p).length() < tol
    }

    /// #1908: a Colorful drawing's section hatch is a fill texture, but it still has to
    /// lose to whatever solid stands in front of the cut — otherwise the hash marks (and the
    /// cut body's color) read through an occluding body.
    #[test]
    fn colorful_section_hatch_does_not_show_through_a_body_in_front() {
        let (state, drawing) = colorful_section_occluded_by_a_front_body();
        let views = state.doc.drawings[drawing].views.clone();
        let view = &views[0];
        let (right, up) = resolved_view_axes(&views, view);
        let toward = right.cross(up);
        let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
        let occlusion = ViewOcclusion::for_view(&state.doc, &views, view).expect("a solid");
        let world_hatch = section_hatch_world_segments(&state.doc, view);
        assert!(!world_hatch.is_empty(), "the cut face is hatched");
        let geo = styled_view_geometry(&state.doc, &views, view);
        assert!(!geo.hatch.is_empty(), "some hatch remains on the exposed cut");
        assert!(
            geo.faces.iter().any(|f| f.tint == [232, 97, 92]),
            "the front body still paints red"
        );

        let on_hatch = |q: glam::Vec2, tol: f32| {
            geo.hatch.iter().any(|(ha, hb)| point_on_segment_2d(*ha, *hb, q, tol))
        };

        // A point on the cut face whose isometric ray hits the front cube's interior.
        let behind = Vec3::new(0.0, 0.0, 40.0);
        assert!(occlusion.hides(behind), "the front cube stands in front of the cut");
        assert!(
            !on_hatch(project(behind), 1.0),
            "hatch must not run through the body that occludes the cut"
        );

        let mut hidden = 0usize;
        let mut leaked = 0usize;
        for (a, b) in &world_hatch {
            for i in 1..6 {
                let t = i as f32 / 6.0;
                let p = a.lerp(*b, t);
                if !occlusion.hides(p) {
                    continue;
                }
                hidden += 1;
                if on_hatch(project(p), 0.4) {
                    leaked += 1;
                }
            }
        }
        assert!(
            hidden >= 8,
            "the front body should hide a stretch of hatch, hid {hidden} samples"
        );
        assert!(
            leaked * 5 < hidden,
            "too much occluded hatch still stroked: {leaked}/{hidden}"
        );

        // The cut body's blue fill must not paint over the red where the red is nearer.
        let inside = |tris: &[[glam::Vec2; 3]], p: glam::Vec2| {
            tris.iter().any(|t| {
                let area2 = (t[1] - t[0]).perp_dot(t[2] - t[0]);
                if area2.abs() < 1e-9 {
                    return false;
                }
                let w0 = (t[1] - p).perp_dot(t[2] - p) / area2;
                let w1 = (t[2] - p).perp_dot(t[0] - p) / area2;
                let w2 = 1.0 - w0 - w1;
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            })
        };
        let depth_at = |(n, c): (Vec3, f32), p: glam::Vec2| {
            (c - p.x * n.dot(right) - p.y * n.dot(up)) / n.dot(toward)
        };
        let mut checked = 0usize;
        // Front cuboid: base centre (15, 15, 45), 20³ — sample its interior in world xy at
        // mid-height and project.
        for gy in 0..20 {
            for gx in 0..20 {
                let q = project(Vec3::new(
                    7.0 + 16.0 * (gx as f32 + 0.5) / 20.0,
                    7.0 + 16.0 * (gy as f32 + 0.5) / 20.0,
                    55.0,
                ));
                let covering: Vec<usize> = (0..geo.faces.len())
                    .filter(|&i| inside(&geo.faces[i].tris, q))
                    .collect();
                let Some(&painted) = covering.last() else {
                    continue;
                };
                let shown = depth_at(geo.faces[painted].plane, q);
                let nearest = covering
                    .iter()
                    .copied()
                    .max_by(|&i, &j| {
                        depth_at(geo.faces[i].plane, q)
                            .total_cmp(&depth_at(geo.faces[j].plane, q))
                    })
                    .unwrap();
                if (depth_at(geo.faces[nearest].plane, q) - shown).abs() > 1e-2 {
                    continue;
                }
                if geo.faces[nearest].tint != [232, 97, 92] {
                    continue;
                }
                checked += 1;
                assert_eq!(
                    geo.faces[painted].tint,
                    [232, 97, 92],
                    "at {q:?} the red body is in front but the page shows {:?}",
                    geo.faces[painted].tint
                );
            }
        }
        assert!(checked >= 10, "should sample the red body, hit {checked}");
    }

    /// #1908: the reporter's drawing — a Colorful isometric of a blue sectioned shell with
    /// a red body in front of the cut. The hatch (and the blue of the opened face) must not
    /// read through the red.
    #[test]
    fn colorful_section_hatch_does_not_peek_through_the_reported_drawing() {
        let bytes = std::fs::read("tests/fixtures/issue_1908.json").expect("fixture");
        let doc = crate::storage::from_json_bytes(&bytes).expect("load");
        let d = doc.drawings.values().next().expect("the drawing");
        let view = d.views.first().expect("the projection");
        assert_eq!(view.style, crate::model::DrawingViewStyle::Colorful);
        let (right, up) = resolved_view_axes(&d.views, view);
        let toward = right.cross(up);
        let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
        let occlusion = ViewOcclusion::for_view(&doc, &d.views, view).expect("a solid");
        let world_hatch = section_hatch_world_segments(&doc, view);
        assert!(!world_hatch.is_empty(), "the cut face is hatched");
        let geo = styled_view_geometry(&doc, &d.views, view);
        let on_hatch = |q: glam::Vec2| {
            geo.hatch.iter().any(|(ha, hb)| point_on_segment_2d(*ha, *hb, q, 0.4))
        };
        let mut hidden = 0usize;
        let mut leaked = 0usize;
        for (a, b) in &world_hatch {
            for i in 1..8 {
                let p = a.lerp(*b, i as f32 / 8.0);
                if !occlusion.hides(p) {
                    continue;
                }
                hidden += 1;
                if on_hatch(project(p)) {
                    leaked += 1;
                }
            }
        }
        assert!(
            hidden >= 8,
            "the red body hides some of the cut hatch, hid {hidden} samples"
        );
        assert!(
            leaked * 5 < hidden,
            "occluded hatch still stroked on the reported drawing: {leaked}/{hidden}"
        );

        let inside = |tris: &[[glam::Vec2; 3]], p: glam::Vec2| {
            tris.iter().any(|t| {
                let area2 = (t[1] - t[0]).perp_dot(t[2] - t[0]);
                if area2.abs() < 1e-9 {
                    return false;
                }
                let w0 = (t[1] - p).perp_dot(t[2] - p) / area2;
                let w1 = (t[2] - p).perp_dot(t[0] - p) / area2;
                let w2 = 1.0 - w0 - w1;
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            })
        };
        let depth_at = |(n, c): (Vec3, f32), p: glam::Vec2| {
            (c - p.x * n.dot(right) - p.y * n.dot(up)) / n.dot(toward)
        };
        let red = [232u8, 97, 92];
        let mut checked = 0usize;
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::INFINITY), glam::Vec2::splat(f32::NEG_INFINITY));
        for f in geo.faces.iter().filter(|f| f.tint == red) {
            for t in &f.tris {
                for p in t {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
            }
        }
        assert!(lo.is_finite(), "the red body paints at least one face");
        for gy in 0..40 {
            for gx in 0..40 {
                let q = glam::Vec2::new(
                    lo.x + (hi.x - lo.x) * (gx as f32 + 0.5) / 40.0,
                    lo.y + (hi.y - lo.y) * (gy as f32 + 0.5) / 40.0,
                );
                let covering: Vec<usize> = (0..geo.faces.len())
                    .filter(|&i| inside(&geo.faces[i].tris, q))
                    .collect();
                let Some(&painted) = covering.last() else {
                    continue;
                };
                let nearest = covering
                    .iter()
                    .copied()
                    .max_by(|&i, &j| {
                        depth_at(geo.faces[i].plane, q)
                            .total_cmp(&depth_at(geo.faces[j].plane, q))
                    })
                    .unwrap();
                if geo.faces[nearest].tint != red {
                    continue;
                }
                checked += 1;
                assert_eq!(
                    geo.faces[painted].tint,
                    red,
                    "at {q:?} the red body is in front but the page shows {:?}",
                    geo.faces[painted].tint
                );
                assert!(
                    !on_hatch(q),
                    "hatch still strokes over the red body at {q:?}"
                );
            }
        }
        assert!(checked >= 20, "should land on the red body, hit {checked}");
    }

    /// #1854: extruding onto a body consumes it and produces a new one. A drawing view of
    /// the consumed body must follow to the live result — it was still projecting the old
    /// solid, so the page showed the part as it was before the boss was added.
    #[test]
    fn a_drawing_view_follows_the_body_that_replaced_its_own() {
        let mut state = crate::actions::AppState::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.width = "40".into();
        shape.depth = "40".into();
        shape.height = "40".into();
        state.apply(crate::actions::Action::CreateShape { shape });
        let original = state.doc.bodies.keys().next().expect("the cuboid's body");
        state.apply(crate::actions::Action::CreateDrawing { name: None });
        let drawing = state.doc.drawings.keys().next().expect("the drawing");
        state.apply(crate::actions::Action::AddDrawingView {
            drawing,
            bodies: vec![original],
            orientation: crate::model::DrawingOrientation::Front,
        });
        let before = crate::extrude::mesh_signed_volume(
            &drawing_view_solid_mesh(&state.doc, &state.doc.drawings[drawing].views[0])
                .expect("the view projects the cuboid"),
        )
        .abs();
        assert!((before - 64000.0).abs() < 50.0, "the 40³ cuboid, got {before}");

        // A 10×10 boss 10 tall on the top face: the cuboid body is consumed, a new one lives.
        let top = crate::model::FaceId::PrimitiveFace {
            primitive: state.doc.primitives.keys().next().expect("the cuboid"),
            face: crate::model::PrimitiveFace::CuboidTop,
        };
        let sketch = state.doc.add_sketch(top);
        let rect = crate::construction::add_line_rectangle(
            &mut state.doc, sketch, 5.0, 5.0, 10.0, 10.0, [false; 4],
        );
        state.apply(crate::actions::Action::CreateExtrusion {
            expression: None,
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(rect.to_vec())],
            distance: 10.0,
            body: crate::actions::ExtrudeBodyChoice::Merge,
            target: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: None,
        });
        assert!(
            state.doc.bodies[original].shadow,
            "the merge should consume the original body"
        );
        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(live.len(), 1, "one live body after the merge, got {live:?}");

        let view = &state.doc.drawings[drawing].views[0];
        assert_eq!(
            drawing_view_bodies(&state.doc, view),
            live,
            "the view should project the body that replaced its own"
        );
        let after = crate::extrude::mesh_signed_volume(
            &drawing_view_solid_mesh(&state.doc, view).expect("the view still projects"),
        )
        .abs();
        assert!(
            (after - 65000.0).abs() < 50.0,
            "the view should show the cuboid plus the 1000 mm³ boss, got {after}"
        );
        assert!(
            !drawing_view_source_label(&state.doc, view).contains(&format!("Body {}", original.index())),
            "the caption should name the live body, got {:?}",
            drawing_view_source_label(&state.doc, view)
        );
    }

    /// #1854, as reported: the body had already been *cut* before the boss was added, so the
    /// new source's "producing" extrusion is the older cut and the host cannot be found by
    /// peeling it. The view still has to follow — a whole chain of consumed bodies leads to
    /// one live one.
    #[test]
    fn a_drawing_view_follows_a_body_through_a_cut_then_an_add() {
        let mut state = crate::actions::AppState::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.width = "40".into();
        shape.depth = "40".into();
        shape.height = "40".into();
        state.apply(crate::actions::Action::CreateShape { shape });
        let primitive = state.doc.primitives.keys().next().expect("the cuboid");
        let original = state.doc.bodies.keys().next().expect("the cuboid's body");
        state.apply(crate::actions::Action::CreateDrawing { name: None });
        let drawing = state.doc.drawings.keys().next().expect("the drawing");
        state.apply(crate::actions::Action::AddDrawingView {
            drawing,
            bodies: vec![original],
            orientation: crate::model::DrawingOrientation::Front,
        });

        let top = crate::model::FaceId::PrimitiveFace {
            primitive,
            face: crate::model::PrimitiveFace::CuboidTop,
        };
        // A 10×10 pocket 10 deep, then a 10×10 boss 10 tall beside it.
        // The top face's frame hangs from a corner, so local (0, 0)..(40, 40) is the face.
        for (x, distance, body) in [
            (5.0, -10.0, crate::actions::ExtrudeBodyChoice::Cut),
            (25.0, 10.0, crate::actions::ExtrudeBodyChoice::Merge),
        ] {
            let sketch = state.doc.add_sketch(top.clone());
            let rect = crate::construction::add_line_rectangle(
                &mut state.doc, sketch, x, 15.0, 10.0, 10.0, [false; 4],
            );
            state.apply(crate::actions::Action::CreateExtrusion {
                expression: None,
                sketch,
                faces: vec![crate::model::ExtrudeFace::Polygon(rect.to_vec())],
                distance,
                body,
                target: None,
                symmetric: false,
                taper: 0.0,
                taper_mode: crate::model::ExtrudeTaperMode::Distance,
                taper_expression: None,
            });
        }
        let live: Vec<_> = state
            .doc
            .bodies
            .iter()
            .filter(|(_, b)| !b.shadow)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(live.len(), 1, "one live body after cut + add, got {live:?}");

        let view = &state.doc.drawings[drawing].views[0];
        assert_eq!(
            drawing_view_bodies(&state.doc, view),
            live,
            "the view should follow the whole chain to the live body"
        );
        let volume = crate::extrude::mesh_signed_volume(
            &drawing_view_solid_mesh(&state.doc, view).expect("the view still projects"),
        )
        .abs();
        assert!(
            (volume - 64000.0).abs() < 50.0,
            "cuboid − 1000 pocket + 1000 boss, got {volume}"
        );
    }

    /// #1785: a curve's length dimension renders in the export — the polyline strokes with
    /// the edges and the measured arc length labels it.
    #[test]
    fn curve_length_dimension_renders_in_the_export() {
        let mut state = crate::actions::AppState::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cylinder);
        shape.origin = [0.0, 0.0, 0.0];
        shape.normal = [0.0, 0.0, 1.0];
        shape.radius = "20".into();
        shape.height = "40".into();
        state.apply(crate::actions::Action::CreateShape { shape });
        state.apply(crate::actions::Action::CreateCrossSection { name: None });
        let section = state.doc.cross_sections.keys().next().expect("the view");
        state.apply(crate::actions::Action::AddCrossSectionCut {
            view: Some(section),
            cut: crate::model::CrossSectionCut {
                origin: glam::Vec3::new(0.0, 0.0, 0.0),
                normal: glam::Vec3::new(0.0, 1.0, 0.0),
                offset_mm: 5.0,
                flip: true,
                roll: 25f32.to_radians(),
                ..Default::default()
            },
        });
        state.apply(crate::actions::Action::CreateDrawing { name: None });
        let drawing = state.doc.drawings.keys().next().expect("the drawing");
        state.apply(crate::actions::Action::AddDrawingView {
            drawing,
            bodies: vec![bkey(0)],
            orientation: crate::model::DrawingOrientation::Front,
        });
        state.apply(crate::actions::Action::SetDrawingViewCrossSection {
            drawing,
            view: 0,
            cross_section: Some(section),
        });
        // The sectioned view offers a curve — the tilted cut's edge — and toggling it
        // stores the canonical polyline.
        let views = state.doc.drawings[drawing].views.clone();
        let view = &views[0];
        let curves = logical_pick_curves(
            &drawing_view_dimensionable_edges(&state.doc, &views, view),
            &|p: Vec3| glam::vec2(p.x, p.z),
        );
        assert!(!curves.is_empty(), "the tilted cut's edge is a curve pick");
        let key = canonical_curve_key(
            &curves[0]
                .iter()
                .map(|p| crate::hierarchy::quantize_body_point(*p))
                .collect::<Vec<_>>(),
        );
        state.apply(crate::actions::Action::ToggleDrawingCurveDimension {
            drawing,
            view: 0,
            points: key,
        });
        let svg = drawing_to_svg(&state.doc, drawing).expect("svg");
        let length = curve_chain_length(&curves[0]);
        let needle = crate::value::format_length_display_in(length, crate::value::LengthUnit::Mm);
        assert!(
            svg.contains(&needle),
            "the exported drawing shows the curve's length {needle}"
        );
    }

    /// #1785: the pick surface exposes a curve as one chain with its world points in walk
    /// order — the same polyline the click toggles and the dimension measures.
    #[test]
    fn pick_curves_come_out_as_ordered_chains_with_length() {
        let flat = |p: Vec3| glam::vec2(p.x, p.y);
        let edges: Vec<(Vec3, Vec3)> = (0..24)
            .map(|i| {
                let a0 = i as f32 / 24.0 * std::f32::consts::FRAC_PI_2;
                let a1 = (i + 1) as f32 / 24.0 * std::f32::consts::FRAC_PI_2;
                (
                    Vec3::new(10.0 + 5.0 * a0.cos(), 10.0 + 5.0 * a0.sin(), 0.0),
                    Vec3::new(10.0 + 5.0 * a1.cos(), 10.0 + 5.0 * a1.sin(), 0.0),
                )
            })
            .collect();
        let lines = logical_pick_edges(&edges, &flat);
        assert!(lines.is_empty(), "the arc's facets are not line picks");
        let curves = logical_pick_curves(&edges, &flat);
        assert_eq!(curves.len(), 1, "the arc is one curve pick");
        let chain = &curves[0];
        // Consecutive points share a facet endpoint: the chain is in walk order.
        for w in chain.windows(2) {
            assert!(
                crate::hierarchy::quantize_body_point(w[0]) != crate::hierarchy::quantize_body_point(w[1]),
                "the chain visits distinct points in order"
            );
        }
        // The chain runs from one arc end to the other (the internal merge may reorder
        // segments, so either end may come first).
        let q = |p: Vec3| crate::hierarchy::quantize_body_point(p);
        let arc_ends = [
            q(edges.first().expect("facets").0),
            q(edges.last().expect("facets").1),
        ];
        let chain_ends = [q(chain[0]), q(*chain.last().expect("points"))];
        assert!(
            (chain_ends[0] == arc_ends[0] && chain_ends[1] == arc_ends[1])
                || (chain_ends[0] == arc_ends[1] && chain_ends[1] == arc_ends[0]),
            "the chain spans the arc's two ends: {chain_ends:?} vs {arc_ends:?}"
        );
        // And its length approximates the arc: a quarter of a radius-5 circle.
        let length: f32 = chain.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let true_length = std::f32::consts::FRAC_PI_2 * 5.0;
        assert!(
            (length - true_length).abs() < true_length * 0.01,
            "chord sum {length} approximates arc {true_length}"
        );
    }

    /// #1785: a curve dimension's stored key is canonical — the same curve toggles to the
    /// same entry whichever end it was walked from.
    #[test]
    fn canonical_curve_key_is_rotation_and_direction_independent() {
        let a: [i32; 3] = [0, 0, 0];
        let b: [i32; 3] = [100, 0, 0];
        let c: [i32; 3] = [100, 100, 0];
        let forward = vec![a, b, c];
        let reversed = vec![c, b, a];
        let rotated = vec![b, c, a];
        assert_eq!(canonical_curve_key(&forward), canonical_curve_key(&reversed));
        assert_eq!(canonical_curve_key(&forward), canonical_curve_key(&rotated));
    }
}
