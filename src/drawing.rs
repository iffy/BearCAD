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
    if c.normal.dot(d).abs() < 0.15 {
        // Edge-on: the major axis is the in-plane direction perpendicular to the view.
        let w = d.cross(c.normal).normalize_or_zero();
        let major = glam::Vec2::new(w.dot(right), w.dot(up)) * c.radius;
        ProjectedCircle::EdgeOn { a: c2 - major, b: c2 + major }
    } else {
        ProjectedCircle::Round { center: c2, radius: c.radius }
    }
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
    })
}

/// PDF points per millimetre (1 pt = 1/72 in): exports are sized in points so the PDF page
/// physically matches the drawing's configured mm page (#298).
pub const PT_PER_MM: f32 = 72.0 / 25.4;
/// Default placed view card size as a fraction of the page (#297/#1207) — kept public so the
/// editor, export, and scripting agree. Per-view sizes live on each view's `size_x`/`size_y`;
/// this is only the historical default.
pub const CELL_FRAC: f32 = 0.42;
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

/// One body's mesh as this view shows it (#1689): cut by the view's cross-section planes when
/// it has any, whole otherwise.
fn drawing_view_body_mesh(
    doc: &Document,
    view: &DrawingView,
    body: crate::model::BodyKey,
) -> Option<crate::extrude::SolidMesh> {
    match drawing_view_cuts(doc, view) {
        [] => crate::extrude::body_solid_mesh(doc, body),
        cuts => crate::extrude::cross_section_body_mesh(doc, body, cuts),
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
    for &bi in &view.bodies {
        if let Some(solid) = drawing_view_body_mesh(doc, view, bi) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// Caption source label for a view: sketch name, single body name, component name when all
/// bodies belong to one component (#1190), otherwise a short multi-body summary (#1191).
pub fn drawing_view_source_label(doc: &Document, view: &DrawingView) -> String {
    use crate::hierarchy::HierarchyNode;
    use crate::names::node_label;
    if let Some(si) = view.sketch {
        return node_label(doc, HierarchyNode::Sketch(si));
    }
    match view.bodies.as_slice() {
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
        for &bi in &view.bodies {
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
    for &bi in &view.bodies {
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
/// output is the extra offset for each, in the same order. Greedy interval colouring per
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
}

/// One coplanar run of a shaded view's front faces, with the grey it's painted in
/// (0..1, 1 = white).
pub struct ShadedFace {
    /// Projected 2D triangles, all sharing one plane of the solid.
    pub tris: Vec<[glam::Vec2; 3]>,
    pub shade: f32,
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
    let mut edges = drawing_view_world_edges(doc, view);
    edges.extend(drawing_view_silhouette_edges(doc, views, view));
    // A sectioned view hatches the faces its planes opened (#1689). The lines join the
    // stroked edges *here* rather than in `drawing_view_world_edges`, so dimensioning and
    // circle detection still see only real geometry.
    edges.extend(section_hatch_world_segments(doc, view));
    let wireframe = || StyledViewGeometry {
        faces: Vec::new(),
        segments: edges.iter().map(|(a, b)| (project(*a), project(*b))).collect(),
    };
    if view.sketch.is_some() || view.style == DrawingViewStyle::Wireframe {
        return wireframe();
    }
    let Some(mesh) = drawing_view_solid_mesh(doc, view) else {
        return wireframe();
    };
    // Depth grows toward the viewer along the view's out-of-page axis.
    let toward = right.cross(up);
    let Some((lo, hi)) = mesh.bounds() else {
        return wireframe();
    };
    let eps = (hi - lo).length().max(1e-3) * 2e-3;

    // Projected triangles with per-vertex depth, for point-occlusion tests.
    struct ProjTri {
        p: [glam::Vec2; 3],
        d: [f32; 3],
        /// Twice the signed area of the projected triangle; ~0 = edge-on, skipped.
        area2: f32,
    }
    let tris: Vec<ProjTri> = mesh
        .triangles
        .iter()
        .map(|t| {
            let p = [project(t[0]), project(t[1]), project(t[2])];
            let area2 = (p[1] - p[0]).perp_dot(p[2] - p[0]);
            ProjTri { p, d: [t[0].dot(toward), t[1].dot(toward), t[2].dot(toward)], area2 }
        })
        .filter(|t| t.area2.abs() > 1e-6)
        .collect();
    /// How far outside a projected triangle a point may sit and still count as inside, in
    /// barycentric units (#1713) — enough to close the seam between two triangles of one
    /// flat, far too little to reach across a real gap.
    const BARY_TOL: f32 = 1e-5;
    // Whether some face is strictly in front of `(point, depth)`.
    let occluded = |point: glam::Vec2, depth: f32| -> bool {
        tris.iter().any(|t| {
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
            w0 * t.d[0] + w1 * t.d[1] + w2 * t.d[2] > depth + eps
        })
    };

    // Sample each edge and keep the visible runs (hidden-line removal).
    const SAMPLES: usize = 32;
    let mut segments = Vec::new();
    for (a, b) in &edges {
        let mut run_start: Option<f32> = None;
        let mut push_run = |from: f32, to: f32| {
            let wa = a.lerp(*b, from);
            let wb = a.lerp(*b, to);
            segments.push((project(wa), project(wb)));
        };
        for i in 0..SAMPLES {
            let t = (i as f32 + 0.5) / SAMPLES as f32;
            let w = a.lerp(*b, t);
            let visible = !occluded(project(w), w.dot(toward));
            match (visible, run_start) {
                (true, None) => run_start = Some(i as f32 / SAMPLES as f32),
                (false, Some(s)) => {
                    push_run(s, i as f32 / SAMPLES as f32);
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = run_start {
            push_run(s, 1.0);
        }
    }

    // Shaded fills: front faces painted back-to-front, greyed by how squarely they face a
    // fixed key light up-and-left of the viewer. Coplanar triangles are gathered into one
    // face (#1651) so a renderer paints each flat as a single surface — drawn one by one
    // they leave the tessellation's diagonals showing between them.
    let mut fills = Vec::new();
    if view.style == DrawingViewStyle::Shaded {
        let light = (toward * 1.2 - right * 0.35 + up * 0.55).normalize();
        // Plane key: the outward normal and the plane's distance from the origin, both
        // quantized so a tessellator's per-triangle rounding still lands on one flat.
        let mut planes: Vec<([i32; 4], f32, f32, Vec<[glam::Vec2; 3]>)> = Vec::new();
        for t in &mesh.triangles {
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            if n == Vec3::ZERO || n.dot(toward) <= 0.0 {
                continue; // back or degenerate face
            }
            let q = |v: f32| (v * 1000.0).round() as i32;
            let key = [q(n.x), q(n.y), q(n.z), q(n.dot(t[0]) * 0.1)];
            let shade = 0.62 + 0.33 * n.dot(light).max(0.0);
            let depth = (t[0] + t[1] + t[2]).dot(toward) / 3.0;
            let tri = [project(t[0]), project(t[1]), project(t[2])];
            match planes.iter_mut().find(|(k, ..)| *k == key) {
                Some((_, d, _, tris)) => {
                    *d = d.min(depth);
                    tris.push(tri);
                }
                None => planes.push((key, depth, shade, vec![tri])),
            }
        }
        // Farthest flat first: coplanar triangles never occlude each other, so one depth
        // per face is enough to order the painting.
        planes.sort_by(|a, b| a.1.total_cmp(&b.1));
        fills = planes
            .into_iter()
            .map(|(_, _, shade, tris)| ShadedFace { tris, shade })
            .collect();
    }

    StyledViewGeometry { faces: fills, segments }
}

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
    // edges so a smooth extrusion's length can be dimensioned (#334).
    let crease_edges = drawing_view_world_edges(doc, view);
    let world_edges = drawing_view_dimensionable_edges(doc, views, view);
    if world_edges.is_empty() {
        return;
    }
    let (right, up) = resolved_view_axes(views, view);
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
    for face in &styled.faces {
        let level = (face.shade.clamp(0.0, 1.0) * 255.0) as u8;
        let fill = Rgb(level, level, level);
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
    for (a, b) in &styled.segments {
        // A segment lying on a detected circle is drawn as part of the smooth circle instead.
        if projected_segment_on_circle(*a, *b, &pcircles) {
            continue;
        }
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
    }
    // Smooth detected circles (round) or their foreshortened diameter line (edge-on).
    for pc in &pcircles {
        match pc {
            ProjectedCircle::Round { center, radius } => {
                let sc = to_screen(*center);
                canvas.circle(sc.x, sc.y, radius * scale, BLACK, MODEL_STROKE);
            }
            ProjectedCircle::EdgeOn { a, b } => {
                let (sa, sb) = (to_screen(*a), to_screen(*b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
            }
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

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        };
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"{size}\" \
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
        // `dominant-baseline="central"` makes `(x, y)` the visual centre, matching the
        // editor and the PDF backend (#1350). Captions still use `text()`, which stays
        // baseline-aligned.
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"{size}\" \
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
    let mut canvas = SvgCanvas { body: String::new() };
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
mod tests {
    use crate::model::drawing_key_for_slot as dkey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use crate::model::{Drawing, DrawingView};

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
            dimensioned_circles: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(), aligned_parent: None, aligned_dir: None,
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
            dimensioned_circles: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(), aligned_parent: None, aligned_dir: None,
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
        let mut c = SvgCanvas { body: String::new() };
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
                dimensioned_circles: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(),
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
}


