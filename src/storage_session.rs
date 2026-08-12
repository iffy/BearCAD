// Live document session: incremental INSERT/UPDATE/DELETE inside one open
// transaction; COMMIT on Save; ROLLBACK on discard (#1341).

use std::collections::BTreeMap;

/// Per-table counts from one incremental flush.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableWrite {
    pub inserts: u32,
    pub updates: u32,
    pub deletes: u32,
}

/// What the last incremental flush wrote. Unrelated tables stay at zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncrementalWrite {
    pub tables: BTreeMap<String, TableWrite>,
}

impl IncrementalWrite {
    #[cfg(test)]
    pub fn inserts(&self, table: &str) -> u32 {
        self.tables.get(table).map(|t| t.inserts).unwrap_or(0)
    }
    #[cfg(test)]
    pub fn updates(&self, table: &str) -> u32 {
        self.tables.get(table).map(|t| t.updates).unwrap_or(0)
    }
    #[cfg(test)]
    pub fn deletes(&self, table: &str) -> u32 {
        self.tables.get(table).map(|t| t.deletes).unwrap_or(0)
    }
    #[cfg(test)]
    pub fn touched(&self, table: &str) -> bool {
        self.inserts(table) + self.updates(table) + self.deletes(table) > 0
    }
    fn entry(&mut self, table: &str) -> &mut TableWrite {
        self.tables.entry(table.to_string()).or_default()
    }
    fn add_insert(&mut self, table: &str) {
        self.entry(table).inserts += 1;
    }
    fn add_update(&mut self, table: &str) {
        self.entry(table).updates += 1;
    }
    fn add_delete(&mut self, table: &str) {
        self.entry(table).deletes += 1;
    }
}

/// Open write connection on a pathed `.bearcad`. Edits mutate the open transaction;
/// [`Self::commit`] publishes them. Drop / [`Self::rollback`] discards them.
pub struct DocumentSession {
    conn: Connection,
    path: String,
    flushed: Document,
    in_txn: bool,
    last_write: IncrementalWrite,
}

impl DocumentSession {
    /// Open the existing file and `BEGIN` a write transaction.
    pub fn attach(path: &str, doc: &Document) -> Result<Self> {
        if path.ends_with(".json") {
            return Err("JSON documents have no live SQLite session".into());
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        // User documents must not grow WAL sidecars.
        conn.pragma_update(None, "journal_mode", "DELETE")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "busy_timeout", 5_000)
            .map_err(|e| e.to_string())?;
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            path: path.to_string(),
            flushed: doc.clone(),
            in_txn: true,
            last_write: IncrementalWrite::default(),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn last_write(&self) -> &IncrementalWrite {
        &self.last_write
    }

    /// INSERT/UPDATE/DELETE only the rows that changed since the last flush.
    pub fn flush(&mut self, doc: &Document) -> Result<&IncrementalWrite> {
        self.ensure_txn()?;
        self.last_write = incremental_write(&self.conn, &self.flushed, doc)?;
        self.flushed = doc.clone();
        Ok(&self.last_write)
    }

    pub fn commit(&mut self) -> Result<()> {
        if self.in_txn {
            self.conn
                .execute_batch("COMMIT")
                .map_err(|e| e.to_string())?;
            self.in_txn = false;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        if self.in_txn {
            self.conn
                .execute_batch("ROLLBACK")
                .map_err(|e| e.to_string())?;
            self.in_txn = false;
        }
        Ok(())
    }

    pub fn begin(&mut self) -> Result<()> {
        self.ensure_txn()
    }

    /// First column of the first row as i64 — sees uncommitted session writes.
    #[cfg(test)]
    pub fn query_i64(&self, sql: &str) -> Result<i64> {
        self.conn
            .query_row(sql, [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }

    /// First column of the first row as text — sees uncommitted session writes.
    #[cfg(test)]
    pub fn query_text(&self, sql: &str) -> Result<String> {
        self.conn
            .query_row(sql, [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }

    /// First column of the first row as a blob — sees uncommitted session writes.
    #[cfg(test)]
    pub fn query_blob(&self, sql: &str) -> Result<Vec<u8>> {
        self.conn
            .query_row(sql, [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }

    fn ensure_txn(&mut self) -> Result<()> {
        if !self.in_txn {
            self.conn
                .execute_batch("BEGIN")
                .map_err(|e| e.to_string())?;
            self.in_txn = true;
        }
        Ok(())
    }
}

fn incremental_write(
    conn: &Connection,
    old: &Document,
    new: &Document,
) -> Result<IncrementalWrite> {
    let mut stats = IncrementalWrite::default();

    sync_arena(
        conn,
        "parameters",
        &old.parameters,
        &new.parameters,
        save_parameters,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketches",
        &old.sketches,
        &new.sketches,
        save_sketches,
        &mut stats,
    )?;
    sync_arena(
        conn, "lines", &old.lines, &new.lines, save_lines, &mut stats,
    )?;
    sync_arena(
        conn,
        "circles",
        &old.circles,
        &new.circles,
        save_circles,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "constraints",
        &old.constraints,
        &new.constraints,
        save_constraints,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "construction_planes",
        &old.construction_planes,
        &new.construction_planes,
        save_planes,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "extrusions",
        &old.extrusions,
        &new.extrusions,
        save_extrusions,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "bodies",
        &old.bodies,
        &new.bodies,
        save_bodies,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "materials",
        &old.materials,
        &new.materials,
        save_materials,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "imported_meshes",
        &old.imported_meshes,
        &new.imported_meshes,
        save_imported_meshes,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "tracing_images",
        &old.tracing_images,
        &new.tracing_images,
        save_tracing_images,
        &mut stats,
    )?;
    sync_arena(
        conn, "lofts", &old.lofts, &new.lofts, save_lofts, &mut stats,
    )?;
    sync_arena(
        conn,
        "revolutions",
        &old.revolutions,
        &new.revolutions,
        save_revolutions,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "primitives",
        &old.primitives,
        &new.primitives,
        save_primitives,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sweeps",
        &old.sweeps,
        &new.sweeps,
        save_sweeps,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "boolean_ops",
        &old.boolean_ops,
        &new.boolean_ops,
        save_boolean_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "move_ops",
        &old.move_ops,
        &new.move_ops,
        save_move_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "mirror_ops",
        &old.mirror_ops,
        &new.mirror_ops,
        save_mirror_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "repeat_ops",
        &old.repeat_ops,
        &new.repeat_ops,
        save_repeat_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "slice_ops",
        &old.slice_ops,
        &new.slice_ops,
        save_slice_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "shell_ops",
        &old.shell_ops,
        &new.shell_ops,
        save_shell_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "edge_treatment_ops",
        &old.edge_treatment_ops,
        &new.edge_treatment_ops,
        save_edge_treatment_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_repeat_ops",
        &old.sketch_repeat_ops,
        &new.sketch_repeat_ops,
        save_sketch_repeat_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_offset_ops",
        &old.sketch_offset_ops,
        &new.sketch_offset_ops,
        save_sketch_offset_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_mirror_ops",
        &old.sketch_mirror_ops,
        &new.sketch_mirror_ops,
        save_sketch_mirror_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_vertex_treatment_ops",
        &old.sketch_vertex_treatment_ops,
        &new.sketch_vertex_treatment_ops,
        save_sketch_vertex_treatment_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_slice_ops",
        &old.sketch_slice_ops,
        &new.sketch_slice_ops,
        save_sketch_slice_ops,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "sketch_texts",
        &old.sketch_texts,
        &new.sketch_texts,
        save_sketch_texts,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "drawings",
        &old.drawings,
        &new.drawings,
        save_drawings,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "joints",
        &old.joints,
        &new.joints,
        save_joints,
        &mut stats,
    )?;
    sync_arena(
        conn, "units", &old.units, &new.units, save_units, &mut stats,
    )?;
    sync_arena(
        conn,
        "unit_instances",
        &old.unit_instances,
        &new.unit_instances,
        save_unit_instances,
        &mut stats,
    )?;
    sync_arena(
        conn,
        "components",
        &old.components,
        &new.components,
        save_components,
        &mut stats,
    )?;

    rewrite_vec(
        conn,
        "component_members",
        &old.component_members,
        &new.component_members,
        |c| save_component_members(c, &new.component_members),
        &mut stats,
    )?;
    rewrite_vec(
        conn,
        "shape_order",
        &old.shape_order,
        &new.shape_order,
        |c| save_shape_order(c, &new.shape_order),
        &mut stats,
    )?;
    rewrite_vec(
        conn,
        "undo_groups",
        &old.undo_groups,
        &new.undo_groups,
        |c| save_undo_groups(c, &new.undo_groups),
        &mut stats,
    )?;

    if old.default_length_unit != new.default_length_unit {
        put_meta(
            conn,
            DEFAULT_LENGTH_UNIT_META_KEY,
            &to_json(&new.default_length_unit)?,
        )?;
        stats.add_update("meta");
    }
    if old.default_angle_unit != new.default_angle_unit {
        put_meta(
            conn,
            DEFAULT_ANGLE_UNIT_META_KEY,
            &to_json(&new.default_angle_unit)?,
        )?;
        stats.add_update("meta");
    }

    Ok(stats)
}

fn delete_row(conn: &Connection, table: &str, id: i64) -> Result<()> {
    conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn sync_arena<T: Clone + PartialEq>(
    conn: &Connection,
    table: &str,
    old: &Arena<T>,
    new: &Arena<T>,
    save: impl Fn(&Connection, &Arena<T>) -> Result<()>,
    stats: &mut IncrementalWrite,
) -> Result<()> {
    for (key, _) in old.iter() {
        if !new.contains(key) {
            let id = key_bits(key);
            delete_row(conn, table, id)?;
            delete_entity_blobs(conn, id)?;
            stats.add_delete(table);
        }
    }
    for (key, val) in new.iter() {
        let existed = old.get(key);
        let changed = match existed {
            None => true,
            Some(old_val) => old_val != val,
        };
        if !changed {
            continue;
        }
        if existed.is_some() {
            let id = key_bits(key);
            delete_row(conn, table, id)?;
            delete_entity_blobs(conn, id)?;
            stats.add_update(table);
        } else {
            stats.add_insert(table);
        }
        let one = Arena::from_keyed(std::iter::once((key, val.clone())))?;
        save(conn, &one)?;
    }
    Ok(())
}

fn rewrite_vec<T: PartialEq>(
    conn: &Connection,
    table: &str,
    old: &T,
    new: &T,
    save: impl FnOnce(&Connection) -> Result<()>,
    stats: &mut IncrementalWrite,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    conn.execute(&format!("DELETE FROM {table}"), [])
        .map_err(|e| e.to_string())?;
    save(conn)?;
    stats.add_update(table);
    Ok(())
}
