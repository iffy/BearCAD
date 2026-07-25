//! Unit-instance evaluation (#722): an embedded unit document plus an instance's
//! parameter overrides, rebuilt into solid meshes the importing document can draw, snap
//! to, and reference.
//!
//! Evaluation is memoized by **(unit, override set)** — two instances of the same part
//! with identical overrides (the common repeated-part case) evaluate once. The memo is
//! guarded by a fingerprint of `Document.units` alone, so an override edit (new key) or
//! a sync that replaces an embedded copy (new fingerprint) re-evaluates, while edits to
//! the importing document's own geometry never do.

use crate::extrude::SolidMesh;
use crate::model::{Document, ImportedUnit};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The unit-local result of evaluating one (unit, override set) (#722).
pub struct UnitEvaluation {
    /// One mesh per live body of the rebuilt embedded document, in the unit's own
    /// coordinates — place them with [`instance_transform`].
    pub meshes: Vec<SolidMesh>,
    /// The rebuilt embedded document itself (overrides applied, geometry recomputed):
    /// the analytic structure behind the meshes, which unit references (#724) resolve
    /// against so they survive override changes the way `FaceId`-based references do.
    pub document: Document,
    /// Why the rebuild failed, if it did. A broken unit must not take the importing
    /// document down: the meshes hold whatever still built, and document health reports
    /// the instance unhealthy with this reason.
    pub error: Option<String>,
}

/// Overrides sorted by parameter name: the cache key must not care what order the
/// instance happens to store them in.
type CacheKey = (usize, Vec<(String, String)>);

thread_local! {
    /// Per-thread memo, keyed by units-fingerprint then (unit, overrides). Two levels —
    /// not one fingerprint slot — because evaluating a **nested** unit (#735) evaluates
    /// the inner document's own instances mid-flight, and a single-slot cache would
    /// thrash between the two documents every frame. Bounded: far-past fingerprints are
    /// dropped once a handful accumulate.
    static UNIT_EVAL_CACHE: RefCell<HashMap<u64, HashMap<CacheKey, Rc<UnitEvaluation>>>> =
        RefCell::new(HashMap::new());

    /// How many uncached evaluations have run on this thread (test hook: identical
    /// instances must share one).
    #[cfg(test)]
    pub static EVAL_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Fingerprint of the units themselves (their sources and embedded documents), streamed
/// through a hasher like [`crate::extrude`]'s document mesh fingerprint. Only unit
/// changes move it — the importing document's own edits leave the memo untouched (#722).
fn units_fingerprint(doc: &Document) -> u64 {
    use std::hash::Hasher;
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(std::collections::hash_map::DefaultHasher::new());
    serde_json::to_writer(&mut writer, &doc.units).ok();
    writer.0.finish()
}

/// Evaluate the unit behind `instance`, memoized. `None` for a missing/deleted instance
/// or a dangling unit index.
pub fn evaluate_instance(doc: &Document, instance: usize) -> Option<Rc<UnitEvaluation>> {
    let inst = doc.unit_instances.get(instance)?;
    if inst.deleted {
        return None;
    }
    let unit = doc.units.get(inst.unit)?;
    let mut overrides = inst.parameter_overrides.clone();
    overrides.sort();
    let key = (inst.unit, overrides);
    let fingerprint = units_fingerprint(doc);
    // The borrow is released before evaluating: a nested unit's evaluation (#735)
    // re-enters this cache for the inner document.
    let hit = UNIT_EVAL_CACHE.with(|cache| {
        cache.borrow().get(&fingerprint).and_then(|m| m.get(&key).cloned())
    });
    if let Some(hit) = hit {
        return Some(hit);
    }
    let eval = Rc::new(evaluate_uncached(unit, &inst.parameter_overrides));
    UNIT_EVAL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 8 && !cache.contains_key(&fingerprint) {
            cache.clear();
        }
        cache.entry(fingerprint).or_default().insert(key, eval.clone());
    });
    Some(eval)
}

/// Rebuild the embedded document with `overrides` applied and mesh its live bodies.
/// Never panics out: a rebuild that blows up becomes an errored evaluation.
fn evaluate_uncached(unit: &ImportedUnit, overrides: &[(String, String)]) -> UnitEvaluation {
    #[cfg(test)]
    EVAL_COUNT.with(|count| count.set(count.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scratch = unit.document.clone();
        let mut error = None;
        for (name, expression) in overrides {
            match scratch
                .parameters
                .iter_mut()
                .find(|p| !p.deleted && p.name == *name)
            {
                Some(parameter) => parameter.expression = expression.clone(),
                None => {
                    error = Some(format!("no parameter named '{name}' in the unit"));
                }
            }
        }
        if let Err(e) = crate::parameters::recompute_document_geometry(&mut scratch) {
            error = Some(e);
        }
        // Uncached meshing on purpose: the shared body-mesh memo is keyed by the *live*
        // document's fingerprint, and meshing a scratch document through it would evict
        // the importing document's own meshes every frame.
        let meshes = (0..scratch.bodies.len())
            .filter(|&bi| !scratch.bodies[bi].deleted)
            .filter_map(|bi| crate::extrude::body_solid_mesh_uncached_pub(&scratch, bi))
            .filter(|mesh| !mesh.is_empty())
            .collect();
        UnitEvaluation { meshes, document: scratch, error }
    }));
    result.unwrap_or_else(|_| UnitEvaluation {
        meshes: Vec::new(),
        document: Document::default(),
        error: Some("the unit's document failed to rebuild".to_string()),
    })
}

/// An instance's placement as a world transform (#722): rotation about its axis through
/// the unit origin, then translation. Placement expressions evaluate in the **importing**
/// document, so `height / 2` follows the importing document's parameters.
pub fn instance_transform(doc: &Document, instance: usize) -> glam::Mat4 {
    let Some(inst) = doc.unit_instances.get(instance) else {
        return glam::Mat4::IDENTITY;
    };
    let placement = &inst.placement;
    let length = |expr: &str| {
        let expr = expr.trim();
        if expr.is_empty() {
            0.0
        } else {
            crate::value::eval_length_mm_in_doc(expr, doc).unwrap_or(0.0)
        }
    };
    let translation = glam::vec3(
        length(&placement.tx),
        length(&placement.ty),
        length(&placement.tz),
    );
    let axis = glam::Vec3::from(placement.axis);
    let angle = {
        let expr = placement.angle.trim();
        if expr.is_empty() {
            0.0
        } else {
            crate::value::eval_angle_rad_in_doc(expr, doc).unwrap_or(0.0)
        }
    };
    let rotation = if axis.length_squared() > 1e-12 && angle != 0.0 {
        glam::Mat4::from_axis_angle(axis.normalize(), angle)
    } else {
        glam::Mat4::IDENTITY
    };
    let base = glam::Mat4::from_translation(translation) * rotation;

    // Move operations targeting this instance (#735) compose on top, like a moved
    // construction plane — the instance itself moves, no output bodies. Guarded against
    // re-entry: a Move's snap points resolve against body meshes, and a snap point on
    // this very instance would otherwise recurse — the guard makes it resolve against
    // the **unmoved** placement, which is exactly what a start point means.
    INSTANCE_TRANSFORM_GUARD.with(|guard| {
        if guard.borrow().contains(&instance) {
            return base;
        }
        guard.borrow_mut().push(instance);
        let mut transform = base;
        for op in doc.move_ops.iter().filter(|o| !o.deleted) {
            if !op.instance_targets.contains(&instance) {
                continue;
            }
            if let Some(m) = crate::extrude::move_op_transform(doc, op) {
                transform = m * transform;
            }
        }
        guard.borrow_mut().pop();
        transform
    })
}

thread_local! {
    /// Instances whose transform is being computed right now (#735) — breaks the
    /// snap-point → mesh → transform cycle; see [`instance_transform`].
    static INSTANCE_TRANSFORM_GUARD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Keep one live derived body per live unit instance (#724): a
/// [`crate::model::BodySource::UnitInstance`] body is what makes the unit's geometry
/// snappable and referenceable exactly like the document's own — Move's point pickers,
/// body-edge dimensions (#647), face pickers, and export all see an ordinary body. Runs
/// on the same every-mutation seam as document health; deleting an instance tombstones
/// its body here on the next pass.
pub fn sync_unit_bodies(doc: &mut Document) {
    use crate::model::BodySource;
    let body_for = |doc: &Document, instance: usize| {
        doc.bodies
            .iter()
            .position(|b| matches!(b.source, BodySource::UnitInstance(i) if i == instance))
    };
    for instance in 0..doc.unit_instances.len() {
        let alive = !doc.unit_instances[instance].deleted;
        match body_for(doc, instance) {
            Some(bi) => doc.bodies[bi].deleted = !alive,
            None if alive => {
                doc.bodies.push(crate::model::Body {
                    source: BodySource::UnitInstance(instance),
                    name: None,
                    deleted: false,
                    shadow: false,
                });
                doc.shape_order.push(crate::model::ShapeKind::Body);
            }
            None => {}
        }
        // Consumed units shadow (#726): a live cut result or a boolean/move/slice input
        // ghosts the intact unit body exactly like any consumed input — recomputed here
        // every pass, so deleting the consuming op un-shadows it again. The unit itself
        // is never mutated; shadowing is importing-document presentation state.
        if let Some(bi) = body_for(doc, instance) {
            let consumed_by_cut = doc.bodies.iter().any(|b| {
                !b.deleted
                    && matches!(b.source,
                        BodySource::UnitCut { instance: i, ref cut } if i == instance && !cut.is_empty())
            });
            doc.bodies[bi].shadow = consumed_by_cut
                || crate::model::body_shadowed_by_other_ops(doc, bi, None, None, None, None);
        }
    }
}

/// `instance`'s evaluated meshes placed by its transform, ready for the scene (#722).
/// Empty when the instance is deleted, dangling, or its unit failed to build.
pub fn placed_instance_meshes(doc: &Document, instance: usize) -> Vec<SolidMesh> {
    let Some(eval) = evaluate_instance(doc, instance) else {
        return Vec::new();
    };
    let transform = instance_transform(doc, instance);
    eval.meshes
        .iter()
        .map(|solid| SolidMesh {
            triangles: solid
                .triangles
                .iter()
                .map(|tri| tri.map(|v| transform.transform_point3(v)))
                .collect(),
        })
        .collect()
}

/// Every analytic face of a unit's rebuilt document (#724): the caps and flat sides of
/// its live extrusions — the faces [`crate::extrude::face_boundary_loop_world`] resolves.
pub fn inner_face_ids(inner: &Document) -> Vec<crate::model::FaceId> {
    use crate::model::{ExtrudeFace, FaceId};
    let mut faces = Vec::new();
    for (ei, extrusion) in inner.extrusions.iter().enumerate() {
        if extrusion.deleted {
            continue;
        }
        for profile in &extrusion.faces {
            for top in [false, true] {
                faces.push(FaceId::ExtrudeCap { extrusion: ei, profile: profile.clone(), top });
            }
            if let ExtrudeFace::Polygon(lines) = profile {
                for edge in 0..lines.len() as u8 {
                    faces.push(FaceId::ExtrudeSide {
                        extrusion: ei,
                        profile: profile.clone(),
                        edge,
                    });
                }
            }
        }
    }
    faces
}

/// Find the analytic identity of a unit body's feature edge (#724): the world segment
/// `a`–`b` mapped back into the unit and matched against its rebuilt document's face
/// boundary loops, returning `(face, edge ordinal)`. Analytic identities survive
/// override changes (the loop re-resolves after a rebuild), where the mesh's quantized
/// keys would go stale. `None` for geometry with no analytic face (e.g. a mesh import
/// inside the unit) — callers fall back to the quantized identity.
pub fn analytic_unit_edge(
    doc: &Document,
    instance: usize,
    a: glam::Vec3,
    b: glam::Vec3,
) -> Option<(crate::model::FaceId, usize)> {
    let eval = evaluate_instance(doc, instance)?;
    let inverse = instance_transform(doc, instance).inverse();
    let (a, b) = (inverse.transform_point3(a), inverse.transform_point3(b));
    const TOL: f32 = 0.05; // forgives the 0.01 mm pick quantization
    for face in inner_face_ids(&eval.document) {
        let Some(loop_world) = crate::extrude::face_boundary_loop_world(&eval.document, &face)
        else {
            continue;
        };
        let n = loop_world.len();
        for edge in 0..n {
            let (p, q) = (loop_world[edge], loop_world[(edge + 1) % n]);
            let matches = ((p - a).length() < TOL && (q - b).length() < TOL)
                || ((p - b).length() < TOL && (q - a).length() < TOL);
            if matches {
                return Some((face, edge));
            }
        }
    }
    None
}

/// The live world endpoints of an analytic unit edge (#724): the `(face, edge)` from
/// [`analytic_unit_edge`], resolved against the instance's current rebuild and placed by
/// its transform. `None` once the face or edge no longer exists.
pub fn unit_edge_world_segment(
    doc: &Document,
    instance: usize,
    face: &crate::model::FaceId,
    edge: usize,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let eval = evaluate_instance(doc, instance)?;
    let loop_world = crate::extrude::face_boundary_loop_world(&eval.document, face)?;
    let n = loop_world.len();
    if n < 2 || edge >= n {
        return None;
    }
    let transform = instance_transform(doc, instance);
    Some((
        transform.transform_point3(loop_world[edge]),
        transform.transform_point3(loop_world[(edge + 1) % n]),
    ))
}

/// The world-space boundary polygon of a unit's analytic face (#725): the inner face's
/// boundary loop, placed by the instance's transform. `None` once the instance, its
/// rebuild, or the face is gone.
pub fn unit_face_world_polygon(
    doc: &Document,
    instance: usize,
    face: &crate::model::FaceId,
) -> Option<Vec<glam::Vec3>> {
    let eval = evaluate_instance(doc, instance)?;
    let loop_world = crate::extrude::face_boundary_loop_world(&eval.document, face)?;
    let transform = instance_transform(doc, instance);
    Some(loop_world.into_iter().map(|p| transform.transform_point3(p)).collect())
}

/// Resolve where a unit's source file lives on this machine (#732): a relative source
/// against the importing document's own directory, a library source against the app's
/// library directory. `None` when the anchor it needs isn't known.
pub fn resolve_unit_source_path(
    source: &crate::model::UnitSource,
    own_path: Option<&str>,
    library: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    match source {
        crate::model::UnitSource::RelativePath(p) => {
            let own = std::path::Path::new(own_path?);
            Some(own.parent().unwrap_or_else(|| std::path::Path::new("")).join(p))
        }
        crate::model::UnitSource::Library(p) => Some(library?.join(p)),
    }
}

/// Whether a unit's embedded copy is behind its source file (#732): a cheap mtime check
/// first, the content hash as the authority (mtimes lie across copies and checkouts). A
/// missing or unresolvable source is **not** stale — the embedded copy is then simply
/// the truth.
pub fn unit_is_stale(
    unit: &ImportedUnit,
    own_path: Option<&str>,
    library: Option<&std::path::Path>,
) -> bool {
    let Some(path) = resolve_unit_source_path(&unit.source, own_path, library) else {
        return false;
    };
    let mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64);
    let Some(mtime) = mtime else {
        return false;
    };
    if unit.source_mtime == Some(mtime) {
        return false;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    unit.source_hash != Some(crate::model::content_hash(&bytes))
}

/// Debounced watcher over dynamic units' source files (#733): reports a unit ready to
/// re-sync only once its file has sat **quiet for a full poll** since the change was
/// first seen — so an editor's rapid rewrites collapse into one rebuild, and a writer
/// caught mid-save (before its temp-file rename lands) is never read half-written.
/// Static units are never watched. Nanosecond mtimes, so back-to-back saves within one
/// second still read as distinct.
#[derive(Default)]
pub struct UnitSourceWatcher {
    /// Last observed source mtime (nanos since epoch) per unit index.
    observed: std::collections::HashMap<usize, u128>,
}

impl UnitSourceWatcher {
    /// One poll pass: the dynamic units whose sources changed and have been quiet since
    /// the previous pass — ready to sync now.
    pub fn poll(
        &mut self,
        doc: &Document,
        own_path: Option<&str>,
        library: Option<&std::path::Path>,
    ) -> Vec<usize> {
        let mut ready = Vec::new();
        for (index, unit) in doc.units.iter().enumerate() {
            if unit.link != crate::model::LinkMode::Dynamic {
                continue;
            }
            let Some(path) = resolve_unit_source_path(&unit.source, own_path, library) else {
                continue;
            };
            let mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos());
            let Some(mtime) = mtime else {
                self.observed.remove(&index);
                continue;
            };
            match self.observed.insert(index, mtime) {
                // Still moving (or first sighting): wait for a quiet poll.
                Some(previous) if previous == mtime => {
                    if unit_is_stale(unit, own_path, library) {
                        ready.push(index);
                    }
                }
                _ => {}
            }
        }
        ready
    }
}

/// Where completed saves announce themselves (#733): a tiny stamp file in the app's
/// config directory that every BearCAD instance rewrites after a successful save. Other
/// instances stat it each tick (one syscall) and, on a change, check their dynamic units
/// right away — the "tell B directly" channel when A is open in BearCAD, without IPC.
#[cfg(not(target_arch = "wasm32"))]
fn save_ping_path() -> Option<std::path::PathBuf> {
    Some(crate::settings::settings_path()?.parent()?.join("save-ping"))
}

/// Announce a completed save to other BearCAD instances (#733). Best-effort.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_save_ping() {
    let Some(path) = save_ping_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let _ = std::fs::write(path, stamp.to_string());
}

/// The save-ping file's current mtime (nanos), `None` when absent (#733).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_ping_stamp() -> Option<u128> {
    let path = save_ping_path()?;
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImportedUnit, LinkMode, UnitInstance, UnitPlacement, UnitSource};

    /// A unit whose document extrudes a 10×10 square to a parametric `width` height, so
    /// an override visibly changes the built box.
    fn boxy_unit_doc() -> Document {
        let mut doc = Document::default();
        doc.parameters.push(crate::model::Parameter {
            name: "width".to_string(),
            expression: "10".to_string(),
            deleted: false,
            primary: false,
            source: None,
        });
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
            distance: 10.0,
            target: None,
            expression: "width".to_string(),
            symmetric: false,
            name: None,
            deleted: false,
            edge_treatments: Vec::new(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        doc
    }

    fn doc_with_unit_and_instances(overrides: Vec<Vec<(String, String)>>) -> Document {
        let mut doc = Document::default();
        doc.units.push(ImportedUnit {
            source: UnitSource::RelativePath("a.bearcad".to_string()),
            link: LinkMode::Static,
            document: boxy_unit_doc(),
            source_mtime: None,
            source_hash: None,
        });
        for parameter_overrides in overrides {
            doc.unit_instances.push(UnitInstance {
                unit: 0,
                name: None,
                parameter_overrides,
                placement: UnitPlacement::default(),
                deleted: false,
            });
        }
        doc
    }

    fn eval_count() -> usize {
        EVAL_COUNT.with(|c| c.get())
    }

    /// #722: two instances with identical overrides share a single evaluation, and the
    /// importing document's own edits don't re-evaluate anything.
    #[test]
    fn identical_instances_share_one_evaluation() {
        let mut doc = doc_with_unit_and_instances(vec![
            vec![("width".to_string(), "20".to_string())],
            vec![("width".to_string(), "20".to_string())],
        ]);
        let start = eval_count();
        let a = evaluate_instance(&doc, 0).expect("instance 0 evaluates");
        let b = evaluate_instance(&doc, 1).expect("instance 1 evaluates");
        assert_eq!(eval_count(), start + 1, "one uncached evaluation for both");
        assert!(Rc::ptr_eq(&a, &b), "both instances share the memoized result");
        assert!(a.error.is_none(), "error: {:?}", a.error);

        // An edit to the importing document's own content leaves the memo warm.
        doc.parameters.push(crate::model::Parameter {
            name: "own".to_string(),
            expression: "1".to_string(),
            deleted: false,
            primary: false,
            source: None,
        });
        let _ = evaluate_instance(&doc, 0).unwrap();
        assert_eq!(eval_count(), start + 1, "B's unrelated edit re-evaluates nothing");
    }

    /// #722: changing one instance's override re-evaluates that instance only.
    #[test]
    fn changing_one_override_leaves_the_others_cached() {
        let mut doc = doc_with_unit_and_instances(vec![
            vec![("width".to_string(), "20".to_string())],
            vec![("width".to_string(), "30".to_string())],
        ]);
        let a = evaluate_instance(&doc, 0).unwrap();
        let b = evaluate_instance(&doc, 1).unwrap();
        assert!(!Rc::ptr_eq(&a, &b), "different overrides evaluate separately");

        doc.unit_instances[1].parameter_overrides[0].1 = "40".to_string();
        let start = eval_count();
        let a_again = evaluate_instance(&doc, 0).unwrap();
        let _b_again = evaluate_instance(&doc, 1).unwrap();
        assert!(Rc::ptr_eq(&a, &a_again), "the untouched instance stays cached");
        assert_eq!(eval_count(), start + 1, "only the edited instance re-evaluates");
    }

    /// #722: overrides actually drive the rebuilt geometry.
    #[test]
    fn overrides_change_the_evaluated_geometry() {
        let doc = doc_with_unit_and_instances(vec![
            Vec::new(),
            vec![("width".to_string(), "20".to_string())],
        ]);
        let base = evaluate_instance(&doc, 0).unwrap();
        let wide = evaluate_instance(&doc, 1).unwrap();
        let height = |eval: &UnitEvaluation| {
            let (min, max) = eval.meshes[0].bounds().unwrap();
            max.z - min.z
        };
        assert_eq!(base.meshes.len(), 1, "error: {:?}", base.error);
        assert!((height(&base) - 10.0).abs() < 1e-3, "default width 10");
        assert!((height(&wide) - 20.0).abs() < 1e-3, "overridden width 20");
    }

    /// #722: a unit that fails to rebuild reports the failure instead of panicking, and
    /// the rest of the document stays usable.
    #[test]
    fn a_broken_unit_reports_unhealthy_rather_than_panicking() {
        let mut doc = doc_with_unit_and_instances(vec![vec![(
            "nope".to_string(),
            "5".to_string(),
        )]]);
        let eval = evaluate_instance(&doc, 0).expect("still evaluates");
        assert!(eval.error.is_some(), "the bad override is reported");

        // Document health carries the reason for the instance (#722).
        let health = crate::document_health::recompute_document_health(&doc);
        assert!(
            health.unit_instances.get(&0).is_some(),
            "instance 0 reports unhealthy"
        );

        // A healthy instance beside it still evaluates cleanly.
        doc.unit_instances.push(crate::model::UnitInstance {
            unit: 0,
            name: None,
            parameter_overrides: Vec::new(),
            placement: UnitPlacement::default(),
            deleted: false,
        });
        let ok = evaluate_instance(&doc, 1).unwrap();
        assert!(ok.error.is_none() && !ok.meshes.is_empty());
    }

    /// #722: placement expressions evaluate in the importing document and move the unit.
    #[test]
    fn instance_transform_follows_the_importing_documents_parameters() {
        let mut doc = doc_with_unit_and_instances(vec![Vec::new()]);
        doc.parameters.push(crate::model::Parameter {
            name: "gap".to_string(),
            expression: "7".to_string(),
            deleted: false,
            primary: false,
            source: None,
        });
        doc.unit_instances[0].placement = crate::model::UnitPlacement {
            tx: "gap * 2".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: [0.0, 0.0, 1.0],
            angle: "90".to_string(),
        };
        let transform = instance_transform(&doc, 0);
        let moved = transform.transform_point3(glam::vec3(1.0, 0.0, 0.0));
        assert!((moved - glam::vec3(14.0, 1.0, 0.0)).length() < 1e-4, "{moved:?}");
    }
}
