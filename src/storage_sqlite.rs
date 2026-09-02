// Native `.bearcad` SQLite format: one table per arena (todoer #1340).
// Typed columns for names, expressions, flags, numeric fields, and integer refs.
// JSON only for tagged unions and irregular lists. Binary data lives in `blobs`.
// No FOREIGN KEY constraints — dangling refs must round-trip.

use crate::arena::{Arena, Key};
use crate::model::{
    Body, BodySource, BooleanOperation, Circle, Component, ComponentMember, Constraint,
    ConstraintKind, ConstructionPlane, ConstructionPlaneParent, Document, Drawing,
    EdgeTreatmentOperation, Extrusion, ImportedMesh, ImportedUnit, Joint, JointKind, Line,
    LinkMode, Loft, Material, MirrorOperation, MoveOperation, Parameter, Primitive,
    RepeatOperation, Revolution, ShapeKind, ShellOperation, Sketch, SketchMirrorOperation,
    SketchOffsetOperation, SketchRepeatOperation, SketchSliceOperation, SketchText,
    SketchVertexTreatmentOperation, SliceOperation, Sweep, TracingImage, UnitInstance,
};
use crate::parameters::validate_document_parameters_no_cycles;
use crate::value::{AngleUnit, LengthUnit};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;

use super::Result;

fn default_true() -> bool {
    true
}

/// Bump when the on-disk schema changes. v1 was the `dag_nodes` dump; this is the typed
/// one-table-per-arena format. Pre-alpha: no reader for the old dump.
const SCHEMA_VERSION: i64 = 2;
const SCHEMA_MIGRATION_NAME: &str = "typed_entity_tables";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_LENGTH_UNIT_META_KEY: &str = "default_length_unit";
const DEFAULT_ANGLE_UNIT_META_KEY: &str = "default_angle_unit";

/// Dedicated `blobs.id` for document-level preview PNG/STL (not an arena key).
pub const PREVIEW_BLOB_ID: i64 = 0;
const BLOB_FONT: &str = "font";
const BLOB_TRACING_IMAGE: &str = "tracing_image";
const BLOB_MESH_TRIANGLES: &str = "mesh_triangles";
const BLOB_STEP: &str = "step";

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS blobs (
            id    INTEGER NOT NULL,
            kind  TEXT NOT NULL,
            bytes BLOB NOT NULL,
            PRIMARY KEY (id, kind)
        );
        CREATE TABLE IF NOT EXISTS parameters (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            expression TEXT NOT NULL,
            is_primary INTEGER NOT NULL DEFAULT 1,
            minimum TEXT,
            maximum TEXT,
            step TEXT,
            source_json TEXT
        );
        CREATE TABLE IF NOT EXISTS sketches (
            id INTEGER PRIMARY KEY,
            name TEXT,
            length_unit TEXT,
            angle_unit TEXT,
            face_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS lines (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            x0 REAL NOT NULL, y0 REAL NOT NULL,
            x1 REAL NOT NULL, y1 REAL NOT NULL,
            construction INTEGER NOT NULL DEFAULT 0,
            shadow INTEGER NOT NULL DEFAULT 0,
            length_locked INTEGER NOT NULL DEFAULT 0,
            length_expr TEXT,
            length_dim_offset REAL,
            name TEXT,
            payload_json TEXT
        );
        CREATE TABLE IF NOT EXISTS circles (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            cx REAL NOT NULL, cy REAL NOT NULL, r REAL NOT NULL,
            construction INTEGER NOT NULL DEFAULT 0,
            shadow INTEGER NOT NULL DEFAULT 0,
            diameter_locked INTEGER NOT NULL DEFAULT 0,
            diameter_expr TEXT,
            diameter_dim_offset REAL,
            diameter_dim_angle REAL NOT NULL DEFAULT 0,
            name TEXT,
            payload_json TEXT
        );
        CREATE TABLE IF NOT EXISTS constraints (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            kind TEXT NOT NULL,
            expression TEXT NOT NULL,
            name TEXT,
            dim_offset REAL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS construction_planes (
            id INTEGER PRIMARY KEY,
            name TEXT,
            parent_kind TEXT NOT NULL,
            parent_id INTEGER,
            ox REAL NOT NULL, oy REAL NOT NULL, oz REAL NOT NULL,
            nx REAL NOT NULL, ny REAL NOT NULL, nz REAL NOT NULL,
            ux REAL NOT NULL, uy REAL NOT NULL, uz REAL NOT NULL,
            vx REAL NOT NULL, vy REAL NOT NULL, vz REAL NOT NULL,
            u_min REAL NOT NULL, u_max REAL NOT NULL,
            v_min REAL NOT NULL, v_max REAL NOT NULL,
            definition_json TEXT NOT NULL,
            repeat_json TEXT
        );
        CREATE TABLE IF NOT EXISTS extrusions (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            distance REAL NOT NULL,
            expression TEXT NOT NULL DEFAULT '',
            symmetric INTEGER NOT NULL DEFAULT 0,
            taper REAL NOT NULL DEFAULT 0,
            taper_mode TEXT NOT NULL DEFAULT 'distance',
            taper_expression TEXT NOT NULL DEFAULT '',
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bodies (
            id INTEGER PRIMARY KEY,
            source_kind TEXT NOT NULL,
            source_id INTEGER,
            material_id INTEGER,
            name TEXT,
            shadow INTEGER NOT NULL DEFAULT 0,
            source_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS materials (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            r INTEGER NOT NULL,
            g INTEGER NOT NULL,
            b INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS imported_meshes (
            id INTEGER PRIMARY KEY,
            source_name TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tracing_images (
            id INTEGER PRIMARY KEY,
            source_name TEXT NOT NULL,
            plane_id INTEGER,
            origin_u REAL NOT NULL,
            origin_v REAL NOT NULL,
            width_mm REAL NOT NULL,
            height_mm REAL NOT NULL,
            opacity REAL NOT NULL DEFAULT 0.9,
            name TEXT,
            base_origin_u REAL,
            base_origin_v REAL,
            calibration_json TEXT,
            rotation REAL NOT NULL DEFAULT 0,
            base_rotation REAL
        );
        CREATE TABLE IF NOT EXISTS lofts (
            id INTEGER PRIMARY KEY,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS revolutions (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            angle_deg REAL NOT NULL,
            pitch_mm REAL NOT NULL DEFAULT 0,
            symmetric INTEGER NOT NULL DEFAULT 0,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS primitives (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            ox REAL NOT NULL, oy REAL NOT NULL, oz REAL NOT NULL,
            nx REAL NOT NULL, ny REAL NOT NULL, nz REAL NOT NULL,
            ux REAL NOT NULL, uy REAL NOT NULL, uz REAL NOT NULL,
            width TEXT NOT NULL DEFAULT '',
            depth TEXT NOT NULL DEFAULT '',
            height TEXT NOT NULL DEFAULT '',
            radius TEXT NOT NULL DEFAULT '',
            -- Expressions driving ox/oy/oz (#1929); '' is a plain number.
            ox_expr TEXT NOT NULL DEFAULT '',
            oy_expr TEXT NOT NULL DEFAULT '',
            oz_expr TEXT NOT NULL DEFAULT '',
            name TEXT
        );
        CREATE TABLE IF NOT EXISTS sweeps (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS boolean_ops (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            keep_b INTEGER NOT NULL DEFAULT 0,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS move_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            keep_inputs INTEGER NOT NULL DEFAULT 0,
            translate_mode TEXT NOT NULL,
            tx TEXT NOT NULL DEFAULT '',
            ty TEXT NOT NULL DEFAULT '',
            tz TEXT NOT NULL DEFAULT '',
            rx TEXT NOT NULL DEFAULT '',
            ry TEXT NOT NULL DEFAULT '',
            rz TEXT NOT NULL DEFAULT '',
            roll_angle TEXT NOT NULL DEFAULT '',
            face_flip INTEGER NOT NULL DEFAULT 0,
            face_spin TEXT NOT NULL DEFAULT '',
            face_offset TEXT NOT NULL DEFAULT '',
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS mirror_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            mode TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS repeat_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            mode TEXT NOT NULL,
            count TEXT NOT NULL DEFAULT '',
            spacing TEXT NOT NULL DEFAULT '',
            length TEXT NOT NULL DEFAULT '',
            around_axis INTEGER NOT NULL DEFAULT 0,
            flip INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS slice_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            extend_infinite INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS shell_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            thickness TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS edge_treatment_ops (
            id INTEGER PRIMARY KEY,
            name TEXT,
            kind TEXT NOT NULL,
            amount REAL NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_repeat_ops (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            dir_u REAL NOT NULL,
            dir_v REAL NOT NULL,
            mode TEXT NOT NULL,
            count TEXT NOT NULL DEFAULT '',
            spacing TEXT NOT NULL DEFAULT '',
            length TEXT NOT NULL DEFAULT '',
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_offset_ops (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            distance TEXT NOT NULL DEFAULT '',
            construction INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_mirror_ops (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            line_id INTEGER,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_vertex_treatment_ops (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_slice_ops (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sketch_texts (
            id INTEGER PRIMARY KEY,
            sketch_id INTEGER,
            text TEXT NOT NULL,
            font_family TEXT NOT NULL,
            bold INTEGER NOT NULL DEFAULT 0,
            italic INTEGER NOT NULL DEFAULT 0,
            underline INTEGER NOT NULL DEFAULT 0,
            size REAL NOT NULL,
            size_expr TEXT NOT NULL DEFAULT '',
            origin_u REAL NOT NULL,
            origin_v REAL NOT NULL,
            rotation REAL NOT NULL DEFAULT 0,
            flip INTEGER NOT NULL DEFAULT 0,
            wrap_width REAL,
            baseline_line INTEGER,
            name TEXT,
            contours_json TEXT
        );
        CREATE TABLE IF NOT EXISTS drawings (
            id INTEGER PRIMARY KEY,
            name TEXT,
            page_width_mm REAL NOT NULL,
            page_height_mm REAL NOT NULL,
            margin_mm REAL NOT NULL,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS cross_sections (
            id INTEGER PRIMARY KEY,
            name TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS joints (
            id INTEGER PRIMARY KEY,
            name TEXT,
            kind TEXT NOT NULL,
            base INTEGER NOT NULL DEFAULT 0,
            position TEXT NOT NULL DEFAULT '',
            position2 TEXT NOT NULL DEFAULT '',
            position3 TEXT NOT NULL DEFAULT '',
            rest TEXT NOT NULL DEFAULT '',
            rest2 TEXT NOT NULL DEFAULT '',
            rest3 TEXT NOT NULL DEFAULT '',
            payload_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS units (
            id INTEGER PRIMARY KEY,
            source_json TEXT NOT NULL,
            link TEXT NOT NULL,
            source_mtime INTEGER,
            source_hash INTEGER,
            document BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS unit_instances (
            id INTEGER PRIMARY KEY,
            unit_id INTEGER,
            name TEXT,
            tx TEXT NOT NULL DEFAULT '',
            ty TEXT NOT NULL DEFAULT '',
            tz TEXT NOT NULL DEFAULT '',
            axis_x REAL NOT NULL DEFAULT 0,
            axis_y REAL NOT NULL DEFAULT 0,
            axis_z REAL NOT NULL DEFAULT 0,
            angle TEXT NOT NULL DEFAULT '',
            overrides_json TEXT
        );
        CREATE TABLE IF NOT EXISTS components (
            id INTEGER PRIMARY KEY,
            name TEXT,
            parent_id INTEGER,
            length_unit TEXT,
            angle_unit TEXT
        );
        CREATE TABLE IF NOT EXISTS component_members (
            member_kind TEXT NOT NULL,
            member_id INTEGER NOT NULL,
            component_id INTEGER NOT NULL,
            PRIMARY KEY (member_kind, member_id)
        );
        CREATE TABLE IF NOT EXISTS shape_order (
            seq INTEGER PRIMARY KEY,
            kind TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS undo_groups (
            seq INTEGER PRIMARY KEY,
            size INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS geometry_cache (
            body_id INTEGER PRIMARY KEY,
            fingerprint INTEGER NOT NULL,
            occt_version TEXT NOT NULL,
            mesh BLOB NOT NULL,
            brep BLOB
        );
        ",
    )?;
    Ok(())
}

fn key_bits<T>(k: Key<T>) -> i64 {
    k.to_bits() as i64
}

fn key_from<T>(bits: i64) -> Key<T> {
    Key::from_bits(bits as u64)
}

fn opt_key_bits<T>(k: Option<Key<T>>) -> Option<i64> {
    k.map(key_bits)
}

fn opt_key_from<T>(bits: Option<i64>) -> Option<Key<T>> {
    bits.map(key_from)
}

fn flag(v: bool) -> i64 {
    i64::from(v)
}

fn to_json<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}

fn from_json<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

fn from_json_or_default<T: DeserializeOwned + Default>(s: Option<&str>) -> Result<T> {
    match s {
        Some(s) if !s.is_empty() => from_json(s),
        _ => Ok(T::default()),
    }
}

fn unit_text<T: Serialize>(v: Option<T>) -> Result<Option<String>> {
    match v {
        Some(u) => Ok(Some(to_json(&u)?)),
        None => Ok(None),
    }
}

fn parse_unit<T: DeserializeOwned>(s: Option<String>) -> Result<Option<T>> {
    match s {
        Some(s) if !s.is_empty() => Ok(Some(from_json(&s)?)),
        _ => Ok(None),
    }
}

fn put_blob(tx: &Connection, id: i64, kind: &str, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT OR REPLACE INTO blobs (id, kind, bytes) VALUES (?1, ?2, ?3)",
        params![id, kind, bytes],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_entity_blobs(tx: &Connection, id: i64) -> Result<()> {
    tx.execute(
        "DELETE FROM blobs WHERE id = ?1 AND kind NOT IN ('preview_png')",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn get_blob(conn: &Connection, id: i64, kind: &str) -> Option<Vec<u8>> {
    conn.query_row(
        "SELECT bytes FROM blobs WHERE id = ?1 AND kind = ?2",
        params![id, kind],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn arena_from<T>(rows: Vec<(i64, T)>) -> Result<Arena<T>> {
    Arena::from_keyed(rows.into_iter().map(|(id, v)| (key_from(id), v)))
}

fn pack_triangles(tris: &[[glam::Vec3; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + tris.len() * 36);
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for tri in tris {
        for v in tri {
            out.extend_from_slice(&v.x.to_le_bytes());
            out.extend_from_slice(&v.y.to_le_bytes());
            out.extend_from_slice(&v.z.to_le_bytes());
        }
    }
    out
}

fn unpack_triangles(bytes: &[u8]) -> Result<Vec<[glam::Vec3; 3]>> {
    if bytes.len() < 4 {
        return Err("mesh_triangles blob too short".into());
    }
    let n = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let need = 4 + n * 36;
    if bytes.len() < need {
        return Err(format!(
            "mesh_triangles blob truncated: {} < {need}",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(n);
    let mut i = 4;
    for _ in 0..n {
        let mut tri = [glam::Vec3::ZERO; 3];
        for v in &mut tri {
            let x = f32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
            let y = f32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap());
            let z = f32::from_le_bytes(bytes[i + 8..i + 12].try_into().unwrap());
            *v = glam::Vec3::new(x, y, z);
            i += 12;
        }
        out.push(tri);
    }
    Ok(out)
}

fn sniff_sqlite(path: &str) -> Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 16];
    let n = f.read(&mut magic).map_err(|e| e.to_string())?;
    Ok(n >= 16 && magic.starts_with(b"SQLite format 3"))
}

/// Save `doc` to `path`, overwriting any existing document content.
pub fn save(path: &str, doc: &Document) -> Result<()> {
    validate_document_parameters_no_cycles(doc)?;
    if crate::storage::saves_as_json(path) {
        let bytes = crate::storage::to_json_bytes(doc)?;
        return std::fs::write(path, bytes).map_err(|e| e.to_string());
    }
    let tmp = format!("{path}.tmp");
    let _ = std::fs::remove_file(&tmp);
    match save_sqlite(&tmp, doc) {
        Ok(()) => std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn save_sqlite(path: &str, doc: &Document) -> Result<()> {
    let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
    // User documents must not grow WAL sidecars.
    conn.pragma_update(None, "journal_mode", "DELETE")
        .map_err(|e| e.to_string())?;
    init_schema(&conn).map_err(|e| e.to_string())?;
    let tx: Transaction<'_> = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations (id, name, applied_at)
         VALUES (?1, ?2, datetime('now'))",
        params![SCHEMA_VERSION, SCHEMA_MIGRATION_NAME],
    )
    .map_err(|e| e.to_string())?;

    put_meta(&tx, "app_version", APP_VERSION)?;
    put_meta(&tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    if let Some(ver) = crate::kernel::occt_version() {
        put_meta(&tx, "occt_version", &ver)?;
    }
    put_meta(
        &tx,
        DEFAULT_LENGTH_UNIT_META_KEY,
        &to_json(&doc.default_length_unit)?,
    )?;
    put_meta(
        &tx,
        DEFAULT_ANGLE_UNIT_META_KEY,
        &to_json(&doc.default_angle_unit)?,
    )?;

    save_all(&tx, doc)?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn save_all(tx: &Connection, doc: &Document) -> Result<()> {
    save_parameters(tx, &doc.parameters)?;
    save_sketches(tx, &doc.sketches)?;
    save_lines(tx, &doc.lines)?;
    save_circles(tx, &doc.circles)?;
    save_constraints(tx, &doc.constraints)?;
    save_planes(tx, &doc.construction_planes)?;
    save_extrusions(tx, &doc.extrusions)?;
    save_bodies(tx, &doc.bodies)?;
    save_materials(tx, &doc.materials)?;
    save_imported_meshes(tx, &doc.imported_meshes)?;
    save_tracing_images(tx, &doc.tracing_images)?;
    save_lofts(tx, &doc.lofts)?;
    save_revolutions(tx, &doc.revolutions)?;
    save_primitives(tx, &doc.primitives)?;
    save_sweeps(tx, &doc.sweeps)?;
    save_boolean_ops(tx, &doc.boolean_ops)?;
    save_move_ops(tx, &doc.move_ops)?;
    save_mirror_ops(tx, &doc.mirror_ops)?;
    save_repeat_ops(tx, &doc.repeat_ops)?;
    save_slice_ops(tx, &doc.slice_ops)?;
    save_shell_ops(tx, &doc.shell_ops)?;
    save_edge_treatment_ops(tx, &doc.edge_treatment_ops)?;
    save_sketch_repeat_ops(tx, &doc.sketch_repeat_ops)?;
    save_sketch_offset_ops(tx, &doc.sketch_offset_ops)?;
    save_sketch_mirror_ops(tx, &doc.sketch_mirror_ops)?;
    save_sketch_vertex_treatment_ops(tx, &doc.sketch_vertex_treatment_ops)?;
    save_sketch_slice_ops(tx, &doc.sketch_slice_ops)?;
    save_sketch_texts(tx, &doc.sketch_texts)?;
    save_drawings(tx, &doc.drawings)?;
    save_cross_sections(tx, &doc.cross_sections)?;
    save_joints(tx, &doc.joints)?;
    save_units(tx, &doc.units)?;
    save_unit_instances(tx, &doc.unit_instances)?;
    save_components(tx, &doc.components)?;
    save_component_members(tx, &doc.component_members)?;
    save_shape_order(tx, &doc.shape_order)?;
    save_undo_groups(tx, &doc.undo_groups)?;
    save_geometry_cache(tx, doc)?;
    Ok(())
}

fn fingerprint_sql(fp: u64) -> i64 {
    fp as i64
}

fn fingerprint_from_sql(fp: i64) -> u64 {
    fp as u64
}

fn upsert_geometry_cache_row(
    conn: &Connection,
    body_id: i64,
    fingerprint: u64,
    mesh: &[u8],
) -> Result<()> {
    conn.execute(
        "INSERT INTO geometry_cache (body_id, fingerprint, occt_version, mesh, brep)
         VALUES (?1, ?2, ?3, ?4, NULL)
         ON CONFLICT(body_id) DO UPDATE SET
           fingerprint = excluded.fingerprint,
           occt_version = excluded.occt_version,
           mesh = excluded.mesh,
           brep = excluded.brep",
        params![
            body_id,
            fingerprint_sql(fingerprint),
            crate::extrude::cache_occt_version(),
            mesh,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn save_geometry_cache(tx: &Connection, doc: &Document) -> Result<()> {
    let occt = crate::extrude::cache_occt_version();
    for committed in crate::extrude::committed_meshes() {
        if !doc.bodies.contains(committed.body) {
            continue;
        }
        let fp = crate::extrude::body_cache_fingerprint(doc, committed.body);
        if fp != committed.fingerprint {
            continue;
        }
        let bytes = pack_triangles(&committed.mesh.triangles);
        tx.execute(
            "INSERT INTO geometry_cache (body_id, fingerprint, occt_version, mesh, brep)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![
                key_bits(committed.body),
                fingerprint_sql(fp),
                occt,
                bytes,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn load_geometry_cache_rows(
    conn: &Connection,
) -> Result<Vec<(crate::model::BodyKey, u64, crate::extrude::SolidMesh)>> {
    let mut stmt = conn
        .prepare("SELECT body_id, fingerprint, occt_version, mesh FROM geometry_cache")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let want_occt = crate::extrude::cache_occt_version();
    let mut out = Vec::new();
    for row in rows {
        let (id, fp, occt, bytes) = row.map_err(|e| e.to_string())?;
        if occt != want_occt {
            continue;
        }
        let tris = unpack_triangles(&bytes)?;
        out.push((
            key_from(id),
            fingerprint_from_sql(fp),
            crate::extrude::SolidMesh { triangles: tris },
        ));
    }
    Ok(out)
}

fn load_geometry_cache_fingerprints(conn: &Connection) -> Result<BTreeMap<i64, u64>> {
    let mut stmt = conn
        .prepare("SELECT body_id, fingerprint FROM geometry_cache")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (id, fp) = row.map_err(|e| e.to_string())?;
        out.insert(id, fingerprint_from_sql(fp));
    }
    Ok(out)
}

fn put_meta(tx: &Connection, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn save_parameters(tx: &Connection, arena: &Arena<Parameter>) -> Result<()> {
    for (key, p) in arena.iter() {
        tx.execute(
            "INSERT INTO parameters (id, name, expression, is_primary, minimum, maximum, step, source_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                key_bits(key),
                p.name,
                p.expression,
                flag(p.primary),
                p.minimum,
                p.maximum,
                p.step,
                p.source.as_ref().map(to_json).transpose()?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketches(tx: &Connection, arena: &Arena<Sketch>) -> Result<()> {
    for (key, s) in arena.iter() {
        tx.execute(
            "INSERT INTO sketches (id, name, length_unit, angle_unit, face_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key_bits(key),
                s.name,
                unit_text(s.length_unit)?,
                unit_text(s.angle_unit)?,
                to_json(&s.face)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct LinePayload {
    #[serde(default)]
    bezier: Option<[(f32, f32); 2]>,
    #[serde(default)]
    chamfer_fillet_parent: Option<crate::model::LineKey>,
    #[serde(default)]
    projection: Option<crate::model::ProjectionSource>,
    #[serde(default)]
    seed: Option<crate::model::LineSeed>,
}

fn save_lines(tx: &Connection, arena: &Arena<Line>) -> Result<()> {
    for (key, l) in arena.iter() {
        let payload = LinePayload {
            bezier: l.bezier,
            chamfer_fillet_parent: l.chamfer_fillet_parent,
            projection: l.projection.clone(),
            seed: l.seed,
        };
        tx.execute(
            "INSERT INTO lines (id, sketch_id, x0, y0, x1, y1, construction, shadow,
             length_locked, length_expr, length_dim_offset, name, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                key_bits(key),
                key_bits(l.sketch),
                l.x0,
                l.y0,
                l.x1,
                l.y1,
                flag(l.construction),
                flag(l.shadow),
                flag(l.length_locked),
                l.length_expr,
                l.length_dim_offset,
                l.name,
                to_json(&payload)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CirclePayload {
    #[serde(default)]
    seed: Option<crate::model::CircleSeed>,
}

fn save_circles(tx: &Connection, arena: &Arena<Circle>) -> Result<()> {
    for (key, c) in arena.iter() {
        let payload = CirclePayload { seed: c.seed };
        tx.execute(
            "INSERT INTO circles (id, sketch_id, cx, cy, r, construction, shadow,
             diameter_locked, diameter_expr, diameter_dim_offset, diameter_dim_angle, name,
             payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                key_bits(key),
                key_bits(c.sketch),
                c.cx,
                c.cy,
                c.r,
                flag(c.construction),
                flag(c.shadow),
                flag(c.diameter_locked),
                c.diameter_expr,
                c.diameter_dim_offset,
                c.diameter_dim_angle,
                c.name,
                to_json(&payload)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn constraint_kind_tag(k: &ConstraintKind) -> &'static str {
    match k {
        ConstraintKind::Distance { .. } => "distance",
        ConstraintKind::Parallel { .. } => "parallel",
        ConstraintKind::Perpendicular { .. } => "perpendicular",
        ConstraintKind::Equal { .. } => "equal",
        ConstraintKind::Coincident { .. } => "coincident",
        ConstraintKind::Midpoint { .. } => "midpoint",
        ConstraintKind::Angle { .. } => "angle",
        ConstraintKind::Tangent { .. } => "tangent",
        ConstraintKind::TangentCircle { .. } => "tangent_circle",
    }
}

fn save_constraints(tx: &Connection, arena: &Arena<Constraint>) -> Result<()> {
    for (key, c) in arena.iter() {
        tx.execute(
            "INSERT INTO constraints (id, sketch_id, kind, expression, name, dim_offset, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key_bits(key),
                key_bits(c.sketch),
                constraint_kind_tag(&c.kind),
                c.expression,
                c.name,
                c.dim_offset,
                to_json(&c.kind)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_planes(tx: &Connection, arena: &Arena<ConstructionPlane>) -> Result<()> {
    for (key, p) in arena.iter() {
        let (parent_kind, parent_id) = match p.parent {
            ConstructionPlaneParent::Root => ("root", None),
            ConstructionPlaneParent::Sketch(s) => ("sketch", Some(key_bits(s))),
        };
        tx.execute(
            "INSERT INTO construction_planes (
                id, name, parent_kind, parent_id,
                ox, oy, oz, nx, ny, nz, ux, uy, uz, vx, vy, vz,
                u_min, u_max, v_min, v_max, definition_json, repeat_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                key_bits(key),
                p.name,
                parent_kind,
                parent_id,
                p.origin.x,
                p.origin.y,
                p.origin.z,
                p.normal.x,
                p.normal.y,
                p.normal.z,
                p.u_axis.x,
                p.u_axis.y,
                p.u_axis.z,
                p.v_axis.x,
                p.v_axis.y,
                p.v_axis.z,
                p.extent.u_min,
                p.extent.u_max,
                p.extent.v_min,
                p.extent.v_max,
                to_json(&p.definition)?,
                p.repeat_instance.as_ref().map(to_json).transpose()?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ExtrusionPayload {
    #[serde(default)]
    faces: Vec<crate::model::ExtrudeFace>,
    #[serde(default)]
    target: Option<crate::model::ExtrudeTarget>,
    #[serde(default)]
    edge_treatments: Vec<crate::model::EdgeTreatment>,
}

fn save_extrusions(tx: &Connection, arena: &Arena<Extrusion>) -> Result<()> {
    for (key, e) in arena.iter() {
        let payload = ExtrusionPayload {
            faces: e.faces.clone(),
            target: e.target.clone(),
            edge_treatments: e.edge_treatments.clone(),
        };
        tx.execute(
            "INSERT INTO extrusions (id, sketch_id, distance, expression, symmetric, taper,
             taper_mode, taper_expression, name, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                key_bits(key),
                key_bits(e.sketch),
                e.distance,
                e.expression,
                flag(e.symmetric),
                e.taper,
                e.taper_mode.as_str(),
                e.taper_expression,
                e.name,
                to_json(&payload)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn body_source_kind(src: &BodySource) -> &'static str {
    match src {
        BodySource::Extrusion(_) => "extrusion",
        BodySource::Extrusions(_) => "extrusions",
        BodySource::Imported(_) => "imported",
        BodySource::Loft(_) => "loft",
        BodySource::Revolve(_) => "revolve",
        BodySource::Primitive(_) => "primitive",
        BodySource::Sweep(_) => "sweep",
        BodySource::Repeated { .. } => "repeated",
        BodySource::Moved { .. } => "moved",
        BodySource::Mirrored { .. } => "mirrored",
        BodySource::Boolean { .. } => "boolean",
        BodySource::Sliced { .. } => "sliced",
        BodySource::Shelled { .. } => "shelled",
        BodySource::EdgeTreated { .. } => "edge_treated",
        BodySource::Solid { .. } => "solid",
        BodySource::UnitInstance(_) => "unit_instance",
        BodySource::UnitCut { .. } => "unit_cut",
        BodySource::Fused { .. } => "fused",
    }
}

fn body_source_id(src: &BodySource) -> Option<i64> {
    match src {
        BodySource::Extrusion(k) => Some(key_bits(*k)),
        BodySource::Imported(k) => Some(key_bits(*k)),
        BodySource::Loft(k) => Some(key_bits(*k)),
        BodySource::Revolve(k) => Some(key_bits(*k)),
        BodySource::Primitive(k) => Some(key_bits(*k)),
        BodySource::Sweep(k) => Some(key_bits(*k)),
        BodySource::Repeated { op, .. } => Some(key_bits(*op)),
        BodySource::Moved { op, .. } => Some(key_bits(*op)),
        BodySource::Mirrored { op, .. } => Some(key_bits(*op)),
        BodySource::Boolean { op, .. } => Some(key_bits(*op)),
        BodySource::Sliced { op, .. } => Some(key_bits(*op)),
        BodySource::Shelled { op, .. } => Some(key_bits(*op)),
        BodySource::EdgeTreated { op, .. } => Some(key_bits(*op)),
        BodySource::UnitInstance(k) => Some(key_bits(*k)),
        BodySource::UnitCut { instance, .. } => Some(key_bits(*instance)),
        BodySource::Extrusions(_) | BodySource::Solid { .. } | BodySource::Fused { .. } => None,
    }
}

fn save_bodies(tx: &Connection, arena: &Arena<Body>) -> Result<()> {
    for (key, b) in arena.iter() {
        tx.execute(
            "INSERT INTO bodies (id, source_kind, source_id, material_id, name, shadow, source_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key_bits(key),
                body_source_kind(&b.source),
                body_source_id(&b.source),
                opt_key_bits(b.material),
                b.name,
                flag(b.shadow),
                to_json(&b.source)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_materials(tx: &Connection, arena: &Arena<Material>) -> Result<()> {
    for (key, m) in arena.iter() {
        tx.execute(
            "INSERT INTO materials (id, name, r, g, b) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key_bits(key), m.name, m.color[0], m.color[1], m.color[2]],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_imported_meshes(tx: &Connection, arena: &Arena<ImportedMesh>) -> Result<()> {
    for (key, m) in arena.iter() {
        let id = key_bits(key);
        tx.execute(
            "INSERT INTO imported_meshes (id, source_name) VALUES (?1, ?2)",
            params![id, m.source_name],
        )
        .map_err(|e| e.to_string())?;
        put_blob(tx, id, BLOB_MESH_TRIANGLES, &pack_triangles(&m.triangles))?;
        if let Some(step) = &m.step_bytes {
            put_blob(tx, id, BLOB_STEP, step)?;
        }
    }
    Ok(())
}

fn save_tracing_images(tx: &Connection, arena: &Arena<TracingImage>) -> Result<()> {
    for (key, img) in arena.iter() {
        let id = key_bits(key);
        tx.execute(
            "INSERT INTO tracing_images (id, source_name, plane_id, origin_u, origin_v,
             width_mm, height_mm, opacity, name, base_origin_u, base_origin_v, calibration_json,
             rotation, base_rotation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id,
                img.source_name,
                key_bits(img.plane),
                img.origin.0,
                img.origin.1,
                img.width_mm,
                img.height_mm,
                img.opacity,
                img.name,
                img.base_origin.map(|o| o.0),
                img.base_origin.map(|o| o.1),
                img.calibration.as_ref().map(to_json).transpose()?,
                img.rotation as f64,
                img.base_rotation.map(|r| r as f64),
            ],
        )
        .map_err(|e| e.to_string())?;
        put_blob(tx, id, BLOB_TRACING_IMAGE, &img.bytes)?;
    }
    Ok(())
}

fn save_lofts(tx: &Connection, arena: &Arena<Loft>) -> Result<()> {
    for (key, l) in arena.iter() {
        tx.execute(
            "INSERT INTO lofts (id, name, payload_json) VALUES (?1, ?2, ?3)",
            params![
                key_bits(key),
                l.name,
                to_json(&serde_json::json!({ "sections": l.sections, "mode": l.mode }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_revolutions(tx: &Connection, arena: &Arena<Revolution>) -> Result<()> {
    for (key, r) in arena.iter() {
        tx.execute(
            "INSERT INTO revolutions (id, sketch_id, angle_deg, pitch_mm, symmetric, name, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key_bits(key),
                key_bits(r.sketch),
                r.angle_deg,
                r.pitch_mm,
                flag(r.symmetric),
                r.name,
                to_json(&serde_json::json!({
                    "faces": r.faces,
                    "axis": r.axis,
                    "mode": r.mode,
                    "angle_expression": r.angle_expression,
                    "angle_is_revolutions": r.angle_is_revolutions,
                    "pitch_expression": r.pitch_expression,
                    "gap_is_offset": r.gap_is_offset,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_primitives(tx: &Connection, arena: &Arena<Primitive>) -> Result<()> {
    for (key, p) in arena.iter() {
        tx.execute(
            "INSERT INTO primitives (id, kind, ox, oy, oz, nx, ny, nz, ux, uy, uz,
             width, depth, height, radius, ox_expr, oy_expr, oz_expr, name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19)",
            params![
                key_bits(key),
                p.kind.script_name(),
                p.origin[0],
                p.origin[1],
                p.origin[2],
                p.normal[0],
                p.normal[1],
                p.normal[2],
                p.u_axis[0],
                p.u_axis[1],
                p.u_axis[2],
                p.width,
                p.depth,
                p.height,
                p.radius,
                p.origin_expression[0],
                p.origin_expression[1],
                p.origin_expression[2],
                p.name,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sweeps(tx: &Connection, arena: &Arena<Sweep>) -> Result<()> {
    for (key, s) in arena.iter() {
        tx.execute(
            "INSERT INTO sweeps (id, sketch_id, name, payload_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                key_bits(s.sketch),
                s.name,
                to_json(&serde_json::json!({
                    "faces": s.faces,
                    "path": s.path,
                    "mode": s.mode,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_boolean_ops(tx: &Connection, arena: &Arena<BooleanOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO boolean_ops (id, kind, keep_b, name, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key_bits(key),
                to_json(&op.kind)?,
                flag(op.keep_b),
                op.name,
                to_json(&serde_json::json!({
                    "a": op.a,
                    "b": op.b,
                    "outputs": op.outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_move_ops(tx: &Connection, arena: &Arena<MoveOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO move_ops (id, name, keep_inputs, translate_mode, tx, ty, tz, rx, ry, rz,
             roll_angle, face_flip, face_spin, face_offset, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                key_bits(key),
                op.name,
                flag(op.keep_inputs),
                to_json(&op.translate_mode)?,
                op.tx,
                op.ty,
                op.tz,
                op.rx,
                op.ry,
                op.rz,
                op.roll_angle,
                flag(op.face_flip),
                op.face_spin,
                op.face_offset,
                to_json(&serde_json::json!({
                    "targets": op.targets,
                    "outputs": op.outputs,
                    "plane_targets": op.plane_targets,
                    "image_targets": op.image_targets,
                    "instance_targets": op.instance_targets,
                    "start_point_a": op.start_point_a,
                    "end_point_a": op.end_point_a,
                    "start_point_b": op.start_point_b,
                    "end_point_b": op.end_point_b,
                    "start_point_c": op.start_point_c,
                    "end_point_c": op.end_point_c,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_mirror_ops(tx: &Connection, arena: &Arena<MirrorOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO mirror_ops (id, name, mode, payload_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                op.name,
                to_json(&op.mode)?,
                to_json(&serde_json::json!({
                    "plane": op.plane,
                    "targets": op.targets,
                    "outputs": op.outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_repeat_ops(tx: &Connection, arena: &Arena<RepeatOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO repeat_ops (id, name, mode, count, spacing, length, around_axis, flip, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                key_bits(key),
                op.name,
                to_json(&op.mode)?,
                op.count,
                op.spacing,
                op.length,
                flag(op.around_axis),
                flag(op.flip),
                to_json(&serde_json::json!({
                    "targets": op.targets,
                    "plane_targets": op.plane_targets,
                    "extrusion_targets": op.extrusion_targets,
                    "sketch_targets": op.sketch_targets,
                    "sketch_plane_outputs": op.sketch_plane_outputs,
                    "sketch_outputs": op.sketch_outputs,
                    "axis": op.axis,
                    "path_circle": op.path_circle,
                    "length_target": op.length_target,
                    "outputs": op.outputs,
                    "plane_outputs": op.plane_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_slice_ops(tx: &Connection, arena: &Arena<SliceOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO slice_ops (id, name, extend_infinite, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                op.name,
                flag(op.extend_infinite),
                to_json(&serde_json::json!({
                    "targets": op.targets,
                    "cutters": op.cutters,
                    "outputs": op.outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_shell_ops(tx: &Connection, arena: &Arena<ShellOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO shell_ops (id, name, thickness, payload_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                op.name,
                op.thickness,
                to_json(&serde_json::json!({
                    "targets": op.targets,
                    "open_faces": op.open_faces,
                    "outputs": op.outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_edge_treatment_ops(tx: &Connection, arena: &Arena<EdgeTreatmentOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO edge_treatment_ops (id, name, kind, amount, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key_bits(key),
                op.name,
                to_json(&op.kind)?,
                op.amount,
                to_json(&serde_json::json!({
                    "targets": op.targets,
                    "edges": op.edges,
                    "outputs": op.outputs,
                    "expression": op.expression,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_repeat_ops(tx: &Connection, arena: &Arena<SketchRepeatOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO sketch_repeat_ops (id, sketch_id, name, dir_u, dir_v, mode, count, spacing, length, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                key_bits(key),
                key_bits(op.sketch),
                op.name,
                op.dir_u,
                op.dir_v,
                to_json(&op.mode)?,
                op.count,
                op.spacing,
                op.length,
                to_json(&serde_json::json!({
                    "line_targets": op.line_targets,
                    "circle_targets": op.circle_targets,
                    "line_outputs": op.line_outputs,
                    "circle_outputs": op.circle_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_offset_ops(tx: &Connection, arena: &Arena<SketchOffsetOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO sketch_offset_ops (id, sketch_id, name, distance, construction, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key_bits(key),
                key_bits(op.sketch),
                op.name,
                op.distance,
                flag(op.construction),
                to_json(&serde_json::json!({
                    "line_targets": op.line_targets,
                    "circle_targets": op.circle_targets,
                    "line_outputs": op.line_outputs,
                    "circle_outputs": op.circle_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_mirror_ops(tx: &Connection, arena: &Arena<SketchMirrorOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO sketch_mirror_ops (id, sketch_id, name, line_id, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key_bits(key),
                key_bits(op.sketch),
                op.name,
                op.line.as_line_key().map(key_bits).unwrap_or(0),
                to_json(&serde_json::json!({
                    "axis": op.line,
                    "line_targets": op.line_targets,
                    "circle_targets": op.circle_targets,
                    "line_outputs": op.line_outputs,
                    "circle_outputs": op.circle_outputs,
                    "constraint_outputs": op.constraint_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_vertex_treatment_ops(
    tx: &Connection,
    arena: &Arena<SketchVertexTreatmentOperation>,
) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO sketch_vertex_treatment_ops (id, sketch_id, name, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                key_bits(op.sketch),
                op.name,
                to_json(&serde_json::json!({
                    "line_targets": op.line_targets,
                    "corners": op.corners,
                    "line_outputs": op.line_outputs,
                    "bridge_outputs": op.bridge_outputs,
                    "constraint_outputs": op.constraint_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_slice_ops(tx: &Connection, arena: &Arena<SketchSliceOperation>) -> Result<()> {
    for (key, op) in arena.iter() {
        tx.execute(
            "INSERT INTO sketch_slice_ops (id, sketch_id, name, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                key_bits(key),
                key_bits(op.sketch),
                op.name,
                to_json(&serde_json::json!({
                    "line_targets": op.line_targets,
                    "cutter_lines": op.cutter_lines,
                    "circle_targets": op.circle_targets,
                    "face_targets": op.face_targets,
                    "line_outputs": op.line_outputs,
                    "constraint_outputs": op.constraint_outputs,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_sketch_texts(tx: &Connection, arena: &Arena<SketchText>) -> Result<()> {
    for (key, t) in arena.iter() {
        let id = key_bits(key);
        tx.execute(
            "INSERT INTO sketch_texts (id, sketch_id, text, font_family, bold, italic, underline,
             size, size_expr, origin_u, origin_v, rotation, flip, wrap_width, baseline_line, name, contours_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                id,
                key_bits(t.sketch),
                t.text,
                t.font_family,
                flag(t.bold),
                flag(t.italic),
                flag(t.underline),
                t.size,
                t.size_expr,
                t.origin.0,
                t.origin.1,
                t.rotation,
                flag(t.flip),
                t.wrap_width,
                t.baseline_line.map(|i| i as i64),
                t.name,
                to_json(&t.contours)?,
            ],
        )
        .map_err(|e| e.to_string())?;
        put_blob(tx, id, BLOB_FONT, &t.font_bytes)?;
    }
    Ok(())
}

fn save_drawings(tx: &Connection, arena: &Arena<Drawing>) -> Result<()> {
    for (key, d) in arena.iter() {
        tx.execute(
            "INSERT INTO drawings (id, name, page_width_mm, page_height_mm, margin_mm, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key_bits(key),
                d.name,
                d.page_width_mm,
                d.page_height_mm,
                d.margin_mm,
                to_json(&serde_json::json!({
                    "views": d.views,
                    "annotations": d.annotations,
                    "white_paper": d.white_paper,
                    "default_view_style": d.default_view_style,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_cross_sections(tx: &Connection, arena: &Arena<crate::model::CrossSection>) -> Result<()> {
    for (key, view) in arena.iter() {
        tx.execute(
            "INSERT INTO cross_sections (id, name, payload_json) VALUES (?1, ?2, ?3)",
            params![key_bits(key), view.name, to_json(&view.cuts)?],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn load_cross_sections(conn: &Connection) -> Result<Arena<crate::model::CrossSection>> {
    let mut stmt = conn
        .prepare("SELECT id, name, payload_json FROM cross_sections")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, payload_json) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            crate::model::CrossSection {
                name,
                cuts: from_json(&payload_json)?,
            },
        ));
    }
    arena_from(entries)
}

fn joint_kind_tag(k: &JointKind) -> &'static str {
    k.name()
}

fn save_joints(tx: &Connection, arena: &Arena<Joint>) -> Result<()> {
    for (key, j) in arena.iter() {
        tx.execute(
            "INSERT INTO joints (id, name, kind, base, position, position2, position3,
             rest, rest2, rest3, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                key_bits(key),
                j.name,
                joint_kind_tag(&j.kind),
                j.base as i64,
                j.position,
                j.position2,
                j.position3,
                j.rest,
                j.rest2,
                j.rest3,
                to_json(&serde_json::json!({
                    "kind": j.kind,
                    "members": j.members,
                    "placement": j.placement,
                    "frame": j.frame,
                    "limits": j.limits,
                }))?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_units(tx: &Connection, arena: &Arena<ImportedUnit>) -> Result<()> {
    for (key, u) in arena.iter() {
        tx.execute(
            "INSERT INTO units (id, source_json, link, source_mtime, source_hash, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key_bits(key),
                to_json(&u.source)?,
                to_json(&u.link)?,
                u.source_mtime,
                u.source_hash.map(|h| h as i64),
                document_to_blob(&u.document)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_unit_instances(tx: &Connection, arena: &Arena<UnitInstance>) -> Result<()> {
    for (key, inst) in arena.iter() {
        tx.execute(
            "INSERT INTO unit_instances (id, unit_id, name, tx, ty, tz, axis_x, axis_y, axis_z, angle, overrides_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                key_bits(key),
                key_bits(inst.unit),
                inst.name,
                inst.placement.tx,
                inst.placement.ty,
                inst.placement.tz,
                inst.placement.axis[0],
                inst.placement.axis[1],
                inst.placement.axis[2],
                inst.placement.angle,
                to_json(&inst.parameter_overrides)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_components(tx: &Connection, arena: &Arena<Component>) -> Result<()> {
    for (key, c) in arena.iter() {
        tx.execute(
            "INSERT INTO components (id, name, parent_id, length_unit, angle_unit)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                key_bits(key),
                c.name,
                opt_key_bits(c.parent),
                unit_text(c.length_unit)?,
                unit_text(c.angle_unit)?,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn member_kind_tag(m: ComponentMember) -> &'static str {
    match m {
        ComponentMember::ConstructionPlane(_) => "construction_plane",
        ComponentMember::Extrusion(_) => "extrusion",
        ComponentMember::Body(_) => "body",
        ComponentMember::Loft(_) => "loft",
        ComponentMember::BooleanOp(_) => "boolean_op",
        ComponentMember::MoveOp(_) => "move_op",
        ComponentMember::MirrorOp(_) => "mirror_op",
        ComponentMember::RepeatOp(_) => "repeat_op",
        ComponentMember::SliceOp(_) => "slice_op",
        ComponentMember::ShellOp(_) => "shell_op",
        ComponentMember::EdgeTreatmentOp(_) => "edge_treatment_op",
        ComponentMember::Revolution(_) => "revolution",
        ComponentMember::Sweep(_) => "sweep",
        ComponentMember::Drawing(_) => "drawing",
    }
}

fn member_id(m: ComponentMember) -> i64 {
    match m {
        ComponentMember::ConstructionPlane(k) => key_bits(k),
        ComponentMember::Extrusion(k) => key_bits(k),
        ComponentMember::Body(k) => key_bits(k),
        ComponentMember::Loft(k) => key_bits(k),
        ComponentMember::BooleanOp(k) => key_bits(k),
        ComponentMember::MoveOp(k) => key_bits(k),
        ComponentMember::MirrorOp(k) => key_bits(k),
        ComponentMember::RepeatOp(k) => key_bits(k),
        ComponentMember::SliceOp(k) => key_bits(k),
        ComponentMember::ShellOp(k) => key_bits(k),
        ComponentMember::EdgeTreatmentOp(k) => key_bits(k),
        ComponentMember::Revolution(k) => key_bits(k),
        ComponentMember::Sweep(k) => key_bits(k),
        ComponentMember::Drawing(k) => key_bits(k),
    }
}

fn save_component_members(
    tx: &Connection,
    members: &[(ComponentMember, crate::model::ComponentKey)],
) -> Result<()> {
    for &(member, component) in members {
        tx.execute(
            "INSERT INTO component_members (member_kind, member_id, component_id)
             VALUES (?1, ?2, ?3)",
            params![
                member_kind_tag(member),
                member_id(member),
                key_bits(component)
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_shape_order(tx: &Connection, order: &[ShapeKind]) -> Result<()> {
    for (seq, kind) in order.iter().enumerate() {
        tx.execute(
            "INSERT INTO shape_order (seq, kind) VALUES (?1, ?2)",
            params![seq as i64, to_json(kind)?],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn save_undo_groups(tx: &Connection, groups: &[usize]) -> Result<()> {
    for (seq, size) in groups.iter().enumerate() {
        tx.execute(
            "INSERT INTO undo_groups (seq, size) VALUES (?1, ?2)",
            params![seq as i64, *size as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Open the document stored at `path`.
pub fn open(path: &str) -> Result<Document> {
    if !sniff_sqlite(path)? {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let doc = super::from_json_bytes(&bytes)?;
        crate::model::validate_units(&doc, Some(std::path::Path::new(path)))?;
        return Ok(doc);
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    // CREATE TABLE IF NOT EXISTS — older v2 files gain geometry_cache.
    init_schema(&conn).map_err(|e| e.to_string())?;
    let version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(id), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if version > SCHEMA_VERSION {
        return Err(format!(
            "this file uses schema version {version}, newer than this BearCAD ({SCHEMA_VERSION})"
        ));
    }
    if version < SCHEMA_VERSION {
        return Err(format!(
            "this file uses the pre-typed dump schema (version {version}); re-save it from a matching BearCAD"
        ));
    }

    let mut doc = hydrate_document(&conn)?;
    super::fixup_loaded_document(&mut doc)?;
    crate::model::validate_units(&doc, Some(std::path::Path::new(path)))?;
    Ok(doc)
}

/// Hydrate arenas from a typed `.bearcad` connection. Nested unit blobs use this
/// without the top-level fixup (cycles / depth are checked on the outer document).
fn hydrate_document(conn: &Connection) -> Result<Document> {
    Ok(Document {
        parameters: load_parameters(conn)?,
        sketches: load_sketches(conn)?,
        lines: load_lines(conn)?,
        circles: load_circles(conn)?,
        constraints: load_constraints(conn)?,
        construction_planes: load_planes(conn)?,
        extrusions: load_extrusions(conn)?,
        bodies: load_bodies(conn)?,
        materials: load_materials(conn)?,
        imported_meshes: load_imported_meshes(conn)?,
        tracing_images: load_tracing_images(conn)?,
        lofts: load_lofts(conn)?,
        revolutions: load_revolutions(conn)?,
        primitives: load_primitives(conn)?,
        sweeps: load_sweeps(conn)?,
        boolean_ops: load_boolean_ops(conn)?,
        move_ops: load_move_ops(conn)?,
        mirror_ops: load_mirror_ops(conn)?,
        repeat_ops: load_repeat_ops(conn)?,
        slice_ops: load_slice_ops(conn)?,
        shell_ops: load_shell_ops(conn)?,
        edge_treatment_ops: load_edge_treatment_ops(conn)?,
        sketch_repeat_ops: load_sketch_repeat_ops(conn)?,
        sketch_offset_ops: load_sketch_offset_ops(conn)?,
        sketch_mirror_ops: load_sketch_mirror_ops(conn)?,
        sketch_vertex_treatment_ops: load_sketch_vertex_treatment_ops(conn)?,
        sketch_slice_ops: load_sketch_slice_ops(conn)?,
        sketch_texts: load_sketch_texts(conn)?,
        drawings: load_drawings(conn)?,
        cross_sections: load_cross_sections(conn)?,
        joints: load_joints(conn)?,
        shape_order: load_shape_order(conn)?,
        undo_groups: load_undo_groups(conn)?,
        default_length_unit: load_default_length_unit(conn),
        default_angle_unit: load_default_angle_unit(conn),
        components: load_components(conn)?,
        component_members: load_component_members(conn)?,
        units: load_units(conn)?,
        unit_instances: load_unit_instances(conn)?,
        mesh_rev: 0,
    })
}

fn unique_nested_blob_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "bearcad_nested_{}_{n}.bearcad",
        std::process::id()
    ))
}

/// Serialize `doc` as a standalone `.bearcad` (SQLite) byte blob.
fn document_to_blob(doc: &Document) -> Result<Vec<u8>> {
    let path = unique_nested_blob_path();
    let path_str = path.to_string_lossy().to_string();
    let result = save_sqlite(&path_str, doc)
        .and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()));
    let _ = std::fs::remove_file(&path);
    result
}

/// Hydrate a nested `.bearcad` blob into an in-memory `Document` (no top-level fixup).
fn document_from_blob(bytes: &[u8]) -> Result<Document> {
    if bytes.len() < 16 || !bytes.starts_with(b"SQLite format 3") {
        return Err("unit document is not a nested .bearcad blob".into());
    }
    let path = unique_nested_blob_path();
    let result = (|| {
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        hydrate_document(&conn)
    })();
    let _ = std::fs::remove_file(&path);
    result
}

fn load_default_length_unit(conn: &Connection) -> LengthUnit {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        [DEFAULT_LENGTH_UNIT_META_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .unwrap_or_default()
}

fn load_default_angle_unit(conn: &Connection) -> AngleUnit {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        [DEFAULT_ANGLE_UNIT_META_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .unwrap_or_default()
}

fn load_parameters(conn: &Connection) -> Result<Arena<Parameter>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, expression, is_primary, minimum, maximum, step, source_json
             FROM parameters",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, expression, primary, minimum, maximum, step, source_json) =
            row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Parameter {
                name,
                expression,
                primary: primary != 0,
                minimum,
                maximum,
                step,
                source: match source_json {
                    Some(s) if !s.is_empty() => Some(from_json(&s)?),
                    _ => None,
                },
            },
        ));
    }
    arena_from(entries)
}

fn load_sketches(conn: &Connection) -> Result<Arena<Sketch>> {
    let mut stmt = conn
        .prepare("SELECT id, name, length_unit, angle_unit, face_json FROM sketches")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, length_unit, angle_unit, face_json) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Sketch {
                face: from_json(&face_json)?,
                name,
                length_unit: parse_unit(length_unit)?,
                angle_unit: parse_unit(angle_unit)?,
            },
        ));
    }
    arena_from(entries)
}

fn load_lines(conn: &Connection) -> Result<Arena<Line>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, x0, y0, x1, y1, construction, shadow, length_locked,
                    length_expr, length_dim_offset, name, payload_json FROM lines",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<f64>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            sketch_id,
            x0,
            y0,
            x1,
            y1,
            construction,
            shadow,
            length_locked,
            length_expr,
            length_dim_offset,
            name,
            payload_json,
        ) = row.map_err(|e| e.to_string())?;
        let payload: LinePayload = from_json_or_default(payload_json.as_deref())?;
        entries.push((
            id,
            Line {
                sketch: key_from(sketch_id),
                x0: x0 as f32,
                y0: y0 as f32,
                x1: x1 as f32,
                y1: y1 as f32,
                seed: payload.seed,
                length_locked: length_locked != 0,
                length_dim_offset: length_dim_offset.map(|v| v as f32),
                length_expr,
                construction: construction != 0,
                shadow: shadow != 0,
                name,
                bezier: payload.bezier,
                chamfer_fillet_parent: payload.chamfer_fillet_parent,
                projection: payload.projection,
            },
        ));
    }
    arena_from(entries)
}

fn load_circles(conn: &Connection) -> Result<Arena<Circle>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, cx, cy, r, construction, shadow, diameter_locked,
                    diameter_expr, diameter_dim_offset, diameter_dim_angle, name, payload_json
                    FROM circles",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            sketch_id,
            cx,
            cy,
            r,
            construction,
            shadow,
            diameter_locked,
            diameter_expr,
            diameter_dim_offset,
            diameter_dim_angle,
            name,
            payload_json,
        ) = row.map_err(|e| e.to_string())?;
        let payload: CirclePayload = from_json_or_default(payload_json.as_deref())?;
        entries.push((
            id,
            Circle {
                sketch: key_from(sketch_id),
                cx: cx as f32,
                cy: cy as f32,
                r: r as f32,
                seed: payload.seed,
                diameter_locked: diameter_locked != 0,
                diameter_dim_offset: diameter_dim_offset.map(|v| v as f32),
                diameter_expr,
                diameter_dim_angle: diameter_dim_angle as f32,
                construction: construction != 0,
                shadow: shadow != 0,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_constraints(conn: &Connection) -> Result<Arena<Constraint>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, expression, name, dim_offset, payload_json FROM constraints",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, expression, name, dim_offset, payload_json) =
            row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Constraint {
                sketch: key_from(sketch_id),
                kind: from_json(&payload_json)?,
                expression,
                dim_offset: dim_offset.map(|v| v as f32),
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_planes(conn: &Connection) -> Result<Arena<ConstructionPlane>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, parent_kind, parent_id, ox, oy, oz, nx, ny, nz,
                    ux, uy, uz, vx, vy, vz, u_min, u_max, v_min, v_max,
                    definition_json, repeat_json FROM construction_planes",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, f64>(11)?,
                row.get::<_, f64>(12)?,
                row.get::<_, f64>(13)?,
                row.get::<_, f64>(14)?,
                row.get::<_, f64>(15)?,
                row.get::<_, f64>(16)?,
                row.get::<_, f64>(17)?,
                row.get::<_, f64>(18)?,
                row.get::<_, f64>(19)?,
                row.get::<_, String>(20)?,
                row.get::<_, Option<String>>(21)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            name,
            parent_kind,
            parent_id,
            ox,
            oy,
            oz,
            nx,
            ny,
            nz,
            ux,
            uy,
            uz,
            vx,
            vy,
            vz,
            u_min,
            u_max,
            v_min,
            v_max,
            definition_json,
            repeat_json,
        ) = row.map_err(|e| e.to_string())?;
        let parent = if parent_kind == "sketch" {
            ConstructionPlaneParent::Sketch(key_from(parent_id.unwrap_or(0)))
        } else {
            ConstructionPlaneParent::Root
        };
        entries.push((
            id,
            ConstructionPlane {
                origin: glam::Vec3::new(ox as f32, oy as f32, oz as f32),
                normal: glam::Vec3::new(nx as f32, ny as f32, nz as f32),
                u_axis: glam::Vec3::new(ux as f32, uy as f32, uz as f32),
                v_axis: glam::Vec3::new(vx as f32, vy as f32, vz as f32),
                parent,
                definition: from_json(&definition_json)?,
                repeat_instance: match repeat_json {
                    Some(s) if !s.is_empty() => Some(from_json(&s)?),
                    _ => None,
                },
                name,
                extent: crate::model::PlaneExtent {
                    u_min: u_min as f32,
                    u_max: u_max as f32,
                    v_min: v_min as f32,
                    v_max: v_max as f32,
                },
            },
        ));
    }
    arena_from(entries)
}

fn load_extrusions(conn: &Connection) -> Result<Arena<Extrusion>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, distance, expression, symmetric, taper, taper_mode,
                    taper_expression, name, payload_json FROM extrusions",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            sketch_id,
            distance,
            expression,
            symmetric,
            taper,
            taper_mode,
            taper_expression,
            name,
            payload_json,
        ) = row.map_err(|e| e.to_string())?;
        let payload: ExtrusionPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Extrusion {
                sketch: key_from(sketch_id),
                faces: payload.faces,
                distance: distance as f32,
                target: payload.target,
                expression,
                symmetric: symmetric != 0,
                taper: taper as f32,
                taper_mode: crate::model::ExtrudeTaperMode::from_name(&taper_mode)
                    .unwrap_or_default(),
                taper_expression,
                name,
                edge_treatments: payload.edge_treatments,
            },
        ));
    }
    arena_from(entries)
}

fn load_bodies(conn: &Connection) -> Result<Arena<Body>> {
    let mut stmt = conn
        .prepare("SELECT id, material_id, name, shadow, source_json FROM bodies")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, material_id, name, shadow, source_json) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Body {
                source: from_json(&source_json)?,
                name,
                material: opt_key_from(material_id),
                shadow: shadow != 0,
            },
        ));
    }
    arena_from(entries)
}

fn load_materials(conn: &Connection) -> Result<Arena<Material>> {
    let mut stmt = conn
        .prepare("SELECT id, name, r, g, b FROM materials")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, r, g, b) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Material {
                name,
                color: [r as u8, g as u8, b as u8],
            },
        ));
    }
    arena_from(entries)
}

fn load_imported_meshes(conn: &Connection) -> Result<Arena<ImportedMesh>> {
    let mut stmt = conn
        .prepare("SELECT id, source_name FROM imported_meshes")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, source_name) = row.map_err(|e| e.to_string())?;
        let triangles = match get_blob(conn, id, BLOB_MESH_TRIANGLES) {
            Some(bytes) => unpack_triangles(&bytes)?,
            None => Vec::new(),
        };
        entries.push((
            id,
            ImportedMesh {
                triangles,
                source_name,
                step_bytes: get_blob(conn, id, BLOB_STEP),
            },
        ));
    }
    arena_from(entries)
}

fn load_tracing_images(conn: &Connection) -> Result<Arena<TracingImage>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source_name, plane_id, origin_u, origin_v, width_mm, height_mm,
                    opacity, name, base_origin_u, base_origin_v, calibration_json,
                    rotation, base_rotation FROM tracing_images",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, Option<f64>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, f64>(12)?,
                row.get::<_, Option<f64>>(13)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            source_name,
            plane_id,
            origin_u,
            origin_v,
            width_mm,
            height_mm,
            opacity,
            name,
            base_origin_u,
            base_origin_v,
            calibration_json,
            rotation,
            base_rotation,
        ) = row.map_err(|e| e.to_string())?;
        let base_origin = match (base_origin_u, base_origin_v) {
            (Some(u), Some(v)) => Some((u as f32, v as f32)),
            _ => None,
        };
        entries.push((
            id,
            TracingImage {
                bytes: get_blob(conn, id, BLOB_TRACING_IMAGE).unwrap_or_default(),
                source_name,
                plane: key_from(plane_id),
                origin: (origin_u as f32, origin_v as f32),
                base_origin,
                width_mm: width_mm as f32,
                height_mm: height_mm as f32,
                opacity: opacity as f32,
                name,
                calibration: match calibration_json {
                    Some(s) if !s.is_empty() => Some(from_json(&s)?),
                    _ => None,
                },
                rotation: rotation as f32,
                base_rotation: base_rotation.map(|r| r as f32),
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct LoftPayload {
    #[serde(default)]
    sections: Vec<crate::model::LoftSection>,
    #[serde(default)]
    mode: crate::model::LoftMode,
}

fn load_lofts(conn: &Connection) -> Result<Arena<Loft>> {
    let mut stmt = conn
        .prepare("SELECT id, name, payload_json FROM lofts")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: LoftPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Loft {
                sections: payload.sections,
                mode: payload.mode,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct RevolutionPayload {
    #[serde(default)]
    faces: Vec<crate::model::ExtrudeFace>,
    axis: crate::model::RevolveAxis,
    mode: crate::model::RevolveMode,
    #[serde(default)]
    angle_expression: String,
    #[serde(default)]
    angle_is_revolutions: bool,
    #[serde(default)]
    pitch_expression: String,
    #[serde(default = "default_true")]
    gap_is_offset: bool,
}

fn load_revolutions(conn: &Connection) -> Result<Arena<Revolution>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, angle_deg, pitch_mm, symmetric, name, payload_json FROM revolutions",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, angle_deg, pitch_mm, symmetric, name, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: RevolutionPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Revolution {
                sketch: key_from(sketch_id),
                faces: payload.faces,
                axis: payload.axis,
                angle_deg: angle_deg as f32,
                angle_expression: payload.angle_expression,
                angle_is_revolutions: payload.angle_is_revolutions,
                pitch_mm: pitch_mm as f32,
                pitch_expression: payload.pitch_expression,
                gap_is_offset: payload.gap_is_offset,
                symmetric: symmetric != 0,
                mode: payload.mode,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_primitives(conn: &Connection) -> Result<Arena<Primitive>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, ox, oy, oz, nx, ny, nz, ux, uy, uz, width, depth, height, radius,
                    ox_expr, oy_expr, oz_expr, name
             FROM primitives",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
                (
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                ),
                row.get::<_, Option<String>>(18)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            kind,
            ox,
            oy,
            oz,
            nx,
            ny,
            nz,
            ux,
            uy,
            uz,
            width,
            depth,
            height,
            radius,
            origin_expression,
            name,
        ) = row.map_err(|e| e.to_string())?;
        let kind = crate::model::PrimitiveKind::from_name(&kind)
            .ok_or_else(|| format!("unknown primitive kind {kind}"))?;
        entries.push((
            id,
            Primitive {
                kind,
                origin: [ox as f32, oy as f32, oz as f32],
                origin_expression: [
                    origin_expression.0,
                    origin_expression.1,
                    origin_expression.2,
                ],
                normal: [nx as f32, ny as f32, nz as f32],
                u_axis: [ux as f32, uy as f32, uz as f32],
                width,
                depth,
                height,
                radius,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct SweepPayload {
    #[serde(default)]
    faces: Vec<crate::model::ExtrudeFace>,
    #[serde(default)]
    path: Vec<crate::model::LineKey>,
    mode: crate::model::SweepMode,
}

fn load_sweeps(conn: &Connection) -> Result<Arena<Sweep>> {
    let mut stmt = conn
        .prepare("SELECT id, sketch_id, name, payload_json FROM sweeps")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: SweepPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Sweep {
                sketch: key_from(sketch_id),
                faces: payload.faces,
                path: payload.path,
                mode: payload.mode,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct BooleanPayload {
    #[serde(default)]
    a: Vec<crate::model::BodyKey>,
    #[serde(default)]
    b: Vec<crate::model::BodyKey>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
}

fn load_boolean_ops(conn: &Connection) -> Result<Arena<BooleanOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, kind, keep_b, name, payload_json FROM boolean_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, kind, keep_b, name, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: BooleanPayload = from_json(&payload_json)?;
        entries.push((
            id,
            BooleanOperation {
                kind: from_json(&kind)?,
                a: payload.a,
                b: payload.b,
                keep_b: keep_b != 0,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct MovePayload {
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
    #[serde(default)]
    plane_targets: Vec<crate::model::ConstructionPlaneKey>,
    #[serde(default)]
    image_targets: Vec<crate::model::TracingImageKey>,
    #[serde(default)]
    instance_targets: Vec<crate::model::UnitInstanceKey>,
    #[serde(default)]
    start_point_a: Option<crate::model::MovePointRef>,
    #[serde(default)]
    end_point_a: Option<crate::model::MovePointRef>,
    #[serde(default)]
    start_point_b: Option<crate::model::MovePointRef>,
    #[serde(default)]
    end_point_b: Option<crate::model::MovePointRef>,
    #[serde(default)]
    start_point_c: Option<crate::model::MovePointRef>,
    #[serde(default)]
    end_point_c: Option<crate::model::MovePointRef>,
}

fn load_move_ops(conn: &Connection) -> Result<Arena<MoveOperation>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, keep_inputs, translate_mode, tx, ty, tz, rx, ry, rz,
                    roll_angle, face_flip, face_spin, face_offset, payload_json FROM move_ops",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            name,
            keep_inputs,
            translate_mode,
            tx,
            ty,
            tz,
            rx,
            ry,
            rz,
            roll_angle,
            face_flip,
            face_spin,
            face_offset,
            payload_json,
        ) = row.map_err(|e| e.to_string())?;
        let payload: MovePayload = from_json(&payload_json)?;
        entries.push((
            id,
            MoveOperation {
                targets: payload.targets,
                keep_inputs: keep_inputs != 0,
                translate_mode: from_json(&translate_mode)?,
                start_point_a: payload.start_point_a,
                end_point_a: payload.end_point_a,
                start_point_b: payload.start_point_b,
                end_point_b: payload.end_point_b,
                start_point_c: payload.start_point_c,
                end_point_c: payload.end_point_c,
                plane_targets: payload.plane_targets,
                image_targets: payload.image_targets,
                instance_targets: payload.instance_targets,
                tx,
                ty,
                tz,
                roll_angle,
                face_flip: face_flip != 0,
                face_spin,
                face_offset,
                rx,
                ry,
                rz,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct MirrorPayload {
    plane: crate::model::FaceId,
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
}

fn load_mirror_ops(conn: &Connection) -> Result<Arena<MirrorOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, name, mode, payload_json FROM mirror_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, mode, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: MirrorPayload = from_json(&payload_json)?;
        entries.push((
            id,
            MirrorOperation {
                plane: payload.plane,
                targets: payload.targets,
                mode: from_json(&mode)?,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct RepeatPayload {
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    plane_targets: Vec<crate::model::ConstructionPlaneKey>,
    #[serde(default)]
    extrusion_targets: Vec<crate::model::ExtrusionKey>,
    #[serde(default)]
    sketch_targets: Vec<crate::model::SketchId>,
    #[serde(default)]
    sketch_plane_outputs: Vec<crate::model::ConstructionPlaneKey>,
    #[serde(default)]
    sketch_outputs: Vec<crate::model::SketchId>,
    axis: crate::model::RevolveAxis,
    #[serde(default)]
    path_circle: Option<crate::model::CircleKey>,
    #[serde(default)]
    length_target: Option<crate::model::ExtrudeTarget>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
    #[serde(default)]
    plane_outputs: Vec<crate::model::ConstructionPlaneKey>,
}

fn load_repeat_ops(conn: &Connection) -> Result<Arena<RepeatOperation>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, mode, count, spacing, length, around_axis, flip, payload_json
             FROM repeat_ops",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, mode, count, spacing, length, around_axis, flip, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: RepeatPayload = from_json(&payload_json)?;
        entries.push((
            id,
            RepeatOperation {
                targets: payload.targets,
                plane_targets: payload.plane_targets,
                extrusion_targets: payload.extrusion_targets,
                sketch_targets: payload.sketch_targets,
                sketch_plane_outputs: payload.sketch_plane_outputs,
                sketch_outputs: payload.sketch_outputs,
                axis: payload.axis,
                path_circle: payload.path_circle,
                around_axis: around_axis != 0,
                flip: flip != 0,
                mode: from_json(&mode)?,
                count,
                spacing,
                length,
                length_target: payload.length_target,
                outputs: payload.outputs,
                plane_outputs: payload.plane_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct SlicePayload {
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    cutters: Vec<crate::model::SliceCutter>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
}

fn load_slice_ops(conn: &Connection) -> Result<Arena<SliceOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, name, extend_infinite, payload_json FROM slice_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, extend_infinite, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: SlicePayload = from_json(&payload_json)?;
        entries.push((
            id,
            SliceOperation {
                targets: payload.targets,
                cutters: payload.cutters,
                extend_infinite: extend_infinite != 0,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct ShellPayload {
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    open_faces: Vec<crate::model::FaceId>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
}

fn load_shell_ops(conn: &Connection) -> Result<Arena<ShellOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, name, thickness, payload_json FROM shell_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, thickness, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: ShellPayload = from_json(&payload_json)?;
        entries.push((
            id,
            ShellOperation {
                targets: payload.targets,
                open_faces: payload.open_faces,
                thickness,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct EdgeTreatmentPayload {
    #[serde(default)]
    targets: Vec<crate::model::BodyKey>,
    #[serde(default)]
    edges: Vec<crate::model::TreatedEdge>,
    #[serde(default)]
    outputs: Vec<crate::model::BodyKey>,
    #[serde(default)]
    expression: String,
}

fn load_edge_treatment_ops(conn: &Connection) -> Result<Arena<EdgeTreatmentOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, name, kind, amount, payload_json FROM edge_treatment_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, kind, amount, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: EdgeTreatmentPayload = from_json(&payload_json)?;
        entries.push((
            id,
            EdgeTreatmentOperation {
                targets: payload.targets,
                edges: payload.edges,
                kind: from_json(&kind)?,
                amount: amount as f32,
                expression: payload.expression,
                outputs: payload.outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct SketchGeomPayload {
    #[serde(default)]
    axis: Option<crate::model::SketchMirrorAxis>,
    #[serde(default)]
    line_targets: Vec<crate::model::LineKey>,
    #[serde(default)]
    circle_targets: Vec<crate::model::CircleKey>,
    #[serde(default)]
    line_outputs: Vec<crate::model::LineKey>,
    #[serde(default)]
    circle_outputs: Vec<crate::model::CircleKey>,
    #[serde(default)]
    constraint_outputs: Vec<crate::model::ConstraintKey>,
}

fn load_sketch_repeat_ops(conn: &Connection) -> Result<Arena<SketchRepeatOperation>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, name, dir_u, dir_v, mode, count, spacing, length, payload_json
             FROM sketch_repeat_ops",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, dir_u, dir_v, mode, count, spacing, length, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: SketchGeomPayload = from_json(&payload_json)?;
        entries.push((
            id,
            SketchRepeatOperation {
                sketch: key_from(sketch_id),
                line_targets: payload.line_targets,
                circle_targets: payload.circle_targets,
                dir_u: dir_u as f32,
                dir_v: dir_v as f32,
                mode: from_json(&mode)?,
                count,
                spacing,
                length,
                line_outputs: payload.line_outputs,
                circle_outputs: payload.circle_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_sketch_offset_ops(conn: &Connection) -> Result<Arena<SketchOffsetOperation>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, name, distance, construction, payload_json FROM sketch_offset_ops",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, distance, construction, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: SketchGeomPayload = from_json(&payload_json)?;
        entries.push((
            id,
            SketchOffsetOperation {
                sketch: key_from(sketch_id),
                line_targets: payload.line_targets,
                circle_targets: payload.circle_targets,
                distance,
                construction: construction != 0,
                line_outputs: payload.line_outputs,
                circle_outputs: payload.circle_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_sketch_mirror_ops(conn: &Connection) -> Result<Arena<SketchMirrorOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, sketch_id, name, line_id, payload_json FROM sketch_mirror_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, line_id, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: SketchGeomPayload = from_json(&payload_json)?;
        entries.push((
            id,
            SketchMirrorOperation {
                sketch: key_from(sketch_id),
                line: payload
                    .axis
                    .unwrap_or(crate::model::SketchMirrorAxis::Line(key_from(line_id))),
                line_targets: payload.line_targets,
                circle_targets: payload.circle_targets,
                line_outputs: payload.line_outputs,
                circle_outputs: payload.circle_outputs,
                constraint_outputs: payload.constraint_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct VertexTreatmentPayload {
    #[serde(default)]
    line_targets: Vec<crate::model::LineKey>,
    #[serde(default)]
    corners: Vec<crate::model::SketchVertexTreatmentCorner>,
    #[serde(default)]
    line_outputs: Vec<crate::model::LineKey>,
    #[serde(default)]
    bridge_outputs: Vec<crate::model::LineKey>,
    #[serde(default)]
    constraint_outputs: Vec<crate::model::ConstraintKey>,
}

fn load_sketch_vertex_treatment_ops(
    conn: &Connection,
) -> Result<Arena<SketchVertexTreatmentOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, sketch_id, name, payload_json FROM sketch_vertex_treatment_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: VertexTreatmentPayload = from_json(&payload_json)?;
        entries.push((
            id,
            SketchVertexTreatmentOperation {
                sketch: key_from(sketch_id),
                line_targets: payload.line_targets,
                corners: payload.corners,
                line_outputs: payload.line_outputs,
                bridge_outputs: payload.bridge_outputs,
                constraint_outputs: payload.constraint_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct SketchSlicePayload {
    #[serde(default)]
    line_targets: Vec<crate::model::LineKey>,
    #[serde(default)]
    cutter_lines: Vec<crate::model::LineKey>,
    #[serde(default)]
    circle_targets: Vec<crate::model::CircleKey>,
    #[serde(default)]
    face_targets: Vec<Vec<crate::model::LineKey>>,
    #[serde(default)]
    line_outputs: Vec<crate::model::LineKey>,
    #[serde(default)]
    constraint_outputs: Vec<crate::model::ConstraintKey>,
}

fn load_sketch_slice_ops(conn: &Connection) -> Result<Arena<SketchSliceOperation>> {
    let mut stmt = conn
        .prepare("SELECT id, sketch_id, name, payload_json FROM sketch_slice_ops")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, sketch_id, name, payload_json) = row.map_err(|e| e.to_string())?;
        let payload: SketchSlicePayload = from_json(&payload_json)?;
        entries.push((
            id,
            SketchSliceOperation {
                sketch: key_from(sketch_id),
                line_targets: payload.line_targets,
                cutter_lines: payload.cutter_lines,
                circle_targets: payload.circle_targets,
                face_targets: payload.face_targets,
                line_outputs: payload.line_outputs,
                constraint_outputs: payload.constraint_outputs,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_sketch_texts(conn: &Connection) -> Result<Arena<SketchText>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sketch_id, text, font_family, bold, italic, underline, size, size_expr,
                    origin_u, origin_v, rotation, flip, wrap_width, baseline_line, name, contours_json
             FROM sketch_texts",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, f64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<f64>>(13)?,
                row.get::<_, Option<i64>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            sketch_id,
            text,
            font_family,
            bold,
            italic,
            underline,
            size,
            size_expr,
            origin_u,
            origin_v,
            rotation,
            flip,
            wrap_width,
            baseline_line,
            name,
            contours_json,
        ) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            SketchText {
                sketch: key_from(sketch_id),
                text,
                font_family,
                bold: bold != 0,
                italic: italic != 0,
                underline: underline != 0,
                size: size as f32,
                size_expr,
                origin: (origin_u as f32, origin_v as f32),
                rotation: rotation as f32,
                flip: flip != 0,
                wrap_width: wrap_width.map(|v| v as f32),
                baseline_line: baseline_line.map(|v| v as usize),
                contours: from_json_or_default(contours_json.as_deref())?,
                font_bytes: get_blob(conn, id, BLOB_FONT).unwrap_or_default(),
                pin: None,
                name,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize, Default)]
struct DrawingPayload {
    #[serde(default)]
    views: Vec<crate::model::DrawingView>,
    #[serde(default)]
    annotations: crate::arena::Arena<crate::model::DrawingAnnotation>,
    #[serde(default)]
    white_paper: bool,
    #[serde(default)]
    default_view_style: crate::model::DrawingViewStyle,
}

fn load_drawings(conn: &Connection) -> Result<Arena<Drawing>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, page_width_mm, page_height_mm, margin_mm, payload_json FROM drawings",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, page_width_mm, page_height_mm, margin_mm, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: DrawingPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Drawing {
                name,
                views: payload.views,
                page_width_mm: page_width_mm as f32,
                page_height_mm: page_height_mm as f32,
                margin_mm: margin_mm as f32,
                annotations: payload.annotations,
                white_paper: payload.white_paper,
                default_view_style: payload.default_view_style,
            },
        ));
    }
    arena_from(entries)
}

#[derive(serde::Deserialize)]
struct JointPayload {
    kind: JointKind,
    #[serde(default)]
    members: Vec<crate::model::JointRef>,
    #[serde(default)]
    placement: crate::model::MoveOperation,
    #[serde(default)]
    frame: crate::model::JointFrame,
    #[serde(default)]
    limits: crate::model::JointLimits,
}

fn load_joints(conn: &Connection) -> Result<Arena<Joint>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, base, position, position2, position3, rest, rest2, rest3, payload_json
             FROM joints",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, base, position, position2, position3, rest, rest2, rest3, payload_json) =
            row.map_err(|e| e.to_string())?;
        let payload: JointPayload = from_json(&payload_json)?;
        entries.push((
            id,
            Joint {
                members: payload.members,
                base: base as usize,
                kind: payload.kind,
                placement: payload.placement,
                frame: payload.frame,
                position,
                position2,
                position3,
                rest,
                rest2,
                rest3,
                limits: payload.limits,
                name,
            },
        ));
    }
    arena_from(entries)
}

fn load_units(conn: &Connection) -> Result<Arena<ImportedUnit>> {
    let mut stmt = conn
        .prepare("SELECT id, source_json, link, source_mtime, source_hash, document FROM units")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, source_json, link, source_mtime, source_hash, document) =
            row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            ImportedUnit {
                source: from_json(&source_json)?,
                link: from_json(&link).unwrap_or(LinkMode::Static),
                document: document_from_blob(&document)?,
                source_mtime,
                source_hash: source_hash.map(|h| h as u64),
            },
        ));
    }
    arena_from(entries)
}

fn load_unit_instances(conn: &Connection) -> Result<Arena<UnitInstance>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, unit_id, name, tx, ty, tz, axis_x, axis_y, axis_z, angle, overrides_json
             FROM unit_instances",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, unit_id, name, tx, ty, tz, axis_x, axis_y, axis_z, angle, overrides_json) =
            row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            UnitInstance {
                unit: key_from(unit_id),
                name,
                parameter_overrides: from_json_or_default(overrides_json.as_deref())?,
                placement: crate::model::UnitPlacement {
                    tx,
                    ty,
                    tz,
                    axis: [axis_x as f32, axis_y as f32, axis_z as f32],
                    angle,
                },
            },
        ));
    }
    arena_from(entries)
}

fn load_components(conn: &Connection) -> Result<Arena<Component>> {
    let mut stmt = conn
        .prepare("SELECT id, name, parent_id, length_unit, angle_unit FROM components")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, name, parent_id, length_unit, angle_unit) = row.map_err(|e| e.to_string())?;
        entries.push((
            id,
            Component {
                name,
                parent: opt_key_from(parent_id),
                length_unit: parse_unit(length_unit)?,
                angle_unit: parse_unit(angle_unit)?,
            },
        ));
    }
    arena_from(entries)
}

fn component_member_from(kind: &str, id: i64) -> Result<ComponentMember> {
    Ok(match kind {
        "construction_plane" => ComponentMember::ConstructionPlane(key_from(id)),
        "extrusion" => ComponentMember::Extrusion(key_from(id)),
        "body" => ComponentMember::Body(key_from(id)),
        "loft" => ComponentMember::Loft(key_from(id)),
        "boolean_op" => ComponentMember::BooleanOp(key_from(id)),
        "move_op" => ComponentMember::MoveOp(key_from(id)),
        "mirror_op" => ComponentMember::MirrorOp(key_from(id)),
        "repeat_op" => ComponentMember::RepeatOp(key_from(id)),
        "slice_op" => ComponentMember::SliceOp(key_from(id)),
        "shell_op" => ComponentMember::ShellOp(key_from(id)),
        "edge_treatment_op" => ComponentMember::EdgeTreatmentOp(key_from(id)),
        "revolution" => ComponentMember::Revolution(key_from(id)),
        "sweep" => ComponentMember::Sweep(key_from(id)),
        "drawing" => ComponentMember::Drawing(key_from(id)),
        other => return Err(format!("unknown component member kind {other}")),
    })
}

fn load_component_members(
    conn: &Connection,
) -> Result<Vec<(ComponentMember, crate::model::ComponentKey)>> {
    let mut stmt = conn
        .prepare("SELECT member_kind, member_id, component_id FROM component_members")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, member_id, component_id) = row.map_err(|e| e.to_string())?;
        out.push((
            component_member_from(&kind, member_id)?,
            key_from(component_id),
        ));
    }
    Ok(out)
}

fn load_shape_order(conn: &Connection) -> Result<Vec<ShapeKind>> {
    let mut stmt = conn
        .prepare("SELECT kind FROM shape_order ORDER BY seq")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let kind = row.map_err(|e| e.to_string())?;
        out.push(from_json(&kind)?);
    }
    Ok(out)
}

fn load_undo_groups(conn: &Connection) -> Result<Vec<usize>> {
    let mut stmt = conn
        .prepare("SELECT size FROM undo_groups ORDER BY seq")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())? as usize);
    }
    Ok(out)
}

/// Write or replace a document-level blob (preview PNG/STL) after save.
pub fn upsert_preview_blob(path: &str, kind: &str, bytes: &[u8]) -> Result<()> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO blobs (id, kind, bytes) VALUES (?1, ?2, ?3)",
        params![PREVIEW_BLOB_ID, kind, bytes],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop a document-level blob by kind.
pub fn delete_preview_blob(path: &str, kind: &str) -> Result<()> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM blobs WHERE id = ?1 AND kind = ?2",
        params![PREVIEW_BLOB_ID, kind],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read a document-level blob by kind.
#[cfg(test)]
pub fn load_preview_blob(path: &str, kind: &str) -> Option<Vec<u8>> {
    let conn = Connection::open(path).ok()?;
    get_blob(&conn, PREVIEW_BLOB_ID, kind)
}

include!("storage_session.rs");
