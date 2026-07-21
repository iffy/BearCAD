//! STEP (ISO 10303-21) export of tessellated solid meshes as FACETED_BREP.

use crate::extrude::SolidMesh;
use glam::Vec3;
use std::fmt::Write;

/// Serialize a solid mesh as an AP203 FACETED_BREP STEP document named `name`.
pub fn write_step(name: &str, mesh: &SolidMesh) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "ISO-10303-21;");
    out.push_str("HEADER;\n");
    out.push_str("FILE_DESCRIPTION(('BearCAD mesh export'),'2;1');\n");
    let _ = writeln!(
        out,
        "FILE_NAME('{name}','2026-01-01T00:00:00',('BearCAD'),('BearCAD'),'BearCAD','BearCAD','');"
    );
    out.push_str("FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\n");
    out.push_str("ENDSEC;\n");
    out.push_str("DATA;\n");

    let mut id = 1usize;
    // A complex (multi-supertype) entity instance must be wrapped in parentheses per
    // Part 21, and the unit list must reference the three unit instances below —
    // OCCT's reader rejected the previous unwrapped form ("Incorrect Syntax", #106).
    let context = id;
    let length_unit = id + 1;
    let angle_unit = id + 2;
    let solid_angle_unit = id + 3;
    out.push_str(&format!(
        "#{context}=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length_unit},#{angle_unit},#{solid_angle_unit}))REPRESENTATION_CONTEXT('{name}','MODEL'));\n"
    ));
    out.push_str(&format!(
        "#{length_unit}=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n"
    ));
    out.push_str(&format!(
        "#{angle_unit}=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\n"
    ));
    out.push_str(&format!(
        "#{solid_angle_unit}=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());\n"
    ));
    id += 4;

    let mut face_ids = Vec::with_capacity(mesh.triangles.len());
    for tri in &mesh.triangles {
        let face_id = write_triangle_face(&mut out, &mut id, *tri);
        face_ids.push(face_id);
    }

    let shell = id;
    let _ = writeln!(
        out,
        "#{shell}=CLOSED_SHELL('',({}));",
        face_refs(&face_ids)
    );
    id += 1;
    let brep = id;
    let _ = writeln!(out, "#{brep}=FACETED_BREP('{name}',#{shell});");
    id += 1;
    let shape = id;
    let _ = writeln!(
        out,
        "#{shape}=ADVANCED_BREP_SHAPE_REPRESENTATION('{name}',(#{brep}),#{context});"
    );
    id += 1;

    // AP203 product scaffolding (#106): readers (OCCT included) only *transfer*
    // geometry that is anchored to a product via SHAPE_DEFINITION_REPRESENTATION —
    // a bare shape representation parses but yields no shapes. This chain makes
    // BearCAD's faceted export consumable by the kernel and third-party CAD.
    let app_ctx = id;
    let _ = writeln!(
        out,
        "#{app_ctx}=APPLICATION_CONTEXT('configuration controlled 3d designs of mechanical parts and assemblies');"
    );
    let _ = writeln!(
        out,
        "#{}=APPLICATION_PROTOCOL_DEFINITION('international standard','config_control_design',1994,#{app_ctx});",
        id + 1
    );
    let product_ctx = id + 2;
    let _ = writeln!(
        out,
        "#{product_ctx}=MECHANICAL_CONTEXT('',#{app_ctx},'mechanical');"
    );
    let product = id + 3;
    let _ = writeln!(
        out,
        "#{product}=PRODUCT('{name}','{name}','',(#{product_ctx}));"
    );
    let formation = id + 4;
    let _ = writeln!(
        out,
        "#{formation}=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('','',#{product},.NOT_KNOWN.);"
    );
    let def_ctx = id + 5;
    let _ = writeln!(
        out,
        "#{def_ctx}=PRODUCT_DEFINITION_CONTEXT('part definition',#{app_ctx},'design');"
    );
    let definition = id + 6;
    let _ = writeln!(
        out,
        "#{definition}=PRODUCT_DEFINITION('design','',#{formation},#{def_ctx});"
    );
    let def_shape = id + 7;
    let _ = writeln!(
        out,
        "#{def_shape}=PRODUCT_DEFINITION_SHAPE('','',#{definition});"
    );
    let _ = writeln!(
        out,
        "#{}=SHAPE_DEFINITION_REPRESENTATION(#{def_shape},#{shape});",
        id + 8
    );

    out.push_str("ENDSEC;\n");
    out.push_str("END-ISO-10303-21;\n");
    out
}

fn face_refs(ids: &[usize]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn write_triangle_face(out: &mut String, next_id: &mut usize, tri: [Vec3; 3]) -> usize {
    let [a, b, c] = tri;
    let p0 = *next_id;
    write_cartesian_point(out, p0, a);
    *next_id += 1;
    let p1 = *next_id;
    write_cartesian_point(out, p1, b);
    *next_id += 1;
    let p2 = *next_id;
    write_cartesian_point(out, p2, c);
    *next_id += 1;

    let poly = *next_id;
    let _ = writeln!(out, "#{poly}=POLY_LOOP('',(#{p0},#{p1},#{p2}));");
    *next_id += 1;
    let bound = *next_id;
    let _ = writeln!(out, "#{bound}=FACE_OUTER_BOUND('',#{poly},.T.);");
    *next_id += 1;

    let normal = (b - a).cross(c - a).normalize_or_zero();
    let mut edge = b - a;
    if edge.length_squared() < 1e-8 {
        edge = c - a;
    }
    edge = (edge - normal * edge.dot(normal)).normalize_or_zero();
    if edge.length_squared() < 1e-8 {
        edge = if normal.z.abs() < 0.9 {
            Vec3::Z.cross(normal).normalize_or_zero()
        } else {
            Vec3::X.cross(normal).normalize_or_zero()
        };
    }

    let dir_normal = *next_id;
    write_direction(out, dir_normal, normal);
    *next_id += 1;
    let dir_edge = *next_id;
    write_direction(out, dir_edge, edge);
    *next_id += 1;
    let placement = *next_id;
    let _ = writeln!(
        out,
        "#{placement}=AXIS2_PLACEMENT_3D('',#{p0},#{dir_normal},#{dir_edge});"
    );
    *next_id += 1;
    let plane = *next_id;
    let _ = writeln!(out, "#{plane}=PLANE('',#{placement});");
    *next_id += 1;
    let face = *next_id;
    let _ = writeln!(out, "#{face}=FACE_SURFACE('',(#{bound}),#{plane},.T.);");
    *next_id += 1;
    face
}

fn write_cartesian_point(out: &mut String, id: usize, p: Vec3) {
    let _ = writeln!(out, "#{id}=CARTESIAN_POINT('',({},{},{}));", p.x, p.y, p.z);
}

fn write_direction(out: &mut String, id: usize, d: Vec3) {
    let _ = writeln!(out, "#{id}=DIRECTION('',({},{},{}));", d.x, d.y, d.z);
}

/// Reconstruct a triangle mesh from a STEP document's `FACETED_BREP` geometry (#71) — the
/// same `POLY_LOOP`-bounded planar `FACE_SURFACE` subset [`write_step`] produces. STEP files
/// using full BREP geometry (`ADVANCED_FACE`/`EDGE_LOOP`/curved surfaces, as most CAD tools
/// export) aren't supported — BearCAD has no NURBS/curve kernel yet (SPEC §10) — and are
/// rejected with a clear error rather than approximated or silently dropped.
pub fn parse_step_mesh(data: &str) -> Result<Vec<[Vec3; 3]>, String> {
    if !data.trim_start().starts_with("ISO-10303-21;") {
        return Err("not a STEP (ISO-10303-21) file".to_string());
    }
    let entities = parse_entities(data)?;

    if entities.values().any(|e| e.kind == "FACE_BOUND") {
        return Err("STEP faces with holes (FACE_BOUND) aren't supported yet".to_string());
    }

    let mut points = std::collections::HashMap::new();
    let mut poly_loops = std::collections::HashMap::new();
    let mut outer_bounds = std::collections::HashMap::new();
    for (&id, entity) in &entities {
        match entity.kind.as_str() {
            "CARTESIAN_POINT" => {
                if let Some(p) = parse_cartesian_point(&entity.args) {
                    points.insert(id, p);
                }
            }
            "POLY_LOOP" => {
                if let Some(refs) = parse_ref_list(&entity.args) {
                    poly_loops.insert(id, refs);
                }
            }
            "FACE_OUTER_BOUND" => {
                if let Some(bound_id) = parse_single_ref(&entity.args) {
                    outer_bounds.insert(id, bound_id);
                }
            }
            _ => {}
        }
    }

    let mut triangles = Vec::new();
    let mut face_count = 0usize;
    for entity in entities.values() {
        if entity.kind != "FACE_SURFACE" {
            continue;
        }
        face_count += 1;
        let args = split_top_level(&entity.args);
        let bound_refs = args
            .get(1)
            .and_then(|s| parse_ref_list(s))
            .ok_or_else(|| "FACE_SURFACE has no bound list".to_string())?;
        for bound_ref in bound_refs {
            let loop_id = outer_bounds.get(&bound_ref).ok_or_else(|| {
                "FACE_SURFACE references a bound type BearCAD can't import (only \
                 FACE_OUTER_BOUND over a POLY_LOOP is supported)"
                    .to_string()
            })?;
            let loop_points = poly_loops
                .get(loop_id)
                .ok_or_else(|| "FACE_OUTER_BOUND references a missing POLY_LOOP".to_string())?;
            let positions: Vec<Vec3> = loop_points
                .iter()
                .map(|pid| {
                    points
                        .get(pid)
                        .copied()
                        .ok_or_else(|| "POLY_LOOP references a missing CARTESIAN_POINT".to_string())
                })
                .collect::<Result<_, _>>()?;
            for i in 1..positions.len().saturating_sub(1) {
                triangles.push([positions[0], positions[i], positions[i + 1]]);
            }
        }
    }

    if face_count == 0 {
        let hint = if entities.values().any(|e| e.kind == "ADVANCED_FACE") {
            " (it uses full BREP geometry — curved/NURBS surfaces — which BearCAD can't \
              import yet; only the simplified FACETED_BREP subset BearCAD itself exports is \
              supported)"
        } else {
            ""
        };
        return Err(format!("no FACE_SURFACE entities found in STEP file{hint}"));
    }
    if triangles.is_empty() {
        return Err("STEP file has faces but no usable triangle geometry".to_string());
    }
    Ok(triangles)
}

struct StepEntity {
    kind: String,
    args: String,
}

/// Parse every `#id=KIND(args);` record in the STEP file's `DATA` section into a lookup table.
/// Records with no simple `KIND(...)` form (e.g. the header's complex unit records) are
/// skipped — they aren't needed to reconstruct `FACETED_BREP` geometry.
fn parse_entities(data: &str) -> Result<std::collections::HashMap<usize, StepEntity>, String> {
    let data_start = data.find("DATA;").ok_or("missing DATA section")? + "DATA;".len();
    let rest = &data[data_start..];
    let data_end = rest.find("ENDSEC;").ok_or("missing ENDSEC after DATA")?;
    let body = &rest[..data_end];

    let mut entities = std::collections::HashMap::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'#' {
            i += 1;
            continue;
        }
        let id_start = i + 1;
        let mut j = id_start;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j == id_start {
            i += 1;
            continue;
        }
        let id: usize = body[id_start..j].parse().map_err(|_| "bad entity id")?;
        let mut k = j;
        while bytes.get(k) == Some(&b' ') {
            k += 1;
        }
        if bytes.get(k) != Some(&b'=') {
            i = j;
            continue;
        }
        k += 1;
        while bytes.get(k) == Some(&b' ') {
            k += 1;
        }
        let kind_start = k;
        while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
            k += 1;
        }
        if k == kind_start || bytes.get(k) != Some(&b'(') {
            // Complex record (e.g. `#5=(LENGTH_UNIT()...)`) or malformed — not needed here.
            i = j;
            continue;
        }
        let kind = body[kind_start..k].to_string();
        let end = find_matching_paren(body, k)?;
        entities.insert(
            id,
            StepEntity {
                kind,
                args: body[k + 1..end].to_string(),
            },
        );
        i = end + 1;
    }
    Ok(entities)
}

/// Index of the `)` matching the `(` at `open_idx`, ignoring parens inside `'...'` strings.
fn find_matching_paren(s: &str, open_idx: usize) -> Result<usize, String> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => in_string = !in_string,
            b'(' if !in_string => depth += 1,
            b')' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("unbalanced parentheses in STEP data".to_string())
}

/// Split a STEP argument list on top-level commas (i.e. not inside nested parens or strings).
fn split_top_level(args: &str) -> Vec<&str> {
    let bytes = args.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut start = 0usize;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'\'' => in_string = !in_string,
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth -= 1,
            b',' if !in_string && depth == 0 => {
                parts.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    parts
}

/// Parse `CARTESIAN_POINT` args, e.g. `'',(1.5,2,3)`.
fn parse_cartesian_point(args: &str) -> Option<Vec3> {
    let top = split_top_level(args);
    let coords = top.get(1)?.trim_start_matches('(').trim_end_matches(')');
    let mut parts = split_top_level(coords).into_iter().map(|s| s.parse::<f32>());
    Some(Vec3::new(parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?))
}

/// Parse a `(#a,#b,...)` argument (e.g. a `POLY_LOOP`'s point list) into entity ids.
fn parse_ref_list(args: &str) -> Option<Vec<usize>> {
    let top = split_top_level(args);
    let list = top.iter().find(|s| s.starts_with('('))?;
    let inner = list.trim_start_matches('(').trim_end_matches(')');
    split_top_level(inner)
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.strip_prefix('#')?.parse().ok())
        .collect()
}

/// Parse the first `#id` reference among a STEP argument list (e.g. `FACE_OUTER_BOUND`'s
/// `'',#loop,.T.`).
fn parse_single_ref(args: &str) -> Option<usize> {
    split_top_level(args)
        .into_iter()
        .find_map(|s| s.strip_prefix('#')?.parse().ok())
}

/// Basic structural validation for tests and round-trip checks.
#[cfg(test)]
pub fn validate_step(data: &str) -> Result<StepSummary, String> {
    if !data.starts_with("ISO-10303-21;") {
        return Err("missing ISO-10303-21 header".into());
    }
    if !data.contains("ENDSEC;") || !data.trim_end().ends_with("END-ISO-10303-21;") {
        return Err("incomplete exchange structure".into());
    }
    let cartesian_points = data.matches("CARTESIAN_POINT").count();
    let face_surfaces = data.matches("FACE_SURFACE").count();
    let faceted_brep = data.matches("FACETED_BREP").count();
    if faceted_brep != 1 {
        return Err(format!("expected 1 FACETED_BREP, found {faceted_brep}"));
    }
    if face_surfaces == 0 {
        return Err("no FACE_SURFACE entities".into());
    }
    if cartesian_points < face_surfaces * 3 {
        return Err("too few CARTESIAN_POINT entities for faces".into());
    }
    Ok(StepSummary {
        cartesian_points,
        face_surfaces,
    })
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepSummary {
    pub cartesian_points: usize,
    pub face_surfaces: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_mesh() -> SolidMesh {
        SolidMesh {
            triangles: vec![
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(10.0, 0.0, 0.0),
                    Vec3::new(10.0, 4.0, 0.0),
                ],
                [
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(10.0, 4.0, 0.0),
                    Vec3::new(0.0, 4.0, 0.0),
                ],
            ],
        }
    }

    /// #106: the representation-context is a complex (multi-supertype) entity and
    /// must be parenthesized per Part 21, with the unit list referencing the three
    /// unit instances — the earlier unwrapped form (with dangling `#2`/`#4` refs)
    /// made OCCT's reader reject BearCAD's own files as "Incorrect Syntax".
    #[test]
    fn write_step_emits_a_conformant_complex_context_entity() {
        let text = write_step("part", &box_mesh());
        assert!(
            text.contains("#1=(GEOMETRIC_REPRESENTATION_CONTEXT(3)"),
            "context entity must be wrapped in parentheses"
        );
        assert!(
            text.contains("GLOBAL_UNIT_ASSIGNED_CONTEXT((#2,#3,#4))"),
            "unit list must reference the three unit instances"
        );
        assert!(text.contains("#2=(LENGTH_UNIT()"));
        assert!(text.contains("#3=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()"));
        assert!(text.contains("#4=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)"));
    }

    /// #106: OCCT's STEP reader must accept BearCAD's own faceted export (the
    /// end-to-end conformance check the unit test above approximates).
    #[test]
    fn occt_reader_accepts_write_step_output() {
        let text = write_step("part", &box_mesh());
        let path = std::env::temp_dir().join("bearcad_step_conformance.step");
        std::fs::write(&path, &text).expect("write temp step");
        let shape = crate::kernel::Shape::read_step(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            shape.is_some(),
            "OCCT must parse BearCAD's own faceted STEP output"
        );
    }

    #[test]
    fn parse_step_mesh_round_trips_write_step_output() {
        let mesh = box_mesh();
        let text = write_step("part", &mesh);
        let mut triangles = parse_step_mesh(&text).expect("parse own export");
        assert_eq!(triangles.len(), mesh.triangles.len());
        // FACE_SURFACE entities are keyed by a HashMap while parsing, so triangle order
        // isn't preserved — only membership (each POLY_LOOP still keeps its own point order).
        for original in &mesh.triangles {
            let pos = triangles.iter().position(|parsed| {
                parsed.iter().zip(original).all(|(p, o)| (*p - *o).length() < 1e-4)
            });
            let pos = pos.unwrap_or_else(|| panic!("missing triangle {original:?} in {triangles:?}"));
            triangles.remove(pos);
        }
    }

    #[test]
    fn parse_step_mesh_rejects_non_step_text() {
        assert!(parse_step_mesh("not a step file").is_err());
    }

    #[test]
    fn parse_step_mesh_rejects_empty_export() {
        let mesh = SolidMesh::default();
        let text = write_step("empty", &mesh);
        let err = parse_step_mesh(&text).unwrap_err();
        assert!(err.contains("no FACE_SURFACE"), "{err}");
    }

    #[test]
    fn parse_step_mesh_gives_a_helpful_error_for_advanced_face_brep() {
        // A minimal document using the ADVANCED_FACE/curved-surface flavor most real CAD
        // tools export, which BearCAD can't tessellate — should fail clearly, not silently
        // produce wrong (or zero) geometry.
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=ADVANCED_FACE('',(#2),#3,.T.);\nENDSEC;\nEND-ISO-10303-21;\n";
        let err = parse_step_mesh(text).unwrap_err();
        assert!(err.contains("full BREP geometry"), "{err}");
    }

    #[test]
    fn parse_step_mesh_rejects_faces_with_holes() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=FACE_BOUND('',#2,.T.);\nENDSEC;\nEND-ISO-10303-21;\n";
        let err = parse_step_mesh(text).unwrap_err();
        assert!(err.contains("holes"), "{err}");
    }

    #[test]
    fn write_step_emits_faceted_brep_structure() {
        let text = write_step("part", &box_mesh());
        let summary = validate_step(&text).expect("valid step");
        assert_eq!(summary.face_surfaces, 2);
        assert!(summary.cartesian_points >= 6);
        assert!(text.contains("FACETED_BREP('part'"));
        assert!(text.contains("CONFIG_CONTROL_DESIGN"));
    }

    #[test]
    fn write_step_empty_mesh_has_no_faces() {
        let mesh = SolidMesh::default();
        let text = write_step("empty", &mesh);
        assert!(text.starts_with("ISO-10303-21;"));
        assert!(text.contains("FACETED_BREP('empty'"));
        assert_eq!(text.matches("FACE_SURFACE").count(), 0);
        assert!(validate_step(&text).is_err());
    }
}