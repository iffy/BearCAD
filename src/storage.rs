//! `.bearcad` file persistence (SPEC §7).
//!
//! Native files are SQLite: one table per arena, typed columns, `blobs` for
//! binary data. The web build uses the JSON codec (`to_json_bytes` /
//! `from_json_bytes`); native `open` sniffs the 16-byte SQLite header.


use crate::face::default_xy_plane;
use crate::model::Document;

pub type Result<T> = std::result::Result<T, String>;

/// The JSON document format: the whole [`Document`] serde-serialized. This is what the
/// **web build** saves and loads (the browser has no SQLite); the native `open` sniffs
/// file magic and accepts either format, so web-saved files open everywhere.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn to_json_bytes(doc: &Document) -> Result<Vec<u8>> {
    serde_json::to_vec(doc).map_err(|e| e.to_string())
}

/// Parse a JSON document (see [`to_json_bytes`]) and run the shared post-load fixups.
pub fn from_json_bytes(bytes: &[u8]) -> Result<Document> {
    let mut doc: Document = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    fixup_loaded_document(&mut doc)?;
    Ok(doc)
}

/// Post-load normalization shared by every load path (SQLite, legacy, JSON).
pub(crate) fn fixup_loaded_document(doc: &mut Document) -> Result<()> {
    // Depth cap + structural cycle check on imported units (#719). Native `open` re-runs
    // this with the file's real path, which also catches cycles across relative sources.
    crate::model::validate_units(doc, None)?;
    crate::units::sync_unit_bodies(doc);
    ensure_a_ground_plane(doc);
    crate::constraints::migrate_legacy_dimensions(doc);
    migrate_text_pins(doc);
    crate::constraints::solve_document_constraints(doc).map_err(|e| e.to_string())?;
    Ok(())
}

/// Convert legacy text position pins (#356) into `Coincident` constraints between the text's
/// anchor point and the pin target (#408), so old documents keep their behaviour under the
/// constraint solver. The pin field is cleared and never written back.
fn migrate_text_pins(doc: &mut Document) {
    for i in doc.sketch_texts.keys().collect::<Vec<_>>() {
        let Some((point, anchor)) = doc.sketch_texts[i].pin.take() else {
            continue;
        };
        doc.constraints.insert(crate::model::Constraint {
            sketch: doc.sketch_texts[i].sketch,
            kind: crate::model::ConstraintKind::Coincident {
                a: crate::model::ConstraintEntity::Point(
                    crate::model::ConstraintPoint::TextAnchor { text: i, anchor },
                ),
                b: crate::model::ConstraintEntity::Point(point),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        doc.shape_order.push(crate::model::ShapeKind::Constraint);
    }
}


/// Every document has at least the XY ground plane. This used to also pad the list until
/// every sketch's plane *index* existed; with keys there is nothing to pad — a sketch whose
/// host key does not resolve is an unhealthy sketch, and document health says so (#1055).
fn ensure_a_ground_plane(doc: &mut Document) {
    if doc.construction_planes.is_empty() {
        doc.construction_planes.insert(default_xy_plane());
    }
}

/// The SQLite `.bearcad` format — native builds only (the bundled SQLite C library
/// doesn't compile for wasm32-unknown-unknown).
#[cfg(not(target_arch = "wasm32"))]
mod sqlite_format {
include!("storage_sqlite.rs");

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::sketch_text_key_for_slot as tkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::unit_key_for_slot as ukey;
    use crate::model::component_key_for_slot as ckey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::slice_op_key_for_slot as slckey;
    use crate::model::boolean_op_key_for_slot as bopkey;
    use super::*;
    use crate::model::{Circle, FaceId};

    fn plane_sketch(doc: &mut Document) -> crate::model::SketchId {
        doc.add_sketch(FaceId::ConstructionPlane(pkey(0)))
    }

    /// #408: a legacy text pin loads as a `Coincident` constraint on the text's anchor point,
    /// and the pin field is cleared (never written back).
    #[test]
    fn legacy_text_pin_migrates_to_coincident_constraint() {
        let mut doc = Document::default();
        let sketch = plane_sketch(&mut doc);
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 30.0, 40.0, 60.0, 40.0));
        doc.sketch_texts.insert(crate::model::SketchText {
            sketch,
            text: "Hi".to_string(),
            font_family: String::new(),
            bold: false,
            italic: false,
            underline: false,
            size: 10.0,
            size_expr: "10".to_string(),
            origin: (0.0, 0.0),
            rotation: 0.0,
            wrap_width: None,
            baseline_line: None,
            contours: vec![vec![(0.0, 0.0), (4.0, 0.0), (4.0, 6.0), (0.0, 6.0)]],
            font_bytes: Vec::new(),
            pin: Some((
                crate::model::ConstraintPoint::LineEndpoint {
                    line: lkey(0),
                    end: crate::model::LineEnd::Start,
                },
                crate::model::TextAnchor::Center,
            )),
            name: None,
        });
        doc.shape_order.push(crate::model::ShapeKind::SketchText);
        crate::storage::fixup_loaded_document(&mut doc).expect("fixup");
        assert!(doc.sketch_texts[tkey(0)].pin.is_none(), "the pin is cleared");
        let migrated = doc.constraints.values().any(|c| {
            matches!(
                &c.kind,
                crate::model::ConstraintKind::Coincident {
                    a: crate::model::ConstraintEntity::Point(
                        crate::model::ConstraintPoint::TextAnchor {
                            text,
                            anchor: crate::model::TextAnchor::Center,
                        }
                    ),
                    ..
                } if *text == tkey(0)
            )
        });
        assert!(migrated, "a coincident constraint replaces the pin");
        // The solve ran as part of load: the centre anchor sits on the line start.
        let (cx, cy) = crate::text::sketch_text_anchor_uv(
            &doc.sketch_texts[tkey(0)],
            crate::model::TextAnchor::Center,
        );
        assert!((cx - 30.0).abs() < 1e-2 && (cy - 40.0).abs() < 1e-2, "centre at ({cx}, {cy})");
    }

    fn assert_world_anchors_match(before: &[glam::Vec3], after: &[glam::Vec3]) {
        assert_eq!(
            before.len(),
            after.len(),
            "element world anchor count should match after reload"
        );
        for (a, b) in before.iter().zip(after) {
            assert!(
                (*a - *b).length() < 1e-3,
                "world anchor {:?} should round-trip as {:?}",
                a,
                b
            );
        }
    }

    fn element_world_anchors(doc: &Document) -> Vec<glam::Vec3> {
        let mut anchors = Vec::new();
        for plane in doc.construction_planes.values() {
            anchors.push(plane.origin);
        }
        for circle in doc.circles.values() {
            anchors.push(crate::face::circle_world_center(doc, circle).unwrap());
        }
        for line in doc.lines.values() {
            let (a, b) = crate::face::line_world_endpoints(doc, line).unwrap();
            anchors.push(a);
            anchors.push(b);
        }
        anchors
    }

    #[test]
    fn round_trips_shapes_and_shape_order() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = plane_sketch(&mut doc);
        crate::construction::add_line_rectangle(&mut doc, sketch, 1.0, 2.0, 4.0, 6.0, [false; 4]);
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 1.0, 1.0, 1.0, 6.0));
        doc.shape_order.push(ShapeKind::Line);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.lines, doc.lines);
        assert_eq!(loaded.constraints, doc.constraints);
        assert_eq!(loaded.shape_order, doc.shape_order);

        std::fs::remove_file(&path).unwrap();
    }

    /// #423: components and their membership survive a save/load round trip.
    #[test]
    fn round_trips_components_and_membership() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_component_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.components.insert(crate::model::Component {
            name: Some("Frame".to_string()),
            parent: None,
            length_unit: Some(crate::value::LengthUnit::In),
            angle_unit: None,
        });
        doc.components.insert(crate::model::Component {
            name: None,
            parent: Some(ckey(0)),
            length_unit: None,
            angle_unit: None,
        });
        doc.set_component_member(crate::model::ComponentMember::ConstructionPlane(pkey(0)), Some(ckey(1)));

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.components, doc.components);
        assert_eq!(loaded.component_members, doc.component_members);

        std::fs::remove_file(&path).unwrap();
    }

    /// A `.json` path saves the web JSON codec, and `open` sniffs and loads it — the
    /// #1055: an arena-backed collection survives both file formats with its keys intact —
    /// a reload must not renumber elements, or every reference stored elsewhere in the
    /// document would point at the wrong one.
    #[test]
    fn loft_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let first = doc.lofts.insert(crate::model::Loft {
            sections: Vec::new(),
            mode: crate::model::LoftMode::NewBody,
            name: Some("first".to_string()),
        });
        let doomed = doc.lofts.insert(crate::model::Loft {
            sections: Vec::new(),
            mode: crate::model::LoftMode::NewBody,
            name: Some("doomed".to_string()),
        });
        let last = doc.lofts.insert(crate::model::Loft {
            sections: Vec::new(),
            mode: crate::model::LoftMode::NewBody,
            name: Some("last".to_string()),
        });
        // Removed for real — the tombstone this replaces would have left it in the file.
        assert!(doc.lofts.remove(doomed).is_some());
        assert_eq!(doc.lofts.len(), 2);

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_loft_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.lofts.len(), 2, "{suffix}: the removed loft stayed gone");
            assert_eq!(
                loaded.lofts.get(first).and_then(|l| l.name.as_deref()),
                Some("first"),
                "{suffix}: the first key still resolves to its own loft"
            );
            assert_eq!(
                loaded.lofts.get(last).and_then(|l| l.name.as_deref()),
                Some("last"),
                "{suffix}: and so does the one that used to shift when its neighbour went"
            );
            assert!(
                loaded.lofts.get(doomed).is_none(),
                "{suffix}: a key to a removed loft does not come back to life"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: a boolean operation keeps its key across a save, and so do the output bodies
    /// that name it through `BodySource::Boolean`.
    #[test]
    fn boolean_op_keys_survive_a_save_and_reload() {
        let op = |kind| crate::model::BooleanOperation {
            kind,
            a: Vec::new(),
            b: Vec::new(),
            keep_b: false,
            outputs: Vec::new(),
            name: None,
        };
        let mut doc = Document::default();
        let doomed = doc.boolean_ops.insert(op(crate::model::BooleanOpKind::Combine));
        let kept = doc.boolean_ops.insert(op(crate::model::BooleanOpKind::Cut));
        assert!(doc.boolean_ops.remove(doomed).is_some());
        let out = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Boolean {
                op: kept,
                solid: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.boolean_ops[kept].outputs = vec![out];

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_boolean_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.boolean_ops.len(), 1, "{suffix}");
            assert_eq!(
                loaded.boolean_ops.get(kept).map(|o| o.kind),
                Some(crate::model::BooleanOpKind::Cut),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies[out].source,
                crate::model::BodySource::Boolean {
                    op: kept,
                    solid: 0,
                    add: Vec::new(),
                    cut: Vec::new(),
                },
                "{suffix}: its output body still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: construction planes keep their keys across a save, and so does everything that
    /// names one — a sketch's host face, a tracing image's plane, a plane's own parent chain.
    #[test]
    fn construction_plane_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let doomed = doc.construction_planes.insert(crate::face::default_xy_plane());
        let kept = doc.construction_planes.insert(crate::model::ConstructionPlane {
            origin: glam::Vec3::new(0.0, 0.0, 17.0),
            ..crate::face::default_xy_plane()
        });
        assert!(doc.construction_planes.remove(doomed).is_some());
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(kept));
        let image = doc.tracing_images.insert(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: kept,
            origin: (0.0, 0.0),
            width_mm: 10.0,
            height_mm: 10.0,
            name: None,
            base_origin: None,
            calibration: None,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_plane_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(
                loaded.construction_planes.get(kept).map(|p| p.origin.z),
                Some(17.0),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert!(loaded.construction_planes.get(doomed).is_none(), "{suffix}");
            assert_eq!(
                loaded.sketches.get(sketch).map(|s| s.face.clone()),
                Some(crate::model::FaceId::ConstructionPlane(kept)),
                "{suffix}: the sketch still sits on its host plane"
            );
            assert_eq!(
                loaded.tracing_images.get(image).map(|i| i.plane),
                Some(kept),
                "{suffix}: the tracing image still names its plane"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: circles keep their keys across a save, and so does everything that names one
    /// — a diameter constraint, an extruded profile face, a sketch hosted on the circle.
    #[test]
    fn circle_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let circle = |r: f32| crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, r, 0.0);
        let doomed = doc.circles.insert(circle(1.0));
        let kept = doc.circles.insert(circle(7.0));
        assert!(doc.circles.remove(doomed).is_some());
        doc.constraints.insert(crate::model::Constraint {
            sketch,
            kind: crate::model::ConstraintKind::Distance {
                target: crate::model::DistanceTarget::CircleDiameter(kept),
            },
            expression: "14".to_string(),
            dim_offset: None,
            name: None,
        });
        let extrusion = doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Circle(kept)],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_circle_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.circles.len(), 1, "{suffix}");
            assert_eq!(
                loaded.circles.get(kept).map(|c| c.r),
                Some(7.0),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert!(
                loaded.constraints.values().any(|c| matches!(
                    &c.kind,
                    crate::model::ConstraintKind::Distance {
                        target: crate::model::DistanceTarget::CircleDiameter(i)
                    } if *i == kept
                )),
                "{suffix}: the diameter dimension still names it"
            );
            assert_eq!(
                loaded.extrusions.get(extrusion).map(|e| e.faces.clone()),
                Some(vec![crate::model::ExtrudeFace::Circle(kept)]),
                "{suffix}: the extruded profile still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1074: a Face Snap mate's point on a face — its face key and its offset across that
    /// face — survives a save. The offset is the new part; a file that dropped it would
    /// silently move every such mate to the middle of its face.
    #[test]
    fn a_point_on_a_face_survives_a_save_and_reload() {
        let mut doc = Document::default();
        let body = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let point = crate::model::MovePointRef::OnFace {
            body,
            centroid: [0, 0, 500],
            normal: [0, 0, 100],
            uv: [325, -750],
        };
        let op = doc.move_ops.insert(crate::model::MoveOperation {
            keep_inputs: false,
            targets: vec![body],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            outputs: Vec::new(),
            translate_mode: crate::model::MoveTranslateMode::PointSnap,
            start_point_a: Some(point),
            end_point_a: Some(crate::model::MovePointRef::Origin),
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            roll_angle: "30 deg".to_string(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            name: None,
            face_flip: true,
            face_spin: "45 deg".to_string(),
            face_offset: String::new(),
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_on_face_point_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();
            assert_eq!(
                loaded.move_ops.get(op).and_then(|o| o.start_point_a),
                Some(point),
                "{suffix}: the face key and the offset across it both came back"
            );
            // #1077/#1078: the mate's side, its turn, and a third pair set as an angle.
            assert_eq!(
                loaded.move_ops.get(op).map(|o| (o.face_flip, o.face_spin.clone(), o.roll_angle.clone())),
                Some((true, "45 deg".to_string(), "30 deg".to_string())),
                "{suffix}"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: lines keep their keys across a save, and so does everything that names one — a
    /// constraint on its endpoint, a length dimension, an extruded polygon profile, and the
    /// bridging line that records it as its chamfer parent.
    #[test]
    fn line_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let line = |y: f32| crate::model::Line::from_local_endpoints(sketch, 0.0, y, 10.0, y);
        let doomed = doc.lines.insert(line(1.0));
        let kept = doc.lines.insert(line(7.0));
        assert!(doc.lines.remove(doomed).is_some());
        let mut bridge = line(9.0);
        bridge.chamfer_fillet_parent = Some(kept);
        let bridge = doc.lines.insert(bridge);
        doc.constraints.insert(crate::model::Constraint {
            sketch,
            kind: crate::model::ConstraintKind::Distance {
                target: crate::model::DistanceTarget::LineLength(kept),
            },
            expression: "10".to_string(),
            dim_offset: None,
            name: None,
        });
        let extrusion = doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(vec![kept, bridge])],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_line_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.lines.len(), 2, "{suffix}");
            assert_eq!(
                loaded.lines.get(kept).map(|l| l.y0),
                Some(7.0),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.lines.get(bridge).and_then(|l| l.chamfer_fillet_parent),
                Some(kept),
                "{suffix}: the bridge still names its chamfer parent"
            );
            assert!(
                loaded.constraints.values().any(|c| matches!(
                    &c.kind,
                    crate::model::ConstraintKind::Distance {
                        target: crate::model::DistanceTarget::LineLength(i)
                    } if *i == kept
                )),
                "{suffix}: the length dimension still names it"
            );
            assert_eq!(
                loaded.extrusions.get(extrusion).map(|e| e.faces.clone()),
                Some(vec![crate::model::ExtrudeFace::Polygon(vec![kept, bridge])]),
                "{suffix}: the extruded profile still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: sketches keep their keys across a save, and so does everything that names one
    /// — a line, a circle, a constraint, an extrusion.
    #[test]
    fn sketch_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let doomed = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let kept = doc.add_sketch(crate::model::FaceId::Circle(rkey(3)));
        assert!(doc.sketches.remove(doomed).is_some());
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(kept, 0.0, 0.0, 10.0, 0.0));
        let extrusion = doc.extrusions.insert(crate::model::Extrusion {
            sketch: kept,
            faces: Vec::new(),
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_sketch_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.sketches.len(), 1, "{suffix}");
            assert_eq!(
                loaded.sketches.get(kept).map(|s| s.face.clone()),
                Some(crate::model::FaceId::Circle(rkey(3))),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(loaded.lines[lkey(0)].sketch, kept, "{suffix}: its line still names it");
            assert_eq!(
                loaded.extrusions.get(extrusion).map(|e| e.sketch),
                Some(kept),
                "{suffix}: the extrusion still names its host sketch"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: constraints keep their keys across a save, and so do the in-sketch operations
    /// that name the stitch coincidences they generated.
    #[test]
    fn constraint_keys_survive_a_save_and_reload() {
        let coincident = |line: usize| crate::model::Constraint {
            sketch: skey(0),
            kind: crate::model::ConstraintKind::Coincident {
                a: crate::model::ConstraintEntity::Line(crate::model::ConstraintLine::Line(lkey(
                    line,
                ))),
                b: crate::model::ConstraintEntity::Line(crate::model::ConstraintLine::Line(lkey(
                    line + 1,
                ))),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        };
        let mut doc = Document::default();
        let doomed = doc.constraints.insert(coincident(0));
        let kept = doc.constraints.insert(coincident(4));
        assert!(doc.constraints.remove(doomed).is_some());
        let op = doc.sketch_slice_ops.insert(crate::model::SketchSliceOperation {
            sketch: skey(0),
            line_targets: Vec::new(),
            cutter_lines: Vec::new(),
            circle_targets: Vec::new(),
            face_targets: Vec::new(),
            line_outputs: Vec::new(),
            constraint_outputs: vec![kept],
            name: None,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_constraint_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.constraints.len(), 1, "{suffix}");
            assert_eq!(
                loaded.constraints.get(kept).map(|c| c.kind.clone()),
                Some(coincident(4).kind),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.sketch_slice_ops.get(op).map(|o| o.constraint_outputs.clone()),
                Some(vec![kept]),
                "{suffix}: the op still names the constraint it generated"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: sketch texts keep their keys across a save, and so do the anchor constraints
    /// and glyph faces that name one.
    #[test]
    fn sketch_text_keys_survive_a_save_and_reload() {
        let text = |content: &str| crate::model::SketchText {
            sketch: skey(0),
            text: content.to_string(),
            font_family: "Helvetica".to_string(),
            bold: false,
            italic: false,
            underline: false,
            size: 10.0,
            size_expr: String::new(),
            origin: (0.0, 0.0),
            rotation: 0.0,
            wrap_width: None,
            baseline_line: None,
            contours: Vec::new(),
            font_bytes: Vec::new(),
            pin: None,
            name: None,
        };
        let mut doc = Document::default();
        let doomed = doc.sketch_texts.insert(text("doomed"));
        let kept = doc.sketch_texts.insert(text("kept"));
        assert!(doc.sketch_texts.remove(doomed).is_some());
        doc.constraints.insert(crate::model::Constraint {
            sketch: skey(0),
            kind: crate::model::ConstraintKind::Coincident {
                a: crate::model::ConstraintEntity::Point(
                    crate::model::ConstraintPoint::TextAnchor {
                        text: kept,
                        anchor: crate::model::TextAnchor::Center,
                    },
                ),
                b: crate::model::ConstraintEntity::Point(
                    crate::model::ConstraintPoint::CircleCenter(rkey(0)),
                ),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_text_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.sketch_texts.len(), 1, "{suffix}");
            assert_eq!(
                loaded.sketch_texts.get(kept).map(|t| t.text.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert!(
                loaded.constraints.values().any(|c| matches!(
                    &c.kind,
                    crate::model::ConstraintKind::Coincident {
                        a: crate::model::ConstraintEntity::Point(
                            crate::model::ConstraintPoint::TextAnchor { text, .. }
                        ),
                        ..
                    } if *text == kept
                )),
                "{suffix}: the anchor constraint still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: extrusions keep their keys across a save, and so does everything that names
    /// one — a body's add/cut lists, a sketch's host cap/side face, an edge treatment.
    #[test]
    fn extrusion_keys_survive_a_save_and_reload() {
        let extrusion = |name: &str| crate::model::Extrusion {
            sketch: skey(0),
            faces: vec![crate::model::ExtrudeFace::Circle(rkey(0))],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: Some(name.to_string()),
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        };
        let mut doc = Document::default();
        let doomed = doc.extrusions.insert(extrusion("doomed"));
        let kept = doc.extrusions.insert(extrusion("kept"));
        let cut = doc.extrusions.insert(extrusion("cut"));
        assert!(doc.extrusions.remove(doomed).is_some());
        let body = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![kept], cut: vec![cut] },
            material: None,
            name: None,
            shadow: false,
        });
        doc.sketches.insert(crate::model::Sketch {
            face: crate::model::FaceId::ExtrudeCap {
                extrusion: kept,
                profile: crate::model::ExtrudeFace::Circle(rkey(0)),
                top: true,
            },
            name: None,
            length_unit: None,
            angle_unit: None,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_extrusion_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.extrusions.len(), 2, "{suffix}");
            assert_eq!(
                loaded.extrusions.get(kept).and_then(|e| e.name.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies[body].source,
                crate::model::BodySource::Solid { base: None, add: vec![kept], cut: vec![cut] },
                "{suffix}: the body still names both"
            );
            assert_eq!(
                loaded.sketches[skey(0)].face,
                crate::model::FaceId::ExtrudeCap {
                    extrusion: kept,
                    profile: crate::model::ExtrudeFace::Circle(rkey(0)),
                    top: true,
                },
                "{suffix}: the sketch still sits on its host cap"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: drawings keep their keys across a save, and so does a membership entry that
    /// names one.
    #[test]
    fn drawing_keys_survive_a_save_and_reload() {
        let drawing = |name: &str| crate::model::Drawing {
            name: Some(name.to_string()),
            ..Default::default()
        };
        let mut doc = Document::default();
        let doomed = doc.drawings.insert(drawing("doomed"));
        let kept = doc.drawings.insert(drawing("kept"));
        assert!(doc.drawings.remove(doomed).is_some());
        let component = doc.components.insert(crate::model::Component {
            name: Some("Sheets".to_string()),
            parent: None,
            length_unit: None,
            angle_unit: None,
        });
        doc.set_component_member(crate::model::ComponentMember::Drawing(kept), Some(component));

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_drawing_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.drawings.len(), 1, "{suffix}");
            assert_eq!(
                loaded.drawings.get(kept).and_then(|d| d.name.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.component_of(crate::model::ComponentMember::Drawing(kept)),
                Some(component),
                "{suffix}: the membership entry still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: a page's text annotations keep their keys across a save — a note deleted from
    /// the middle does not renumber the ones after it.
    #[test]
    fn annotation_keys_survive_a_save_and_reload() {
        let annotation = |text: &str| crate::model::DrawingAnnotation {
            text: text.to_string(),
            pos_x: 0.1,
            pos_y: 0.1,
            size_frac: 0.028,
            wrap_frac: None,
        };
        let mut page = crate::model::Drawing::default();
        let doomed = page.annotations.insert(annotation("doomed"));
        let kept = page.annotations.insert(annotation("kept"));
        assert!(page.annotations.remove(doomed).is_some());
        let mut doc = Document::default();
        let drawing = doc.drawings.insert(page);

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_annotation_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            let page = loaded.drawings.get(drawing).expect("the page");
            assert_eq!(page.annotations.len(), 1, "{suffix}");
            assert_eq!(
                page.annotations.get(kept).map(|a| a.text.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert!(page.annotations.get(doomed).is_none(), "{suffix}");
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: components keep their keys across a save, and so do the membership entries
    /// and parent links that name one.
    #[test]
    fn component_keys_survive_a_save_and_reload() {
        let component = |name: &str| crate::model::Component {
            name: Some(name.to_string()),
            parent: None,
            length_unit: None,
            angle_unit: None,
        };
        let mut doc = Document::default();
        let doomed = doc.components.insert(component("doomed"));
        let kept = doc.components.insert(component("kept"));
        assert!(doc.components.remove(doomed).is_some());
        let nested = doc.components.insert(crate::model::Component {
            parent: Some(kept),
            ..component("nested")
        });
        doc.set_component_member(crate::model::ComponentMember::ConstructionPlane(pkey(0)), Some(kept));

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_component_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.components.len(), 2, "{suffix}");
            assert_eq!(
                loaded.components.get(kept).and_then(|c| c.name.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.components.get(nested).and_then(|c| c.parent),
                Some(kept),
                "{suffix}: the child still names its parent"
            );
            assert_eq!(
                loaded.component_of(crate::model::ComponentMember::ConstructionPlane(pkey(0))),
                Some(kept),
                "{suffix}: the membership entry still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: embedded units keep their keys across a save, and so do the instances that
    /// place them. A refused import removes the trial copy, so the collection does have a
    /// removal path even though ordinary deletion leaves the copy behind.
    #[test]
    fn unit_keys_survive_a_save_and_reload() {
        let unit = |name: &str| crate::model::ImportedUnit {
            source: crate::model::UnitSource::RelativePath(format!("{name}.bearcad")),
            link: crate::model::LinkMode::Dynamic,
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        };
        let mut doc = Document::default();
        let doomed = doc.units.insert(unit("doomed"));
        let kept = doc.units.insert(unit("kept"));
        assert!(doc.units.remove(doomed).is_some());
        let instance = doc.unit_instances.insert(crate::model::UnitInstance {
            unit: kept,
            name: Some("a".to_string()),
            parameter_overrides: Vec::new(),
            placement: Default::default(),
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_unit_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.units.len(), 1, "{suffix}");
            assert_eq!(
                loaded.units.get(kept).map(|u| u.source.clone()),
                Some(crate::model::UnitSource::RelativePath("kept.bearcad".to_string())),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.unit_instances.get(instance).map(|i| i.unit),
                Some(kept),
                "{suffix}: its placement still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: unit instances keep their keys across a save, and so does everything that
    /// names one — the materialized body's source and a move op's instance targets.
    #[test]
    fn unit_instance_keys_survive_a_save_and_reload() {
        let instance = |name: &str| crate::model::UnitInstance {
            unit: ukey(0),
            name: Some(name.to_string()),
            parameter_overrides: Vec::new(),
            placement: Default::default(),
        };
        let mut doc = Document::default();
        doc.units.insert(crate::model::ImportedUnit {
            source: crate::model::UnitSource::RelativePath("bracket.bearcad".to_string()),
            link: crate::model::LinkMode::Dynamic,
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        });
        let doomed = doc.unit_instances.insert(instance("doomed"));
        let kept = doc.unit_instances.insert(instance("kept"));
        assert!(doc.unit_instances.remove(doomed).is_some());
        let body = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::UnitInstance(kept),
            material: None,
            name: None,
            shadow: false,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_instance_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.unit_instances.len(), 1, "{suffix}");
            assert_eq!(
                loaded.unit_instances.get(kept).and_then(|i| i.name.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies[body].source,
                crate::model::BodySource::UnitInstance(kept),
                "{suffix}: its materialized body still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: joints keep their keys across a save, so a deleted joint leaves a hole rather
    /// than shifting the survivor down into its place.
    #[test]
    fn joint_keys_survive_a_save_and_reload() {
        let mut doc = Document::default();
        let joint = |name: &str| crate::model::Joint {
            members: Vec::new(),
            base: 0,
            kind: crate::model::JointKind::Revolute,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: Default::default(),
            name: Some(name.to_string()),
            // #1079: the joint's own frame — the axis its freedoms run along, which is no
            // longer derivable from the mate and so has to survive the file.
            frame: crate::model::JointFrame {
                origin: Some(crate::model::MovePointRef::Origin),
                primary: Some(crate::model::MateRef::Axis(crate::construction::GlobalAxis::X)),
                secondary: None,
            },
        };
        let doomed = doc.joints.insert(joint("doomed"));
        let kept = doc.joints.insert(joint("kept"));
        assert!(doc.joints.remove(doomed).is_some());

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_joint_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.joints.len(), 1, "{suffix}");
            assert_eq!(
                loaded.joints.get(kept).map(|j| j.frame.clone()),
                Some(joint("kept").frame),
                "{suffix}: the joint's frame came back"
            );
            assert_eq!(
                loaded.joints.get(kept).and_then(|j| j.name.clone()),
                Some("kept".to_string()),
                "{suffix}: the survivor did not shift into the hole"
            );
            assert!(loaded.joints.get(doomed).is_none(), "{suffix}");
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: bodies keep their keys across a save, and so does everything that names one —
    /// an operation's inputs and outputs, a joint's members, a drawing view's source. This is
    /// the collection with the widest blast radius, so the test carries one of each.
    #[test]
    fn body_keys_survive_a_save_and_reload() {
        let body = |name: &str| crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            name: Some(name.to_string()),
            material: None,
            shadow: false,
        };
        let mut doc = Document::default();
        let doomed = doc.bodies.insert(body("doomed"));
        let input = doc.bodies.insert(body("input"));
        let output = doc.bodies.insert(body("output"));
        assert!(doc.bodies.remove(doomed).is_some());
        doc.move_ops.insert(crate::model::MoveOperation {
            keep_inputs: false,
            targets: vec![input],
            outputs: vec![output],
            translate_mode: Default::default(),
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: "5mm".to_string(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        });
        doc.joints.insert(crate::model::Joint {
            members: vec![
                crate::model::JointRef::Body(input),
                crate::model::JointRef::Body(output),
            ],
            base: 0,
            kind: crate::model::JointKind::Rigid,
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

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_body_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.bodies.len(), 2, "{suffix}");
            assert_eq!(
                loaded.bodies.get(input).and_then(|b| b.name.clone()),
                Some("input".to_string()),
                "{suffix}: the surviving bodies did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies.get(output).and_then(|b| b.name.clone()),
                Some("output".to_string()),
                "{suffix}"
            );
            assert!(loaded.bodies.get(doomed).is_none(), "{suffix}: removed stays removed");
            assert_eq!(loaded.move_ops.values().nth(0).unwrap().targets, vec![input], "{suffix}: op input");
            assert_eq!(loaded.move_ops.values().nth(0).unwrap().outputs, vec![output], "{suffix}: op output");
            assert_eq!(
                loaded.joints.values().nth(0).unwrap().members,
                vec![
                    crate::model::JointRef::Body(input),
                    crate::model::JointRef::Body(output),
                ],
                "{suffix}: joint members"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: a sweep keeps its key across a save, and the body it produced keeps pointing
    /// at it.
    #[test]
    fn sweep_keys_survive_a_save_and_reload() {
        let sweep = |path: Vec<usize>| crate::model::Sweep {
            sketch: skey(0),
            faces: Vec::new(),
            path: path.into_iter().map(lkey).collect(),
            mode: crate::model::SweepMode::NewBody,
            name: None,
        };
        let mut doc = Document::default();
        let doomed = doc.sweeps.insert(sweep(vec![0]));
        let kept = doc.sweeps.insert(sweep(vec![1, 2]));
        assert!(doc.sweeps.remove(doomed).is_some());
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Sweep(kept),
            material: None,
            name: None,
            shadow: false,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_sweep_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.sweeps.len(), 1, "{suffix}");
            assert_eq!(
                loaded.sweeps.get(kept).map(|s| s.path.clone()),
                Some(vec![lkey(1), lkey(2)]),
                "{suffix}: the surviving sweep did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies.values().nth(0).unwrap().source,
                crate::model::BodySource::Sweep(kept),
                "{suffix}: its body still points at it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: a revolve keeps its key across a save, and so does the `FaceId::RevolveCap`
    /// a sketch hosted on its flat face holds — a renumbering reload would host that sketch
    /// on a different revolve.
    #[test]
    fn revolution_keys_survive_a_save_and_reload() {
        let revolution = |angle: f32| crate::model::Revolution {
            sketch: skey(0),
            faces: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            angle_deg: angle,
            pitch_mm: 0.0,
            symmetric: false,
            mode: crate::model::RevolveMode::NewBody,
            name: None,
        };
        let mut doc = Document::default();
        let doomed = doc.revolutions.insert(revolution(90.0));
        let kept = doc.revolutions.insert(revolution(180.0));
        assert!(doc.revolutions.remove(doomed).is_some());
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Revolve(kept),
            material: None,
            name: None,
            shadow: false,
        });
        doc.add_sketch(crate::model::FaceId::RevolveCap {
            revolution: kept,
            profile: crate::model::ExtrudeFace::Circle(rkey(0)),
            end: true,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_revolve_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.revolutions.len(), 1, "{suffix}");
            assert_eq!(
                loaded.revolutions.get(kept).map(|r| r.angle_deg),
                Some(180.0),
                "{suffix}: the surviving revolve did not shift into the hole"
            );
            assert_eq!(
                loaded.bodies.values().nth(0).unwrap().source,
                crate::model::BodySource::Revolve(kept),
                "{suffix}: its body still points at it"
            );
            assert_eq!(
                loaded.sketches[skey(0)].face,
                doc.sketches[skey(0)].face,
                "{suffix}: and so does the sketch hosted on its cap"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: the same for imported meshes, which a body names through
    /// `BodySource::Imported`.
    #[test]
    fn imported_mesh_keys_survive_a_save_and_reload() {
        let mesh = |name: &str| crate::model::ImportedMesh {
            triangles: Vec::new(),
            source_name: name.to_string(),
            step_bytes: None,
        };
        let mut doc = Document::default();
        let doomed = doc.imported_meshes.insert(mesh("doomed"));
        let kept = doc.imported_meshes.insert(mesh("kept"));
        assert!(doc.imported_meshes.remove(doomed).is_some());
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(kept),
            material: None,
            name: None,
            shadow: false,
        });

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_mesh_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.imported_meshes.len(), 1, "{suffix}");
            assert_eq!(
                loaded.bodies.values().nth(0).unwrap().source,
                crate::model::BodySource::Imported(kept),
                "{suffix}: the body still names the mesh it was imported from"
            );
            assert_eq!(
                loaded.imported_meshes.get(kept).map(|m| m.source_name.as_str()),
                Some("kept"),
                "{suffix}: which did not shift into the hole"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: the same for tracing images, whose keys are held by move-op targets and by
    /// the calibration constraints on them — a renumbering reload would point those at the
    /// wrong image.
    #[test]
    fn tracing_image_keys_survive_a_save_and_reload() {
        let image = |name: &str| crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: name.to_string(),
            plane: pkey(0),
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 10.0,
            height_mm: 10.0,
            name: None,
            calibration: None,
        };
        let mut doc = Document::default();
        let doomed = doc.tracing_images.insert(image("doomed"));
        let kept = doc.tracing_images.insert(image("kept"));
        assert!(doc.tracing_images.remove(doomed).is_some());

        for suffix in [".bearcad", ".bearcad.json"] {
            let path = std::env::temp_dir().join(format!("bearcad_image_keys_test{suffix}"));
            let path = path.to_string_lossy().to_string();
            let _ = std::fs::remove_file(&path);
            save(&path, &doc).unwrap();
            let loaded = open(&path).unwrap();

            assert_eq!(loaded.tracing_images.len(), 1, "{suffix}");
            assert_eq!(
                loaded.tracing_images.get(kept).map(|i| i.source_name.as_str()),
                Some("kept"),
                "{suffix}: the surviving image did not shift into the hole"
            );
            assert!(
                loaded.tracing_images.get(doomed).is_none(),
                "{suffix}: a key to a removed image does not come back to life"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// `?open=<url>` document a screenshot scene publishes.
    #[test]
    fn json_path_saves_the_web_codec() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_json_save_test.bearcad.json");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.construction_planes[pkey(0)].name = Some("Ground".to_string());
        save(&path, &doc).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.first(), Some(&b'{'), "JSON, not SQLite");
        let loaded = open(&path).unwrap();
        assert_eq!(
            loaded.construction_planes[pkey(0)].name.as_deref(),
            Some("Ground")
        );

        std::fs::remove_file(&path).unwrap();
    }

    /// #892: a joint round-trips through the SQLite format — members, kind (with its
    /// embedded lead expression), frames, positions, rest pose, and limits.
    #[test]
    fn round_trips_joints() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_joint_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            name: None,
            material: None,
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(1)),
            name: None,
            material: None,
            shadow: false,
        });
        doc.joints.insert(crate::model::Joint {
            members: vec![
                crate::model::JointRef::Body(bkey(0)),
                crate::model::JointRef::Body(bkey(1)),
            ],
            base: 0,
            kind: crate::model::JointKind::Screw { lead: "2 * pitch".to_string() },
            placement: Default::default(),
            position: "90".to_string(),
            position2: String::new(),
            position3: String::new(),
            rest: "0".to_string(),
            rest2: String::new(),
            rest3: String::new(),
            limits: crate::model::JointLimits {
                slide_min: "-5".to_string(),
                slide_max: "height / 2".to_string(),
                slide_min_target: None,
                slide_max_target: None,
                turn_min: String::new(),
                turn_max: "110".to_string(),
            },
            name: Some("Lead screw".to_string()),
            frame: Default::default(),
        });
        doc.shape_order.push(crate::model::ShapeKind::Body);
        doc.shape_order.push(crate::model::ShapeKind::Body);
        doc.shape_order.push(crate::model::ShapeKind::Joint);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.joints, doc.joints);
        assert_eq!(loaded.shape_order, doc.shape_order);

        std::fs::remove_file(&path).unwrap();
    }

    /// #909: a primitive shape round-trips — kind, frame, and its dimension expressions —
    /// with the body that points back at it.
    #[test]
    fn round_trips_primitive_shapes() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_shape_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cylinder);
        shape.origin = [10.0, -4.0, 2.5];
        shape.normal = [0.0, 1.0, 0.0];
        shape.u_axis = [0.0, 0.0, 1.0];
        shape.radius = "bore / 2".to_string();
        shape.height = "18".to_string();
        shape.name = Some("Boss".to_string());
        // A shape removed before the save: the survivor must not slide into its slot.
        let doomed = doc
            .primitives
            .insert(crate::model::Primitive::new(crate::model::PrimitiveKind::Sphere));
        doc.primitives.remove(doomed);
        let key = doc.primitives.insert(shape);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(key),
            name: None,
            material: None,
            shadow: false,
        });
        doc.shape_order.push(crate::model::ShapeKind::Primitive);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.primitives, doc.primitives);
        assert_eq!(loaded.bodies, doc.bodies);
        assert_eq!(loaded.shape_order, doc.shape_order);
        assert_eq!(
            loaded.primitives.get(key).and_then(|s| s.name.clone()),
            Some("Boss".to_string()),
            "the shape's key still resolves to it (#1055)"
        );
        assert!(loaded.primitives.get(doomed).is_none(), "and the removed one stays gone");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_boolean_ops_and_shadow_bodies() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_boolean_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: true,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Boolean {
                op: bopkey(0),
                solid: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: Some("Result".to_string()),
            shadow: false,
        });
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![bkey(0)],
            b: vec![bkey(3)],
            keep_b: true,
            outputs: vec![bkey(1)],
            name: Some("Slot".to_string()),
        });
        doc.shape_order.push(ShapeKind::BooleanOperation);
        doc.shape_order.push(ShapeKind::Body);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.boolean_ops, doc.boolean_ops);
        assert_eq!(loaded.bodies, doc.bodies);
        assert_eq!(loaded.shape_order, doc.shape_order);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_slice_ops_and_shadow_bodies() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_slice_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: true,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Sliced {
                op: slckey(0),
                target: 0,
                piece: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: Some("Top".to_string()),
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Sliced {
                op: slckey(0),
                target: 0,
                piece: 1,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: Some("Bottom".to_string()),
            shadow: false,
        });
        doc.slice_ops.insert(crate::model::SliceOperation {
            targets: vec![bkey(0)],
            cutters: vec![crate::model::SliceCutter::Face(
                crate::model::FaceId::ConstructionPlane(pkey(3)),
            )],
            extend_infinite: true,
            outputs: vec![bkey(1), bkey(2)],
            name: Some("Halved".to_string()),
        });
        doc.shape_order.push(ShapeKind::SliceOperation);
        doc.shape_order.push(ShapeKind::Body);
        doc.shape_order.push(ShapeKind::Body);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.slice_ops, doc.slice_ops);
        assert_eq!(loaded.bodies, doc.bodies);
        assert_eq!(loaded.shape_order, doc.shape_order);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn world_positions_round_trip_through_save() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_world_positions_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let offset_plane = crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                25.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        );
        let mut doc = Document::default();
        retain_ground_plane_only(&mut doc);
        doc.construction_planes.insert(offset_plane);

        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(s0, 12.0, -8.0, 15.0, 0.4));
        doc.shape_order.push(ShapeKind::Circle);

        let s1 = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));
        crate::construction::add_line_rectangle(&mut doc, s1, 3.0, 4.0, 10.0, 10.0, [false; 4]);
        doc.lines
            .insert(Line::from_local_endpoints(s1, -2.0, 1.0, 8.0, 6.0));
        doc.shape_order.push(ShapeKind::Line);

        let before = element_world_anchors(&doc);
        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        let after = element_world_anchors(&loaded);
        assert_world_anchors_match(&before, &after);

        // A rectangle edge on the offset plane should keep its world height.
        let (a, _) = crate::face::line_world_endpoints(&loaded, &loaded.lines[lkey(0)]).unwrap();
        assert!(
            (a.z - 25.0).abs() < 1e-3,
            "geometry on the offset plane should keep its world height"
        );

        std::fs::remove_file(&path).unwrap();
    }

    /// #834: materials and each body's material survive a save/load.
    #[test]
    fn materials_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_materials_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        // A material removed before the save: the file must not renumber Brass onto its
        // slot, and the key the body holds has to keep meaning Brass (#1055).
        let doomed = doc.materials.insert(crate::model::Material {
            name: "Doomed".to_string(),
            color: [0, 0, 0],
        });
        doc.materials.remove(doomed);
        let brass = doc.materials.insert(crate::model::Material {
            name: "Brass".to_string(),
            color: [0xc8, 0x8a, 0x4a],
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: Some(brass),
            name: None,
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(1)),
            material: None,
            name: None,
            shadow: false,
        });

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.materials, doc.materials);
        assert_eq!(loaded.bodies.values().nth(0).unwrap().material, Some(brass));
        assert_eq!(loaded.materials[brass].name, "Brass");
        assert_eq!(loaded.materials.get(doomed), None, "and it stays removed");
        assert_eq!(loaded.bodies.values().nth(1).unwrap().material, None);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn construction_planes_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_construction_plane_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let offset_plane = crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                25.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        );
        let mut doc = Document::default();
        retain_ground_plane_only(&mut doc);
        doc.construction_planes.insert(offset_plane.clone());
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.construction_planes.len(), 2);
        assert_eq!(loaded.construction_planes[pkey(1)], offset_plane);
        assert_eq!(
            loaded.sketches[skey(0)].face,
            FaceId::ConstructionPlane(pkey(1)),
            "sketch should stay on the offset plane"
        );
        let (a, _) = crate::face::line_world_endpoints(&loaded, &loaded.lines[lkey(0)]).unwrap();
        assert!(
            (a.z - 25.0).abs() < 1e-3,
            "loaded geometry should keep its offset-plane world position"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn default_construction_plane_origin_round_trips() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_plane0_origin_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.construction_planes[pkey(0)].origin.z = 30.0;
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);

        let before_origin = doc.construction_planes[pkey(0)].origin;
        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(
            (loaded.construction_planes[pkey(0)].origin - before_origin).length() < 1e-3,
            "edited default plane origin should round-trip"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_files_without_planes_get_placeholder_indices() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_legacy_plane_ref_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 10.0));
        doc.shape_order.push(ShapeKind::Line);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(
            loaded.construction_planes.len() >= 2,
            "legacy sketch references to plane 1 should not crash on load"
        );
        assert!(crate::face::line_world_endpoints(&loaded, &loaded.lines[lkey(0)]).is_some());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_sketches() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_sketch_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        crate::construction::add_line_rectangle(&mut doc, s0, 0.0, 0.0, 1.0, 1.0, [false; 4]);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.sketches.len(), 2);
        assert_eq!(loaded.sketches[skey(0)].face, FaceId::ConstructionPlane(pkey(0)));
        assert_eq!(loaded.sketches[skey(1)].face, FaceId::ConstructionPlane(pkey(0)));
        assert_eq!(loaded.lines[lkey(0)].sketch, s0);
        let _ = s1;

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_extrusions_and_bodies() {
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion};
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_extrusion_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 12.0,
            target: None,
            expression: String::new(),
            name: Some("Boss".to_string()),
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc.shape_order.push(ShapeKind::Extrusion);
        doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.extrusions.len(), 1);
        assert_eq!(
            loaded.extrusions[xkey(0)].faces,
            vec![ExtrudeFace::Polygon(rect_lines.to_vec())]
        );
        assert_eq!(loaded.extrusions[xkey(0)].distance, 12.0);
        assert_eq!(loaded.extrusions[xkey(0)].name.as_deref(), Some("Boss"));
        assert_eq!(loaded.bodies.len(), 1);
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source, BodySource::Extrusion(xkey(0)));
        assert!(loaded.shape_order.contains(&ShapeKind::Extrusion));
        assert!(loaded.shape_order.contains(&ShapeKind::Body));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_body_with_cut_extrusion() {
        // A `Solid { add, cut }` body (#35): the cut list must survive save/load.
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion};
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_cut_body_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let outer =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        let inner =
            crate::construction::add_line_rectangle(&mut doc, sketch, 3.0, 3.0, 4.0, 4.0, [false; 4]);
        for face in [
            ExtrudeFace::Polygon(outer.to_vec()),
            ExtrudeFace::Polygon(inner.to_vec()),
        ] {
            doc.extrusions.insert(Extrusion {
                sketch,
                faces: vec![face],
                distance: 5.0,
                target: None,
                expression: String::new(),
                name: None,
                symmetric: false,
                taper: 0.0,
                taper_mode: crate::model::ExtrudeTaperMode::Distance,
                taper_expression: String::new(),
                edge_treatments: Vec::new(),
            });
            doc.shape_order.push(ShapeKind::Extrusion);
        }
        doc.bodies.insert(Body {
            source: BodySource::Solid { base: None, add: vec![xkey(0)],
                cut: vec![xkey(1)],
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(
            loaded.bodies.values().nth(0).unwrap().source,
            BodySource::Solid { base: None, add: vec![xkey(0)],
                cut: vec![xkey(1)],
            }
        );
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source.extrusion_indices(), [xkey(0)]);
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source.cut_extrusion_indices(), [xkey(1)]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_circles() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_circle_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut circle = Circle::from_local_center_radius(sketch, 5.0, 5.0, 10.0, 0.5);
        circle.diameter_dim_offset = Some(18.0);
        circle.diameter_dim_angle = 1.2;
        circle.construction = true;
        doc.circles.insert(circle);
        doc.shape_order.push(ShapeKind::Circle);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.circles, doc.circles);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_rejects_circular_parameters() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_circular_params_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.parameters.insert(Parameter {
            name: "A".to_string(),
            expression: "B".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.parameters.insert(Parameter {
            name: "B".to_string(),
            expression: "A".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        doc.shape_order.push(ShapeKind::Parameter);

        let err = save(&path, &doc).unwrap_err();
        assert!(err.contains("Circular dependency"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trips_parameters() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_parameters_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.parameters.insert(Parameter {
            name: "A".to_string(),
            expression: "5mm".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.parameters.insert(Parameter {
            name: "B".to_string(),
            expression: "A + 5in".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        doc.shape_order.push(ShapeKind::Parameter);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.parameters, doc.parameters);
        assert_eq!(loaded.shape_order, doc.shape_order);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_removed_entities() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_removal_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.remove(lkey(0));
        // A parameter that was removed for real (#1055): the file must not bring it back,
        // and its key must not come back to life either.
        let gone = doc.parameters.insert(Parameter {
            name: "width".to_string(),
            expression: "10mm".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        let kept = doc.parameters.insert(Parameter {
            name: "height".to_string(),
            expression: "20mm".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        doc.parameters.remove(gone);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(!loaded.lines.contains(lkey(0)));
        assert_eq!(loaded.lines.len(), 0);
        assert_eq!(loaded.parameters.len(), 1);
        assert_eq!(
            loaded.parameters.get(kept).map(|p| p.name.as_str()),
            Some("height"),
            "the surviving parameter kept its key"
        );
        assert!(loaded.parameters.get(gone).is_none(), "and the removed one stays gone");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_chamfer_fillet_parent_on_a_bridging_line() {
        // #76: `Line::chamfer_fillet_parent` is a `#[serde(default)]` field on an entity
        // already persisted via typed `lines` columns plus leftover payload JSON, so it should
        // round-trip — verify that rather than just trusting it.
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_chamfer_fillet_parent_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.insert(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        doc.shape_order.push(ShapeKind::Line);
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        bridge.chamfer_fillet_parent = Some(lkey(0));
        doc.lines.insert(bridge);
        doc.shape_order.push(ShapeKind::Line);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.lines.len(), 3);
        assert_eq!(loaded.lines[lkey(0)].chamfer_fillet_parent, None);
        assert_eq!(loaded.lines[lkey(1)].chamfer_fillet_parent, None);
        assert_eq!(loaded.lines[lkey(2)].chamfer_fillet_parent, Some(lkey(0)));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_a_deleted_line_with_an_alive_sibling() {
        use crate::document_lifecycle::delete_element;
        use crate::hierarchy::SceneElement;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine};

        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_removal_sibling.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(0)),
                line_b: ConstraintLine::Line(lkey(1)),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        doc.shape_order.push(ShapeKind::Constraint);
        delete_element(&mut doc, SceneElement::Line(lkey(0)));

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.lines.len(), 1);
        assert!(!loaded.lines.contains(lkey(0)));
        assert!(loaded.lines.contains(lkey(1)));
        assert_eq!(loaded.constraints.len(), 1);
        let health = crate::document_health::recompute_document_health(&loaded);
        assert_eq!(
            health.element_status(SceneElement::Line(lkey(1))),
            crate::document_health::HealthStatus::Unstable
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_document_default_units() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_document_units_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.default_length_unit = LengthUnit::In;
        doc.default_angle_unit = AngleUnit::Rad;

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.default_length_unit, LengthUnit::In);
        assert_eq!(loaded.default_angle_unit, AngleUnit::Rad);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_files_without_unit_meta_keys_fall_back_to_mm_and_deg() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_legacy_units_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        // Save a document, then delete the unit meta keys to simulate a pre-#52 file.
        let doc = Document::default();
        save(&path, &doc).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "DELETE FROM meta WHERE key IN (?1, ?2)",
                rusqlite::params![DEFAULT_LENGTH_UNIT_META_KEY, DEFAULT_ANGLE_UNIT_META_KEY],
            )
            .unwrap();
        }

        let loaded = open(&path).unwrap();
        assert_eq!(loaded.default_length_unit, LengthUnit::Mm);
        assert_eq!(loaded.default_angle_unit, AngleUnit::Deg);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_sketch_unit_override_and_inherit() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_sketch_units_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let overridden = plane_sketch(&mut doc);
        doc.sketches[overridden].length_unit = Some(LengthUnit::Cm);
        doc.sketches[overridden].angle_unit = Some(AngleUnit::Rad);
        let inheriting = plane_sketch(&mut doc);
        assert_eq!(doc.sketches[inheriting].length_unit, None);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.sketches[overridden].length_unit, Some(LengthUnit::Cm));
        assert_eq!(loaded.sketches[overridden].angle_unit, Some(AngleUnit::Rad));
        assert_eq!(loaded.sketches[inheriting].length_unit, None);
        assert_eq!(loaded.sketches[inheriting].angle_unit, None);

        std::fs::remove_file(&path).unwrap();
    }

    /// A small standalone document to embed as a unit (#719).
    fn unit_source_doc(param: &str) -> Document {
        let mut doc = Document::default();
        doc.parameters.insert(crate::model::Parameter {
            name: param.to_string(),
            expression: "10".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc
    }

    /// #719: a document with two units and several instances round-trips through SQLite —
    /// and, since the sources' files don't exist on disk, this also shows a document whose
    /// unit file is missing still loads (the embedded copies make it self-contained).
    #[test]
    fn units_and_instances_round_trip() {
        use crate::model::{ImportedUnit, LinkMode, UnitInstance, UnitPlacement, UnitSource};
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_units_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.units.insert(ImportedUnit {
            source: UnitSource::RelativePath("missing/bracket.bearcad".to_string()),
            link: LinkMode::Static,
            document: unit_source_doc("width"),
            source_mtime: Some(1_700_000_000),
            source_hash: Some(crate::model::content_hash(b"bracket bytes")),
        });
        doc.units.insert(ImportedUnit {
            source: UnitSource::Library("hardware/bolt.bearcad".to_string()),
            link: LinkMode::Dynamic,
            document: unit_source_doc("length"),
            source_mtime: None,
            source_hash: None,
        });
        doc.unit_instances.insert(UnitInstance {
            unit: ukey(0),
            name: Some("bracket1".to_string()),
            parameter_overrides: vec![("width".to_string(), "20".to_string())],
            placement: UnitPlacement {
                tx: "5".to_string(),
                ty: String::new(),
                tz: "height / 2".to_string(),
                axis: [0.0, 0.0, 1.0],
                angle: "90".to_string(),
            },
        });
        doc.unit_instances.insert(UnitInstance {
            unit: ukey(1),
            name: Some("bolt_a".to_string()),
            parameter_overrides: Vec::new(),
            placement: UnitPlacement::default(),
        });

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.units, doc.units);
        assert_eq!(loaded.unit_instances, doc.unit_instances);

        // The JSON byte format (web save/load) round-trips the same content.
        let bytes = super::super::to_json_bytes(&doc).unwrap();
        let reloaded = super::super::from_json_bytes(&bytes).unwrap();
        assert_eq!(reloaded.units, doc.units);
        assert_eq!(reloaded.unit_instances, doc.unit_instances);

        std::fs::remove_file(&path).unwrap();
    }

    /// #719: an existing pre-units document (no `units`/`unit_instances` fields in its
    /// JSON) still loads, with both defaulting to empty.
    #[test]
    fn documents_without_unit_fields_still_load() {
        let mut value =
            serde_json::to_value(Document::default()).expect("serialize default document");
        let obj = value.as_object_mut().unwrap();
        obj.remove("units");
        obj.remove("unit_instances");
        let bytes = serde_json::to_vec(&value).unwrap();
        let loaded = super::super::from_json_bytes(&bytes).expect("pre-units document loads");
        assert!(loaded.units.is_empty());
        assert!(loaded.unit_instances.is_empty());
    }

    /// #719: a cycle — the opened file A embeds B, whose embedded copy claims to import A
    /// again — is refused at load, matched on resolved source path.
    #[test]
    fn unit_import_cycle_is_refused_at_load() {
        use crate::model::{ImportedUnit, LinkMode, UnitSource};
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_unit_cycle_test.bearcad");
        let path_str = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut inner_b = Document::default();
        inner_b.units.insert(ImportedUnit {
            source: UnitSource::RelativePath("bearcad_unit_cycle_test.bearcad".to_string()),
            link: LinkMode::Static,
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        });
        let mut doc = Document::default();
        doc.units.insert(ImportedUnit {
            source: UnitSource::RelativePath("b.bearcad".to_string()),
            link: LinkMode::Static,
            document: inner_b,
            source_mtime: None,
            source_hash: None,
        });

        save(&path_str, &doc).unwrap();
        let err = open(&path_str).expect_err("cycle must refuse to load");
        assert!(err.contains("cycle"), "error should name the cycle: {err}");

        std::fs::remove_file(&path).unwrap();
    }

    /// #719: unit nesting deeper than the hard cap is refused with a clear error rather
    /// than recursing toward a stack overflow.
    #[test]
    fn unit_nesting_deeper_than_cap_is_refused() {
        use crate::model::{ImportedUnit, LinkMode, UnitSource, MAX_UNIT_DEPTH};
        let mut doc = Document::default();
        for level in 0..=MAX_UNIT_DEPTH {
            let mut outer = Document::default();
            outer.units.insert(ImportedUnit {
                source: UnitSource::RelativePath(format!("level{level}.bearcad")),
                link: LinkMode::Static,
                document: doc,
                source_mtime: None,
                source_hash: None,
            });
            doc = outer;
        }
        let bytes = super::super::to_json_bytes(&doc).unwrap();
        let err = super::super::from_json_bytes(&bytes)
            .expect_err("over-deep nesting must refuse to load");
        assert!(err.contains("nest"), "error should mention nesting: {err}");
    }

    /// #1340: the file is a real schema — `SELECT name FROM parameters` works without
    /// walking a JSON dump, and `dag_nodes` is gone.
    #[test]
    fn parameters_table_is_queryable_without_json_dump() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_typed_parameters_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        doc.parameters.insert(Parameter {
            name: "width".to_string(),
            expression: "24mm".to_string(),
            primary: true,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.parameters.insert(Parameter {
            name: "height".to_string(),
            expression: "width * 2".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        doc.shape_order.push(ShapeKind::Parameter);

        save(&path, &doc).unwrap();

        let conn = Connection::open(&path).unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM parameters ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(names, vec!["height".to_string(), "width".to_string()]);

        let width_expr: String = conn
            .query_row(
                "SELECT expression FROM parameters WHERE name = 'width'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(width_expr, "24mm");

        let dag: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'dag_nodes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dag, 0, "the dump table must not exist");

        let version: i64 = conn
            .query_row("SELECT MAX(id) FROM schema_migrations", [], |row| row.get(0))
            .unwrap();
        assert!(
            version >= 2,
            "schema_migrations must record the typed-tables version, got {version}"
        );

        if let Ok(out) = std::process::Command::new("sqlite3")
            .args([&path, "SELECT name FROM parameters ORDER BY name;"])
            .output()
        {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                assert!(
                    stdout.contains("width") && stdout.contains("height"),
                    "sqlite3 CLI should see typed parameter names: {stdout}"
                );
            }
        }

        std::fs::remove_file(&path).unwrap();
    }

    /// #1340: preview PNG/STL live in `blobs`, not base64 meta text.
    #[test]
    fn preview_is_stored_as_a_blob() {
        use crate::model::{Body, BodySource, ImportedMesh};
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_preview_blob_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let p = |x, y, z| glam::Vec3::new(x, y, z);
        let mesh = doc.imported_meshes.insert(ImportedMesh {
            triangles: vec![
                [p(0., 0., 0.), p(10., 0., 0.), p(0., 10., 0.)],
                [p(0., 0., 0.), p(0., 10., 0.), p(0., 0., 10.)],
            ],
            source_name: "tri".into(),
            step_bytes: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Imported(mesh),
            material: None,
            name: Some("tri".into()),
            shadow: false,
        });

        save(&path, &doc).unwrap();
        crate::file_preview::attach_preview_after_save(&path, &doc);

        let conn = Connection::open(&path).unwrap();
        let png: Vec<u8> = conn
            .query_row(
                "SELECT bytes FROM blobs WHERE kind = 'preview_png'",
                [],
                |row| row.get(0),
            )
            .expect("preview_png must be a blob row");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "PNG magic");
        assert!(png.len() > 50);

        let stl: Vec<u8> = conn
            .query_row(
                "SELECT bytes FROM blobs WHERE kind = 'preview_stl'",
                [],
                |row| row.get(0),
            )
            .expect("preview_stl must be a blob row");
        assert!(stl.len() >= 84, "binary STL header + count");

        let meta_png: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key IN ('preview_png', 'preview_stl')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(meta_png, 0, "preview must not live in meta as base64");

        std::fs::remove_file(&path).unwrap();
    }

    /// #1340: a dangling `lines.sketch_id` must load. Integrity is document health,
    /// not a FOREIGN KEY refuse.
    #[test]
    fn line_whose_sketch_is_gone_reloads_unhealthy() {
        use crate::document_health::{recompute_document_health, HealthStatus};
        use crate::hierarchy::SceneElement;

        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_dangling_sketch_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = plane_sketch(&mut doc);
        let line = doc
            .lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        assert!(doc.sketches.remove(sketch).is_some());

        save(&path, &doc).expect("dangling sketch_id must save");
        let loaded = open(&path).expect("dangling sketch_id must load, not be refused");
        assert!(loaded.lines.contains(line), "the line survived");
        assert!(!loaded.sketches.contains(sketch), "the sketch stayed gone");
        assert_eq!(
            loaded.lines[line].sketch, sketch,
            "the dangling sketch_id must round-trip, not be healed"
        );

        let health = recompute_document_health(&loaded);
        assert_ne!(
            health.element_status(SceneElement::Line(line)),
            HealthStatus::Healthy,
            "a line whose sketch is gone is unhealthy"
        );

        std::fs::remove_file(&path).unwrap();
    }

    /// #1340: open reads only the 16-byte SQLite header, not the whole file.
    #[test]
    fn open_sniffs_sixteen_bytes_not_the_whole_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_sniff_header_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut header = b"SQLite format 3\0".to_vec();
        header.extend_from_slice(&[0u8; 64]);
        std::fs::write(&path, &header).unwrap();

        let err = open(&path).expect_err("truncated sqlite should fail as sqlite, not JSON");
        assert!(
            !err.contains("expected value") && !err.contains("EOF while parsing"),
            "must not parse the file as JSON after sniffing sqlite magic: {err}"
        );

        std::fs::remove_file(&path).unwrap();
    }
}

}

#[cfg(not(target_arch = "wasm32"))]
pub use sqlite_format::{delete_preview_blob, open, save, upsert_preview_blob};
#[cfg(all(not(target_arch = "wasm32"), test))]
pub use sqlite_format::load_preview_blob;

/// Path-based IO doesn't exist in the browser — the web build opens/saves through the
/// file-picker byte flows (`to_json_bytes`/`from_json_bytes`). These stubs keep the
/// path-based `Action::Open`/`Action::SaveAs` arms compiling; reaching them on web is a
/// clear error rather than a crash.
#[cfg(target_arch = "wasm32")]
pub fn open(_path: &str) -> Result<Document> {
    Err("opening by file path isn't available in the browser".to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn save(_path: &str, _doc: &Document) -> Result<()> {
    Err("saving by file path isn't available in the browser".to_string())
}
