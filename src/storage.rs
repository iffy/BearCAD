//! `.bearcad` file persistence (SPEC §7).
//!
//! A `.bearcad` is a SQLite database. This early version implements only a small
//! part of the schema from the spec — enough to round-trip sketch primitives —
//! but keeps the pieces that matter for forward compatibility: a `meta` table
//! and a `schema_migrations` table, and shapes stored as DAG nodes with a
//! JSON payload (SPEC §7.3). When real features arrive they slot into the same
//! `dag_nodes` shape.


use crate::face::default_xy_plane;
use crate::model::{Document, FaceId};

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
    ensure_construction_plane_indices(doc);
    crate::constraints::migrate_legacy_dimensions(doc);
    migrate_text_pins(doc);
    crate::constraints::solve_document_constraints(doc).map_err(|e| e.to_string())?;
    Ok(())
}

/// Convert legacy text position pins (#356) into `Coincident` constraints between the text's
/// anchor point and the pin target (#408), so old documents keep their behaviour under the
/// constraint solver. The pin field is cleared and never written back.
fn migrate_text_pins(doc: &mut Document) {
    for i in 0..doc.sketch_texts.len() {
        let Some((point, anchor)) = doc.sketch_texts[i].pin.take() else {
            continue;
        };
        if doc.sketch_texts[i].deleted {
            continue;
        }
        doc.constraints.push(crate::model::Constraint {
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
            deleted: false,
        });
        doc.shape_order.push(crate::model::ShapeKind::Constraint);
    }
}


fn ensure_construction_plane_indices(doc: &mut Document) {
    if doc.construction_planes.is_empty() {
        doc.construction_planes.push(default_xy_plane());
    }
    let max_index = doc
        .sketches
        .iter()
        .filter_map(|sketch| match sketch.face {
            FaceId::ConstructionPlane(index) => Some(index),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    while doc.construction_planes.len() <= max_index {
        doc.construction_planes.push(default_xy_plane());
    }
}

/// The SQLite `.bearcad` format — native builds only (the bundled SQLite C library
/// doesn't compile for wasm32-unknown-unknown).
#[cfg(not(target_arch = "wasm32"))]
mod sqlite_format {
use crate::face::default_xy_plane;
use crate::model::{
    Circle, ConstructionPlane, Constraint, Document, Line, Parameter, ShapeKind,
    Sketch,
};
use crate::parameters::validate_document_parameters_no_cycles;
use crate::value::{AngleUnit, LengthUnit};
use rusqlite::Connection;

/// Bump when the on-disk schema changes; pair with a migration below.
const SCHEMA_VERSION: i64 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONSTRUCTION_PLANES_META_KEY: &str = "construction_planes";
const SHAPE_ORDER_META_KEY: &str = "shape_order";
/// Undo-group sizes (#105); files saved before this key existed load with none and
/// are reconciled into per-entry groups on the first action.
const UNDO_GROUPS_META_KEY: &str = "undo_groups";
/// Components (#423) and their membership, stored as meta JSON like construction planes.
const COMPONENTS_META_KEY: &str = "components";
const COMPONENT_MEMBERS_META_KEY: &str = "component_members";
/// Document-level default length unit (#52); missing for files saved before this change,
/// which fall back to [`LengthUnit::default`] (mm), matching their pre-existing behaviour.
const DEFAULT_LENGTH_UNIT_META_KEY: &str = "default_length_unit";
/// Document-level default angle unit (#52); missing for files saved before this change,
/// which fall back to [`AngleUnit::default`] (deg), matching their pre-existing behaviour.
const DEFAULT_ANGLE_UNIT_META_KEY: &str = "default_angle_unit";

use super::Result;

/// Create the tables for a fresh database (idempotent).
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
            value TEXT
        );
        CREATE TABLE IF NOT EXISTS dag_nodes (
            id           INTEGER PRIMARY KEY,
            component_id INTEGER,
            kind         TEXT NOT NULL,
            payload      TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// Save `doc` to `path`, overwriting any existing document content.
pub fn save(path: &str, doc: &Document) -> Result<()> {
    validate_document_parameters_no_cycles(doc)?;
    // A `.json` path saves the web build's JSON codec instead of SQLite — the format the
    // web app's `?open=<url>` fetches, so a docs scene can publish a loadable document.
    if path.ends_with(".json") {
        let bytes = crate::storage::to_json_bytes(doc)?;
        return std::fs::write(path, bytes).map_err(|e| e.to_string());
    }
    let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
    init_schema(&conn).map_err(|e| e.to_string())?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations (id, name, applied_at)
         VALUES (?1, 'initial', datetime('now'))",
        rusqlite::params![SCHEMA_VERSION],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('app_version', ?1)",
        rusqlite::params![APP_VERSION],
    )
    .map_err(|e| e.to_string())?;

    let planes_payload =
        serde_json::to_string(&doc.construction_planes).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![CONSTRUCTION_PLANES_META_KEY, planes_payload],
    )
    .map_err(|e| e.to_string())?;

    let shape_order_payload =
        serde_json::to_string(&doc.shape_order).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![SHAPE_ORDER_META_KEY, shape_order_payload],
    )
    .map_err(|e| e.to_string())?;

    let undo_groups_payload =
        serde_json::to_string(&doc.undo_groups).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![UNDO_GROUPS_META_KEY, undo_groups_payload],
    )
    .map_err(|e| e.to_string())?;

    let default_length_unit_payload =
        serde_json::to_string(&doc.default_length_unit).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![DEFAULT_LENGTH_UNIT_META_KEY, default_length_unit_payload],
    )
    .map_err(|e| e.to_string())?;

    let default_angle_unit_payload =
        serde_json::to_string(&doc.default_angle_unit).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![DEFAULT_ANGLE_UNIT_META_KEY, default_angle_unit_payload],
    )
    .map_err(|e| e.to_string())?;

    let components_payload = serde_json::to_string(&doc.components).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![COMPONENTS_META_KEY, components_payload],
    )
    .map_err(|e| e.to_string())?;

    let component_members_payload =
        serde_json::to_string(&doc.component_members).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![COMPONENT_MEMBERS_META_KEY, component_members_payload],
    )
    .map_err(|e| e.to_string())?;

    // Every kind is re-inserted below, so clear the whole table: the old hardcoded kind
    // list silently omitted newer kinds (boolean_op, move_op, sweep, ...), which then
    // accumulated duplicate rows on every in-place save and loaded back duplicated.
    tx.execute("DELETE FROM dag_nodes", [])
        .map_err(|e| e.to_string())?;

    let mut row_id = 0i64;
    save_indexed_nodes(&tx, &mut row_id, "sketch", &doc.sketches)?;
    save_indexed_nodes(&tx, &mut row_id, "line", &doc.lines)?;
    save_indexed_nodes(&tx, &mut row_id, "circle", &doc.circles)?;
    save_arena_nodes(&tx, &mut row_id, "parameter", &doc.parameters)?;
    save_indexed_nodes(&tx, &mut row_id, "constraint", &doc.constraints)?;
    save_indexed_nodes(&tx, &mut row_id, "extrusion", &doc.extrusions)?;
    save_arena_nodes(&tx, &mut row_id, "body", &doc.bodies)?;
    save_arena_nodes(&tx, &mut row_id, "material", &doc.materials)?;
    save_arena_nodes(&tx, &mut row_id, "imported_mesh", &doc.imported_meshes)?;
    save_arena_nodes(&tx, &mut row_id, "tracing_image", &doc.tracing_images)?;
    save_arena_nodes(&tx, &mut row_id, "loft", &doc.lofts)?;
    save_arena_nodes(&tx, &mut row_id, "revolution", &doc.revolutions)?;
    save_arena_nodes(&tx, &mut row_id, "primitive", &doc.primitives)?;
    save_arena_nodes(&tx, &mut row_id, "sweep", &doc.sweeps)?;
    save_arena_nodes(&tx, &mut row_id, "boolean_op", &doc.boolean_ops)?;
    save_arena_nodes(&tx, &mut row_id, "move_op", &doc.move_ops)?;
    save_arena_nodes(&tx, &mut row_id, "mirror_op", &doc.mirror_ops)?;
    save_arena_nodes(&tx, &mut row_id, "repeat_op", &doc.repeat_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "slice_op", &doc.slice_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "edge_treatment_op", &doc.edge_treatment_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "sketch_repeat_op", &doc.sketch_repeat_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "sketch_offset_op", &doc.sketch_offset_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "sketch_mirror_op", &doc.sketch_mirror_ops)?;
    save_indexed_nodes(
        &tx,
        &mut row_id,
        "sketch_vertex_treatment_op",
        &doc.sketch_vertex_treatment_ops,
    )?;
    save_indexed_nodes(&tx, &mut row_id, "sketch_slice_op", &doc.sketch_slice_ops)?;
    save_indexed_nodes(&tx, &mut row_id, "sketch_text", &doc.sketch_texts)?;
    save_indexed_nodes(&tx, &mut row_id, "drawing", &doc.drawings)?;
    save_indexed_nodes(&tx, &mut row_id, "joint", &doc.joints)?;
    save_indexed_nodes(&tx, &mut row_id, "unit", &doc.units)?;
    save_indexed_nodes(&tx, &mut row_id, "unit_instance", &doc.unit_instances)?;
    if doc.construction_planes.len() > 1 {
        save_indexed_nodes(
            &tx,
            &mut row_id,
            "construction_plane",
            &doc.construction_planes[1..],
        )?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Save an arena's live elements, one row each, keyed by [`crate::arena::Key::to_bits`]
/// rather than by position (#1055) — position is no longer identity, so the file has to
/// carry the key itself or a reload would hand every element a different one.
fn save_arena_nodes<T: serde::Serialize>(
    tx: &rusqlite::Transaction<'_>,
    row_id: &mut i64,
    kind: &str,
    arena: &crate::arena::Arena<T>,
) -> Result<()> {
    for (key, entity) in arena.iter() {
        let payload = serde_json::to_string(&(key, entity)).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO dag_nodes (id, component_id, kind, payload)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![*row_id, key.to_bits() as i64, kind, payload],
        )
        .map_err(|e| e.to_string())?;
        *row_id += 1;
    }
    Ok(())
}

/// Rebuild an arena from the rows [`save_arena_nodes`] wrote, keys intact.
fn load_arena_entities<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    kind: &str,
) -> Result<crate::arena::Arena<T>> {
    let mut stmt = conn
        .prepare("SELECT payload FROM dag_nodes WHERE kind = ?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([kind], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut entries: Vec<(crate::arena::Key<T>, T)> = Vec::new();
    for row in rows {
        let payload = row.map_err(|e| e.to_string())?;
        entries.push(serde_json::from_str(&payload).map_err(|e| e.to_string())?);
    }
    Ok(crate::arena::Arena::from_keyed(entries))
}

fn save_indexed_nodes<T: serde::Serialize>(
    tx: &rusqlite::Transaction<'_>,
    row_id: &mut i64,
    kind: &str,
    entities: &[T],
) -> Result<()> {
    for (index, entity) in entities.iter().enumerate() {
        let payload = serde_json::to_string(entity).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO dag_nodes (id, component_id, kind, payload)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![*row_id, index as i64, kind, payload],
        )
        .map_err(|e| e.to_string())?;
        *row_id += 1;
    }
    Ok(())
}

fn load_shape_order_meta(conn: &Connection) -> Option<Vec<ShapeKind>> {
    let payload: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            rusqlite::params![SHAPE_ORDER_META_KEY],
            |row| row.get(0),
        )
        .ok()?;
    serde_json::from_str(&payload).ok()
}

/// Undo-group sizes (#105); empty for files saved before the key existed (legacy
/// content reconciles into per-entry groups).
/// Load a meta row's JSON payload, `None` if absent or unparsable.
fn load_meta_json<T: serde::de::DeserializeOwned>(conn: &Connection, key: &str) -> Option<T> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
}

fn load_undo_groups_meta(conn: &Connection) -> Vec<usize> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        rusqlite::params![UNDO_GROUPS_META_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .unwrap_or_default()
}

/// Load the document-level default length unit, falling back to mm for files saved before
/// this key existed (#52).
fn load_default_length_unit_meta(conn: &Connection) -> LengthUnit {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        rusqlite::params![DEFAULT_LENGTH_UNIT_META_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .unwrap_or_default()
}

/// Load the document-level default angle unit, falling back to degrees for files saved
/// before this key existed (#52).
fn load_default_angle_unit_meta(conn: &Connection) -> AngleUnit {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        rusqlite::params![DEFAULT_ANGLE_UNIT_META_KEY],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .unwrap_or_default()
}

fn load_indexed_entities<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    kind: &str,
) -> Result<Vec<T>> {
    let mut stmt = conn
        .prepare(
            "SELECT component_id, payload FROM dag_nodes
             WHERE kind = ?1
             ORDER BY component_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![kind], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut entities = Vec::new();
    for row in rows {
        let (index, payload) = row.map_err(|e| e.to_string())?;
        let index = usize::try_from(index).map_err(|_| format!("bad {kind} index"))?;
        if index != entities.len() {
            return Err(format!(
                "{kind} indices must be dense starting at 0 (expected {}, got {index})",
                entities.len()
            ));
        }
        let entity: T = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        entities.push(entity);
    }
    Ok(entities)
}

fn load_construction_planes(
    conn: &Connection,
    dag_planes: Vec<ConstructionPlane>,
) -> Result<Vec<ConstructionPlane>> {
    if let Ok(payload) = conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        rusqlite::params![CONSTRUCTION_PLANES_META_KEY],
        |row| row.get::<_, String>(0),
    ) {
        if let Ok(planes) = serde_json::from_str::<Vec<ConstructionPlane>>(&payload) {
            if !planes.is_empty() {
                return Ok(planes);
            }
        }
    }
    let mut planes = vec![default_xy_plane()];
    planes.extend(dag_planes);
    Ok(planes)
}

/// Ensure every sketch-hosted construction-plane index exists after load.
fn load_legacy_document_nodes(
    conn: &Connection,
) -> Result<(
    crate::arena::Arena<Parameter>,
    Vec<Sketch>,
    Vec<Line>,
    Vec<Circle>,
    Vec<Constraint>,
    Vec<ConstructionPlane>,
    Vec<ShapeKind>,
)> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, payload FROM dag_nodes
             WHERE kind IN ('sketch', 'line', 'circle', 'parameter', 'constraint', 'construction_plane')
             ORDER BY id",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;

    let mut parameters = crate::arena::Arena::new();
    let mut sketches = Vec::new();
    let mut lines = Vec::new();
    let mut circles = Vec::new();
    let mut constraints = Vec::new();
    let mut construction_planes = Vec::new();
    let mut shape_order = Vec::new();
    for row in rows {
        let (kind, payload) = row.map_err(|e| e.to_string())?;
        match kind.as_str() {
            "sketch" => {
                let sketch: Sketch = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                sketches.push(sketch);
                shape_order.push(ShapeKind::Sketch);
            }
            "line" => {
                let line: Line = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                lines.push(line);
                shape_order.push(ShapeKind::Line);
            }
            "circle" => {
                let circle: Circle = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                circles.push(circle);
                shape_order.push(ShapeKind::Circle);
            }
            "parameter" => {
                let param: Parameter = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                parameters.insert(param);
                shape_order.push(ShapeKind::Parameter);
            }
            "constraint" => {
                let constraint: Constraint =
                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                constraints.push(constraint);
                shape_order.push(ShapeKind::Constraint);
            }
            "construction_plane" => {
                let plane: ConstructionPlane =
                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                construction_planes.push(plane);
                shape_order.push(ShapeKind::ConstructionPlane);
            }
            _ => {}
        }
    }
    Ok((
        parameters,
        sketches,
        lines,
        circles,
        constraints,
        construction_planes,
        shape_order,
    ))
}

/// Open the document stored at `path`.
pub fn open(path: &str) -> Result<Document> {
    // Documents saved by the web build are plain JSON (the browser has no SQLite);
    // sniff the magic bytes rather than trusting the extension, so either format opens.
    if let Ok(bytes) = std::fs::read(path) {
        if !bytes.starts_with(b"SQLite format 3") {
            let doc = super::from_json_bytes(&bytes)?;
            crate::model::validate_units(&doc, Some(std::path::Path::new(path)))?;
            return Ok(doc);
        }
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;

    let (
        parameters,
        sketches,
        lines,
        circles,
        constraints,
        construction_planes,
        shape_order,
    ) = if let Some(shape_order) = load_shape_order_meta(&conn) {
        let parameters = load_arena_entities(&conn, "parameter")?;
        let sketches = load_indexed_entities(&conn, "sketch")?;
        let lines = load_indexed_entities(&conn, "line")?;
        let circles = load_indexed_entities(&conn, "circle")?;
        let constraints = load_indexed_entities(&conn, "constraint")?;
        let dag_planes = load_indexed_entities(&conn, "construction_plane")?;
        (
            parameters,
            sketches,
            lines,
            circles,
            constraints,
            dag_planes,
            shape_order,
        )
    } else {
        load_legacy_document_nodes(&conn)?
    };

    let construction_planes =
        load_construction_planes(&conn, construction_planes).map_err(|e| e.to_string())?;
    // Extrusions/bodies (empty for legacy files that predate them).
    let extrusions = load_indexed_entities(&conn, "extrusion")?;
    let bodies = load_arena_entities(&conn, "body")?;
    // Materials (#834) — empty for files saved before they existed.
    let materials = load_arena_entities(&conn, "material")?;
    let imported_meshes = load_arena_entities(&conn, "imported_mesh")?;
    let tracing_images = load_arena_entities(&conn, "tracing_image")?;
    let lofts = load_arena_entities(&conn, "loft")?;
    let revolutions = load_arena_entities(&conn, "revolution")?;
    let primitives = load_arena_entities(&conn, "primitive")?;
    let sweeps = load_arena_entities(&conn, "sweep")?;
    let boolean_ops = load_arena_entities(&conn, "boolean_op")?;
    let move_ops = load_arena_entities(&conn, "move_op")?;
    let mirror_ops = load_arena_entities(&conn, "mirror_op")?;
    let repeat_ops = load_arena_entities(&conn, "repeat_op")?;
    let slice_ops = load_indexed_entities(&conn, "slice_op")?;
    let edge_treatment_ops = load_indexed_entities(&conn, "edge_treatment_op")?;
    let sketch_repeat_ops = load_indexed_entities(&conn, "sketch_repeat_op")?;
    let sketch_offset_ops = load_indexed_entities(&conn, "sketch_offset_op")?;
    let sketch_mirror_ops = load_indexed_entities(&conn, "sketch_mirror_op")?;
    let sketch_vertex_treatment_ops =
        load_indexed_entities(&conn, "sketch_vertex_treatment_op")?;
    let sketch_slice_ops = load_indexed_entities(&conn, "sketch_slice_op")?;
    let sketch_texts = load_indexed_entities(&conn, "sketch_text")?;
    let drawings = load_indexed_entities(&conn, "drawing")?;
    let joints = load_indexed_entities(&conn, "joint")?;
    let units = load_indexed_entities(&conn, "unit")?;
    let unit_instances = load_indexed_entities(&conn, "unit_instance")?;
    let default_length_unit = load_default_length_unit_meta(&conn);
    let default_angle_unit = load_default_angle_unit_meta(&conn);
    let undo_groups = load_undo_groups_meta(&conn);

    let components = load_meta_json(&conn, COMPONENTS_META_KEY).unwrap_or_default();
    let component_members = load_meta_json(&conn, COMPONENT_MEMBERS_META_KEY).unwrap_or_default();

    let mut doc = Document {
        parameters,
        sketches,
        lines,
        circles,
        constraints,
        construction_planes,
        extrusions,
        bodies,
        materials,
        imported_meshes,
        tracing_images,
        lofts,
        revolutions,
        primitives,
        sweeps,
        boolean_ops,
        move_ops,
        mirror_ops,
        repeat_ops,
        slice_ops,
        edge_treatment_ops,
        sketch_repeat_ops,
        sketch_offset_ops,
        sketch_mirror_ops,
        sketch_vertex_treatment_ops,
        sketch_slice_ops,
        sketch_texts,
        drawings,
        joints,
        shape_order,
        undo_groups,
        default_length_unit,
        default_angle_unit,
        components,
        component_members,
        units,
        unit_instances,
        // Cache generation starts at 0; open/save bumps it so idle frames stay cheap (#1027).
        mesh_rev: 0,
    };
    super::fixup_loaded_document(&mut doc)?;
    crate::model::validate_units(&doc, Some(std::path::Path::new(path)))?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use crate::model::body_key_for_slot as bkey;
    use crate::model::boolean_op_key_for_slot as bopkey;
    use super::*;
    use crate::model::{Circle, FaceId};

    fn plane_sketch(doc: &mut Document) -> usize {
        doc.add_sketch(FaceId::ConstructionPlane(0))
    }

    /// #408: a legacy text pin loads as a `Coincident` constraint on the text's anchor point,
    /// and the pin field is cleared (never written back).
    #[test]
    fn legacy_text_pin_migrates_to_coincident_constraint() {
        let mut doc = Document::default();
        let sketch = plane_sketch(&mut doc);
        doc.lines
            .push(crate::model::Line::from_local_endpoints(sketch, 30.0, 40.0, 60.0, 40.0));
        doc.sketch_texts.push(crate::model::SketchText {
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
                    line: 0,
                    end: crate::model::LineEnd::Start,
                },
                crate::model::TextAnchor::Center,
            )),
            name: None,
            deleted: false,
        });
        doc.shape_order.push(crate::model::ShapeKind::SketchText);
        crate::storage::fixup_loaded_document(&mut doc).expect("fixup");
        assert!(doc.sketch_texts[0].pin.is_none(), "the pin is cleared");
        let migrated = doc.constraints.iter().any(|c| {
            matches!(
                &c.kind,
                crate::model::ConstraintKind::Coincident {
                    a: crate::model::ConstraintEntity::Point(
                        crate::model::ConstraintPoint::TextAnchor {
                            text: 0,
                            anchor: crate::model::TextAnchor::Center,
                        }
                    ),
                    ..
                }
            )
        });
        assert!(migrated, "a coincident constraint replaces the pin");
        // The solve ran as part of load: the centre anchor sits on the line start.
        let (cx, cy) = crate::text::sketch_text_anchor_uv(
            &doc.sketch_texts[0],
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
        for plane in &doc.construction_planes {
            anchors.push(plane.origin);
        }
        for circle in &doc.circles {
            anchors.push(crate::face::circle_world_center(doc, circle).unwrap());
        }
        for line in &doc.lines {
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
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines
            .push(Line::from_local_endpoints(sketch, 1.0, 1.0, 1.0, 6.0));
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
        doc.components.push(crate::model::Component {
            name: Some("Frame".to_string()),
            parent: None,
            length_unit: Some(crate::value::LengthUnit::In),
            angle_unit: None,
            deleted: false,
        });
        doc.components.push(crate::model::Component {
            name: None,
            parent: Some(0),
            length_unit: None,
            angle_unit: None,
            deleted: false,
        });
        doc.set_component_member(crate::model::ComponentMember::ConstructionPlane(0), Some(1));

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
            source: crate::model::BodySource::Boolean { op: kept, solid: 0 },
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
                crate::model::BodySource::Boolean { op: kept, solid: 0 },
                "{suffix}: its output body still names it"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// #1055: bodies keep their keys across a save, and so does everything that names one —
    /// an operation's inputs and outputs, a joint's members, a drawing view's source. This is
    /// the collection with the widest blast radius, so the test carries one of each.
    #[test]
    fn body_keys_survive_a_save_and_reload() {
        let body = |name: &str| crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
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
            name: None,
        });
        doc.joints.push(crate::model::Joint {
            members: vec![
                crate::model::JointRef::Body(input),
                crate::model::JointRef::Body(output),
            ],
            base: 0,
            kind: crate::model::JointKind::Rigid,
            mate: crate::model::JointMate::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: crate::model::JointLimits::default(),
            name: None,
            deleted: false,
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
                loaded.joints[0].members,
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
            sketch: 0,
            faces: Vec::new(),
            path,
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
                Some(vec![1, 2]),
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
            sketch: 0,
            faces: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            angle_deg: angle,
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
            profile: crate::model::ExtrudeFace::Circle(0),
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
                loaded.sketches[0].face,
                doc.sketches[0].face,
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
            plane: 0,
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
        doc.construction_planes[0].name = Some("Ground".to_string());
        save(&path, &doc).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.first(), Some(&b'{'), "JSON, not SQLite");
        let loaded = open(&path).unwrap();
        assert_eq!(
            loaded.construction_planes[0].name.as_deref(),
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
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            material: None,
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(1),
            name: None,
            material: None,
            shadow: false,
        });
        doc.joints.push(crate::model::Joint {
            members: vec![
                crate::model::JointRef::Body(bkey(0)),
                crate::model::JointRef::Body(bkey(1)),
            ],
            base: 0,
            kind: crate::model::JointKind::Screw { lead: "2 * pitch".to_string() },
            mate: crate::model::JointMate {
                moving_face: Some(crate::model::MateRef::Face {
                    body: bkey(1),
                    centroid: [500, 0, 0],
                    normal: [0, 0, 100],
                }),
                fixed_face: Some(crate::model::MateRef::Plane(0)),
                flip: true,
                offset: "1.5".to_string(),
                line_up: vec![crate::model::MateLineUp {
                    moving: Some(crate::model::MateRef::Edge {
                        body: bkey(1),
                        a: [0, 0, 0],
                        b: [1000, 0, 0],
                    }),
                    fixed: Some(crate::model::MateRef::Point(
                        crate::model::MovePointRef::Origin,
                    )),
                }],
            },
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
            deleted: false,
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
            source: crate::model::BodySource::Boolean { op: bopkey(0), solid: 0 },
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
            source: crate::model::BodySource::Sliced { op: 0, target: 0, piece: 0 },
            material: None,
            name: Some("Top".to_string()),
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Sliced { op: 0, target: 0, piece: 1 },
            material: None,
            name: Some("Bottom".to_string()),
            shadow: false,
        });
        doc.slice_ops.push(crate::model::SliceOperation {
            targets: vec![bkey(0)],
            cutters: vec![crate::model::FaceId::ConstructionPlane(3)],
            extend_infinite: true,
            outputs: vec![bkey(1), bkey(2)],
            name: Some("Halved".to_string()),
            deleted: false,
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
        doc.construction_planes.truncate(1);
        doc.construction_planes.push(offset_plane);

        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(s0, 12.0, -8.0, 15.0, 0.4));
        doc.shape_order.push(ShapeKind::Circle);

        let s1 = doc.add_sketch(FaceId::ConstructionPlane(1));
        crate::construction::add_line_rectangle(&mut doc, s1, 3.0, 4.0, 10.0, 10.0, [false; 4]);
        doc.lines
            .push(Line::from_local_endpoints(s1, -2.0, 1.0, 8.0, 6.0));
        doc.shape_order.push(ShapeKind::Line);

        let before = element_world_anchors(&doc);
        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        let after = element_world_anchors(&loaded);
        assert_world_anchors_match(&before, &after);

        // A rectangle edge on the offset plane should keep its world height.
        let (a, _) = crate::face::line_world_endpoints(&loaded, &loaded.lines[0]).unwrap();
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
            source: crate::model::BodySource::Extrusion(0),
            material: Some(brass),
            name: None,
            shadow: false,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(1),
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
        doc.construction_planes.truncate(1);
        doc.construction_planes.push(offset_plane.clone());
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(1));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.construction_planes.len(), 2);
        assert_eq!(loaded.construction_planes[1], offset_plane);
        assert_eq!(
            loaded.sketches[0].face,
            FaceId::ConstructionPlane(1),
            "sketch should stay on the offset plane"
        );
        let (a, _) = crate::face::line_world_endpoints(&loaded, &loaded.lines[0]).unwrap();
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.construction_planes[0].origin.z = 30.0;
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);

        let before_origin = doc.construction_planes[0].origin;
        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(
            (loaded.construction_planes[0].origin - before_origin).length() < 1e-3,
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(1));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 10.0));
        doc.shape_order.push(ShapeKind::Line);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(
            loaded.construction_planes.len() >= 2,
            "legacy sketch references to plane 1 should not crash on load"
        );
        assert!(crate::face::line_world_endpoints(&loaded, &loaded.lines[0]).is_some());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_sketches() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_sketch_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(0));
        crate::construction::add_line_rectangle(&mut doc, s0, 0.0, 0.0, 1.0, 1.0, [false; 4]);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.sketches.len(), 2);
        assert_eq!(loaded.sketches[0].face, FaceId::ConstructionPlane(0));
        assert_eq!(loaded.sketches[1].face, FaceId::ConstructionPlane(0));
        assert_eq!(loaded.lines[0].sketch, s0);
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        doc.extrusions.push(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 12.0,
            target: None,
            expression: String::new(),
            name: Some("Boss".to_string()),
            symmetric: false,
            deleted: false,
            edge_treatments: Vec::new(),
        });
        doc.shape_order.push(ShapeKind::Extrusion);
        doc.bodies.insert(Body {
            source: BodySource::Extrusion(0),
            material: None,
            name: None,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.extrusions.len(), 1);
        assert_eq!(
            loaded.extrusions[0].faces,
            vec![ExtrudeFace::Polygon(rect_lines.to_vec())]
        );
        assert_eq!(loaded.extrusions[0].distance, 12.0);
        assert_eq!(loaded.extrusions[0].name.as_deref(), Some("Boss"));
        assert_eq!(loaded.bodies.len(), 1);
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source, BodySource::Extrusion(0));
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let outer =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        let inner =
            crate::construction::add_line_rectangle(&mut doc, sketch, 3.0, 3.0, 4.0, 4.0, [false; 4]);
        for face in [
            ExtrudeFace::Polygon(outer.to_vec()),
            ExtrudeFace::Polygon(inner.to_vec()),
        ] {
            doc.extrusions.push(Extrusion {
                sketch,
                faces: vec![face],
                distance: 5.0,
                target: None,
                expression: String::new(),
                name: None,
                symmetric: false,
                deleted: false,
                edge_treatments: Vec::new(),
            });
            doc.shape_order.push(ShapeKind::Extrusion);
        }
        doc.bodies.insert(Body {
            source: BodySource::Solid {
                add: vec![0],
                cut: vec![1],
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
            BodySource::Solid {
                add: vec![0],
                cut: vec![1],
            }
        );
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source.extrusion_indices(), [0]);
        assert_eq!(loaded.bodies.values().nth(0).unwrap().source.cut_extrusion_indices(), [1]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_circles() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_circle_roundtrip_test.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let mut circle = Circle::from_local_center_radius(sketch, 5.0, 5.0, 10.0, 0.5);
        circle.diameter_dim_offset = Some(18.0);
        circle.diameter_dim_angle = 1.2;
        circle.construction = true;
        doc.circles.push(circle);
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
            source: None,
        });
        doc.parameters.insert(Parameter {
            name: "B".to_string(),
            expression: "A".to_string(),
            primary: false,
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
            source: None,
        });
        doc.parameters.insert(Parameter {
            name: "B".to_string(),
            expression: "A + 5in".to_string(),
            primary: false,
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
    fn round_trips_tombstoned_entities() {
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_tombstone_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines[0].deleted = true;
        // A parameter that was removed for real (#1055): the file must not bring it back,
        // and its key must not come back to life either.
        let gone = doc.parameters.insert(Parameter {
            name: "width".to_string(),
            expression: "10mm".to_string(),
            primary: false,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        let kept = doc.parameters.insert(Parameter {
            name: "height".to_string(),
            expression: "20mm".to_string(),
            primary: false,
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        doc.parameters.remove(gone);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert!(loaded.lines[0].deleted);
        assert_eq!(loaded.lines.len(), 1);
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
        // already persisted generically via `dag_nodes` JSON payloads, so it should round-trip
        // with no `storage.rs` changes — verify that assumption rather than just trusting it.
        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_chamfer_fillet_parent_roundtrip.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        doc.shape_order.push(ShapeKind::Line);
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        bridge.chamfer_fillet_parent = Some(0);
        doc.lines.push(bridge);
        doc.shape_order.push(ShapeKind::Line);

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.lines.len(), 3);
        assert_eq!(loaded.lines[0].chamfer_fillet_parent, None);
        assert_eq!(loaded.lines[1].chamfer_fillet_parent, None);
        assert_eq!(loaded.lines[2].chamfer_fillet_parent, Some(0));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn round_trips_tombstoned_line_with_alive_sibling() {
        use crate::document_lifecycle::tombstone_element;
        use crate::hierarchy::SceneElement;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine};

        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_tombstone_sibling.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.constraints.push(Constraint {
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
        doc.shape_order.push(ShapeKind::Constraint);
        tombstone_element(&mut doc, SceneElement::Line(0));

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();
        assert_eq!(loaded.lines.len(), 2);
        assert!(loaded.lines[0].deleted);
        assert!(!loaded.lines[1].deleted);
        assert_eq!(loaded.constraints.len(), 1);
        let health = crate::document_health::recompute_document_health(&loaded);
        assert_eq!(
            health.element_status(SceneElement::Line(1)),
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
            source: None,
        });
        doc.shape_order.push(ShapeKind::Parameter);
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
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
        doc.units.push(ImportedUnit {
            source: UnitSource::RelativePath("missing/bracket.bearcad".to_string()),
            link: LinkMode::Static,
            document: unit_source_doc("width"),
            source_mtime: Some(1_700_000_000),
            source_hash: Some(crate::model::content_hash(b"bracket bytes")),
        });
        doc.units.push(ImportedUnit {
            source: UnitSource::Library("hardware/bolt.bearcad".to_string()),
            link: LinkMode::Dynamic,
            document: unit_source_doc("length"),
            source_mtime: None,
            source_hash: None,
        });
        doc.unit_instances.push(UnitInstance {
            unit: 0,
            name: Some("bracket1".to_string()),
            parameter_overrides: vec![("width".to_string(), "20".to_string())],
            placement: UnitPlacement {
                tx: "5".to_string(),
                ty: String::new(),
                tz: "height / 2".to_string(),
                axis: [0.0, 0.0, 1.0],
                angle: "90".to_string(),
            },
            deleted: false,
        });
        doc.unit_instances.push(UnitInstance {
            unit: 0,
            name: None,
            parameter_overrides: Vec::new(),
            placement: UnitPlacement::default(),
            deleted: true,
        });
        doc.unit_instances.push(UnitInstance {
            unit: 1,
            name: Some("bolt_a".to_string()),
            parameter_overrides: Vec::new(),
            placement: UnitPlacement::default(),
            deleted: false,
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
        inner_b.units.push(ImportedUnit {
            source: UnitSource::RelativePath("bearcad_unit_cycle_test.bearcad".to_string()),
            link: LinkMode::Static,
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        });
        let mut doc = Document::default();
        doc.units.push(ImportedUnit {
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
            outer.units.push(ImportedUnit {
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
}

}

#[cfg(not(target_arch = "wasm32"))]
pub use sqlite_format::{open, save};

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
