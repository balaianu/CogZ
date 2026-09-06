use std::collections::HashMap;

use crate::index::sync::{CodeSyncResult, ParsedEntity};
use crate::storage;

pub(crate) fn sync_entities_to_db(
    conn: &rusqlite::Connection,
    all_entities: &[ParsedEntity],
    result: &mut CodeSyncResult,
) {
    let ids: Vec<String> = all_entities.iter().map(|(id, _)| id.clone()).collect();
    let existing_map: HashMap<String, storage::crud::Entity> =
        storage::crud::get_entities_batch(conn, &ids)
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut synced_ids: Vec<String> = Vec::new();

    // Wrap all inserts/updates in a single transaction to avoid
    // one fsync per row. unchecked_transaction() provides Drop-based
    // rollback on panic — if the code panics between BEGIN and COMMIT,
    // the Transaction's Drop impl rolls back automatically. This is
    // safer than raw execute_batch("BEGIN") which leaves the
    // transaction open on panic.
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!("failed to begin entity transaction — falling back to autocommit: {e}");
            sync_entities_loop(
                conn,
                all_entities,
                &existing_map,
                &mut created,
                &mut updated,
                &mut skipped,
                &mut synced_ids,
            );
            apply_counts(result, created, updated, skipped, synced_ids);
            return;
        }
    };

    sync_entities_loop(
        &tx,
        all_entities,
        &existing_map,
        &mut created,
        &mut updated,
        &mut skipped,
        &mut synced_ids,
    );

    if let Err(e) = tx.commit() {
        tracing::warn!("failed to commit entity transaction: {e}");
        // Transaction rolled back — discard created/updated counts
        // since those writes were undone. But skipped entities were
        // never modified (hash matched, no SQL executed), so the
        // skipped count is still accurate.
        result.skipped += skipped;
        return;
    }

    apply_counts(result, created, updated, skipped, synced_ids);
}

/// Inner loop for entity sync — works with either a `Connection` or
/// `Transaction` (both deref to `Connection`). Updates the count
/// counters and synced_ids in place.
fn sync_entities_loop(
    conn: &rusqlite::Connection,
    all_entities: &[ParsedEntity],
    existing_map: &HashMap<String, storage::crud::Entity>,
    created: &mut usize,
    updated: &mut usize,
    skipped: &mut usize,
    synced_ids: &mut Vec<String>,
) {
    for (id, entity) in all_entities {
        match existing_map.get(id) {
            Some(existing) => {
                if existing.status == "stale" {
                    let mut updated_entity = entity.clone();
                    updated_entity.status = "active".to_string();
                    updated_entity.created_at = existing.created_at.clone();
                    if let Err(e) = storage::crud::update_entity(conn, &updated_entity) {
                        tracing::warn!("failed to reactivate code entity {}: {}", id, e);
                        continue;
                    }
                    *updated += 1;
                    synced_ids.push(id.clone());
                    continue;
                }
                if existing.content_hash == entity.content_hash {
                    *skipped += 1;
                    continue;
                }
                let mut updated_entity = entity.clone();
                updated_entity.status = existing.status.clone();
                updated_entity.created_at = existing.created_at.clone();
                if let Err(e) = storage::crud::update_entity(conn, &updated_entity) {
                    tracing::warn!("failed to update code entity {}: {}", id, e);
                    continue;
                }
                *updated += 1;
                synced_ids.push(id.clone());
            }
            None => {
                if let Err(e) = storage::crud::insert_entity(conn, entity) {
                    tracing::warn!("failed to insert code entity {}: {}", id, e);
                    continue;
                }
                *created += 1;
                synced_ids.push(id.clone());
            }
        }
    }
}

/// Apply local counts to the sync result after a successful commit.
fn apply_counts(
    result: &mut CodeSyncResult,
    created: usize,
    updated: usize,
    skipped: usize,
    synced_ids: Vec<String>,
) {
    result.created += created;
    result.updated += updated;
    result.skipped += skipped;
    result.synced_entity_ids.extend(synced_ids);
}

/// Mark code entities as stale if their source file is no longer present.
///
/// Returns the number of entities marked stale. Uses a single batched
/// UPDATE query instead of fetching all entities and updating in a loop.
pub(crate) fn mark_stale_code_entities(
    conn: &rusqlite::Connection,
    current_files: &HashMap<String, Vec<String>>,
    failed_paths: &std::collections::HashSet<String>,
) -> usize {
    // Find active code entities whose file_path is NOT in current_files
    // and NOT in failed_paths. Files that failed to read are not deleted
    // — their entities must not be marked stale just because of an I/O
    // error, or valid edges will be lost and the baseline advanced.
    let code_types = ["function", "class", "file", "module"];
    let type_placeholders = (0..code_types.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT DISTINCT file_path FROM entities \
         WHERE status = 'active' AND type IN ({type_placeholders}) \
         AND file_path IS NOT NULL"
    );
    let params: Vec<&dyn rusqlite::ToSql> = code_types
        .iter()
        .map(|t| t as &dyn rusqlite::ToSql)
        .collect();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to query code entity file paths: {}", e);
            return 0;
        }
    };
    let rows = match stmt.query_map(params.as_slice(), |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to query code entity file paths: {}", e);
            return 0;
        }
    };

    let mut stale_paths: Vec<String> = Vec::new();
    for row in rows.flatten() {
        if !current_files.contains_key(&row) && !failed_paths.contains(&row) {
            stale_paths.push(row);
        }
    }

    storage::crud::mark_code_entities_stale_by_file_paths(conn, &stale_paths).unwrap_or(0)
}
