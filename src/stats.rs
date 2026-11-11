use crate::error::Result;
use crate::store::ROOT;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

// Unlike Store's rollback guard, this must never roll back the caller's transaction.
struct Snapshot<'a>(&'a Connection);
impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        let _ = self
            .0
            .execute_batch("ROLLBACK TO nocturne_capacity_stats; RELEASE nocturne_capacity_stats");
    }
}

/// Database-wide observations, not purge candidates or estimates of reclaimable space.
/// Orphans are non-ROOT nodes with no path in any namespace.
/// All byte counts for text are UTF-8 payload sizes, not physical table sizes.
pub fn stats(c: &Connection) -> Result<Value> {
    c.execute_batch("SAVEPOINT nocturne_capacity_stats")?;
    let guard = Snapshot(c);
    let result = collect(c)?;
    c.execute_batch("RELEASE nocturne_capacity_stats")?;
    std::mem::forget(guard);
    Ok(result)
}

fn collect(c: &Connection) -> Result<Value> {
    // Establish the main database read snapshot before reading its page metadata.
    let memory_nodes: i64 = c.query_row(
        "SELECT count(*) FROM main.nodes WHERE uuid<>?1",
        [ROOT],
        |r| r.get(0),
    )?;
    let page_size: i64 =
        c.pragma_query_value(Some("main"), "page_size", |r| r.get(0))?;
    let pages: i64 =
        c.pragma_query_value(Some("main"), "page_count", |r| r.get(0))?;
    let free: i64 =
        c.pragma_query_value(Some("main"), "freelist_count", |r| r.get(0))?;
    let main_path: Option<String> = c.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |r| r.get(0),
    )?;
    let (main_file_bytes, wal_file_bytes) = match main_path
        .filter(|s| !s.is_empty())
    {
        Some(path) => {
            let main = std::fs::metadata(&path)?.len();
            let mut wal = Path::new(&path).as_os_str().to_os_string();
            wal.push("-wal");
            let wal = match std::fs::metadata(wal) {
                Ok(meta) => meta.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error.into()),
            };
            (Some(main), Some(wal))
        }
        None => (None, None),
    };
    let counts = c.query_row(
        "SELECT (SELECT count(*) FROM main.edges), (SELECT count(*) FROM main.paths),
         (SELECT count(*) FROM main.glossary_keywords), (SELECT count(*) FROM main.presets),
         (SELECT count(*) FROM main.search_documents)",
        [],
        |r| {
            Ok(json!({
                "memory_nodes": memory_nodes, "edges": r.get::<_, i64>(0)?,
                "paths": r.get::<_, i64>(1)?, "glossary_bindings": r.get::<_, i64>(2)?,
                "presets": r.get::<_, i64>(3)?, "search_documents": r.get::<_, i64>(4)?
            }))
        },
    )?;
    let contents = c.query_row(
        "WITH bodies AS (
            SELECT length(CAST(b.content AS BLOB)) AS bytes,
                EXISTS(SELECT 1 FROM main.memories m WHERE m.content_id=b.id) AS business,
                EXISTS(SELECT 1 FROM main.change_rows r WHERE r.before_content_id=b.id OR r.after_content_id=b.id) AS audit
            FROM main.contents b
         ) SELECT count(*),coalesce(sum(bytes),0),
            count(CASE WHEN NOT business AND audit THEN 1 END),
            coalesce(sum(CASE WHEN NOT business AND audit THEN bytes ELSE 0 END),0),
            count(CASE WHEN NOT business AND NOT audit THEN 1 END),
            coalesce(sum(CASE WHEN NOT business AND NOT audit THEN bytes ELSE 0 END),0)
         FROM bodies",
        [],
        |r| Ok(json!({
            "stored":{"rows":r.get::<_,i64>(0)?,"content_bytes":r.get::<_,i64>(1)?},
            "audit_only":{"rows":r.get::<_,i64>(2)?,"content_bytes":r.get::<_,i64>(3)?},
            "unreferenced":{"rows":r.get::<_,i64>(4)?,"content_bytes":r.get::<_,i64>(5)?}
        })),
    )?;
    let mut attrs_rows = 0u64;
    let mut attrs_bytes = 0u64;
    let mut q = c.prepare("SELECT data_json FROM main.node_attrs")?;
    for row in q.query_map([], |r| r.get::<_, String>(0))? {
        attrs_rows += 1;
        attrs_bytes += row?.len() as u64;
    }
    let orphan_nodes: i64 = c.query_row(
        "SELECT count(*) FROM main.nodes n WHERE n.uuid<>?1
         AND NOT EXISTS(SELECT 1 FROM main.paths p WHERE p.node_uuid=n.uuid)",
        [ROOT],
        |r| r.get(0),
    )?;
    let (mut active, mut deprecated, mut active_bytes, mut deprecated_bytes) =
        (0u64, 0u64, 0u64, 0u64);
    let (mut orphan_versions, mut orphan_bytes) = (0u64, 0u64);
    let mut q = c.prepare(
        "SELECT m.content,m.deprecated,m.node_uuid<>?1 AND NOT EXISTS
         (SELECT 1 FROM main.paths p WHERE p.node_uuid=m.node_uuid) FROM main.memory_rows m",
    )?;
    let mut rows = q.query([ROOT])?;
    while let Some(row) = rows.next()? {
        // Decode to Rust text so this remains UTF-8 even for a UTF-16 SQLite database.
        let bytes = row.get::<_, String>(0)?.len() as u64;
        if row.get::<_, i64>(1)? == 0 {
            active += 1;
            active_bytes += bytes;
        } else {
            deprecated += 1;
            deprecated_bytes += bytes;
        }
        if row.get::<_, bool>(2)? {
            orphan_versions += 1;
            orphan_bytes += bytes;
        }
    }
    let mut review = json!({});
    for status in ["pending", "accepted", "rolled_back"] {
        let changesets: i64 = c.query_row(
            "SELECT count(*) FROM main.changesets WHERE status=?1",
            [status],
            |r| r.get(0),
        )?;
        let mut q = c.prepare(
            "SELECT r.before_row,r.after_row FROM main.change_rows r JOIN main.changesets s
             ON s.id=r.changeset_id WHERE s.status=?1",
        )?;
        let mut rows = q.query([status])?;
        let (mut change_rows, mut snapshot_bytes) = (0u64, 0u64);
        while let Some(row) = rows.next()? {
            change_rows += 1;
            for index in [0, 1] {
                if let Some(snapshot) = row.get::<_, Option<String>>(index)? {
                    let value = crate::content::expand(
                        c,
                        serde_json::from_str(&snapshot)?,
                    )?;
                    snapshot_bytes += value.to_string().len() as u64;
                }
            }
        }
        review[status] = json!({"changesets":changesets,"change_rows":change_rows,"snapshot_bytes":snapshot_bytes});
    }
    let access_logs = c.query_row(
        "SELECT count(*),min(accessed_at),max(accessed_at) FROM main.memory_access_logs", [],
        |r| Ok(json!({"count":r.get::<_,i64>(0)?,"oldest":r.get::<_,Option<String>>(1)?,"newest":r.get::<_,Option<String>>(2)?})),
    )?;
    Ok(json!({
        "scope":"database",
        "semantics": {
            "statistics_only":true, "purge_candidates":false,
            "reclaimable_bytes_estimated":false,
            "memory_nodes_excludes_root":true,
            "orphan_definition":"Non-ROOT nodes with no path in any namespace; versions and bytes include all their memory versions",
            "text_bytes":"UTF-8 payload bytes, not physical table allocation",
            "storage_bytes":"SQLite main database page accounting, not per-table allocation",
            "contents_bytes":"Stored text payload bytes in database encoding, not physical allocation"
        },
        "storage":{"page_size_bytes":page_size,"page_count":pages,"freelist_pages":free,
            "allocated_bytes":page_size*pages,"free_bytes":page_size*free,"used_bytes":page_size*(pages-free),
            "main_file_bytes":main_file_bytes,"wal_file_bytes":wal_file_bytes},
        "counts":counts,
        "contents":contents,
        "node_attrs":{"rows":attrs_rows,"data_bytes":attrs_bytes},
        "memories":{
            "total":{"versions":active+deprecated,"content_bytes":active_bytes+deprecated_bytes},
            "active":{"versions":active,"content_bytes":active_bytes},
            "deprecated":{"versions":deprecated,"content_bytes":deprecated_bytes},
            "orphan_nodes":orphan_nodes,"orphan_versions":orphan_versions,"orphan_bytes":orphan_bytes
        },
        "review":review,"access_logs":access_logs
    }))
}
