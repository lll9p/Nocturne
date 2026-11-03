use crate::error::{Error, Result};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) struct Temporary(pub PathBuf);
impl Temporary {
    pub(crate) fn new(target: &Path) -> Result<Self> {
        if target.exists() {
            return Err(Error::Conflict("destination already exists".into()));
        }
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let path =
            parent.join(format!(".nocturne-{}.db", uuid::Uuid::new_v4()));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self(path))
    }
    pub(crate) fn publish(&self, target: &Path) -> Result<()> {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&self.0)?
            .sync_all()?;
        std::fs::hard_link(&self.0, target)?;
        Ok(())
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ =
                std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

pub fn backup(conn: &Connection, destination: &Path) -> Result<Value> {
    let temporary = Temporary::new(destination)?;
    {
        let mut output = Connection::open(&temporary.0)?;
        let backup = rusqlite::backup::Backup::new(conn, &mut output)?;
        backup.run_to_completion(
            128,
            std::time::Duration::from_millis(10),
            None,
        )?;
        drop(backup);
        output.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    }
    temporary.publish(destination)?;
    Ok(json!({"ok": true, "destination": destination}))
}

pub fn doctor(conn: &Connection) -> Result<Value> {
    let mut issues = Vec::<Value>::new();
    for (check, expected) in [
        ("application_id", crate::store::APPLICATION_ID),
        ("user_version", crate::store::SCHEMA_VERSION),
    ] {
        let actual: i64 = conn.pragma_query_value(None, check, |r| r.get(0))?;
        if actual != expected {
            issues.push(
                json!({"check":check,"expected":expected,"actual":actual}),
            );
        }
    }
    let mut statement = conn.prepare("PRAGMA integrity_check")?;
    for item in statement.query_map([], |r| r.get::<_, String>(0))? {
        let detail = item?;
        if detail != "ok" {
            issues.push(json!({"check":"integrity", "detail":detail}));
        }
    }
    let mut statement = conn.prepare("PRAGMA foreign_key_check")?;
    for item in statement.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })? {
        issues.push(json!({"check":"foreign_key", "detail":item?}));
    }
    for (check, sql) in [
        (
            "attrs_json",
            "SELECT count(*) FROM node_attrs WHERE CASE WHEN json_valid(data_json) THEN json_type(data_json)<>'object' ELSE 1 END",
        ),
        (
            "missing_root",
            "SELECT NOT EXISTS(SELECT 1 FROM nodes WHERE uuid='00000000-0000-0000-0000-000000000000')",
        ),
        (
            "path_without_active",
            "SELECT count(*) FROM paths p WHERE p.node_uuid<>'00000000-0000-0000-0000-000000000000' AND NOT EXISTS(SELECT 1 FROM memories m WHERE m.node_uuid=p.node_uuid AND m.deprecated=0)",
        ),
        (
            "path_without_edge",
            "SELECT count(*) FROM paths WHERE edge_id IS NULL AND node_uuid<>'00000000-0000-0000-0000-000000000000'",
        ),
        (
            "active_versions",
            "SELECT count(*) FROM (SELECT node_uuid FROM memories WHERE deprecated=0 GROUP BY node_uuid HAVING count(*)>1)",
        ),
        (
            "active_presets",
            "SELECT max(count(*)-1,0) FROM presets WHERE is_active=1",
        ),
        (
            "path_edge",
            "SELECT count(*) FROM paths p LEFT JOIN edges e ON e.id=p.edge_id WHERE p.edge_id IS NOT NULL AND (e.id IS NULL OR e.child_uuid<>p.node_uuid)",
        ),
        (
            "version_reference",
            "SELECT count(*) FROM memories m LEFT JOIN memories n ON n.id=m.migrated_to WHERE m.migrated_to IS NOT NULL AND (n.id IS NULL OR n.node_uuid<>m.node_uuid)",
        ),
        (
            "version_cycle",
            "WITH RECURSIVE reach(a,b) AS (SELECT id,migrated_to FROM memories WHERE migrated_to IS NOT NULL UNION SELECT r.a,m.migrated_to FROM reach r JOIN memories m ON m.id=r.b WHERE m.migrated_to IS NOT NULL) SELECT count(*) FROM reach WHERE a=b",
        ),
        ("graph_cycle", crate::graph::CYCLE_COUNT_SQL),
        (
            "active_unreachable",
            "SELECT count(*) FROM memories m WHERE deprecated=0 AND node_uuid<>'00000000-0000-0000-0000-000000000000' AND NOT EXISTS(SELECT 1 FROM paths p WHERE p.node_uuid=m.node_uuid)",
        ),
    ] {
        let count: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        if count > 0 {
            issues.push(json!({"check":check,"count":count}));
        }
    }
    let mut statement = conn.prepare("SELECT p.namespace,p.domain,p.path,e.parent_uuid FROM paths p JOIN edges e ON e.id=p.edge_id")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })? {
        let (ns, domain, path, parent) = row?;
        let valid = match path.rsplit_once('/') {
            Some((prefix,_)) => conn.query_row("SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3 AND node_uuid=?4)", rusqlite::params![ns,domain,prefix,parent], |r| r.get::<_,bool>(0))?,
            None => parent == crate::store::ROOT,
        };
        if !valid {
            issues.push(json!({"check":"path_tree","namespace":ns,"domain":domain,"path":path}));
        }
    }
    // Search tables are derived, but missing rows must never look healthy.
    let expected: i64 = conn.query_row("SELECT count(*) FROM paths p JOIN memories m ON m.node_uuid=p.node_uuid AND m.deprecated=0", [], |r| r.get(0))?;
    let actual: i64 =
        conn.query_row("SELECT count(*) FROM search_documents", [], |r| {
            r.get(0)
        })?;
    if expected != actual {
        issues.push(
            json!({"check":"search_count","expected":expected,"actual":actual}),
        );
    }
    if let Err(error) = conn.execute_batch(
        "INSERT INTO search_documents_fts(search_documents_fts,rank) VALUES('integrity-check',1)",
    ) {
        issues.push(json!({"check":"fts_integrity","detail":error.to_string()}));
    }
    let stale: i64 = conn.query_row("SELECT count(*) FROM search_documents s LEFT JOIN paths p ON p.namespace=s.namespace AND p.domain=s.domain AND p.path=s.path LEFT JOIN memories m ON m.id=s.memory_id LEFT JOIN edges e ON e.id=p.edge_id WHERE p.node_uuid IS NOT s.node_uuid OR m.node_uuid IS NOT s.node_uuid OR m.deprecated IS NOT 0 OR m.content IS NOT s.content OR s.priority IS NOT e.priority OR s.disclosure IS NOT e.disclosure OR s.uri IS NOT (p.domain || '://' || p.path)", [], |r| r.get(0))?;
    if stale > 0 {
        issues.push(json!({"check":"search_stale","count":stale}));
    }
    Ok(json!({"ok":issues.is_empty(),"issues":issues}))
}

/// Compact in place. Checkpoints respect the connection's configured busy timeout.
pub fn vacuum(conn: &Connection) -> Result<Value> {
    if !conn.is_autocommit() {
        return Err(Error::Invalid("vacuum requires autocommit".into()));
    }
    fn size(conn: &Connection) -> Result<Value> {
        let pages: i64 =
            conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
        let free: i64 =
            conn.pragma_query_value(None, "freelist_count", |r| r.get(0))?;
        let page_size: i64 =
            conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
        Ok(
            json!({"page_count":pages,"freelist_count":free,"page_size":page_size,"db_bytes":pages*page_size}),
        )
    }
    fn checkpoint(conn: &Connection) -> Result<()> {
        let busy: i64 =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                r.get(0)
            })?;
        if busy != 0 {
            return Err(Error::Conflict(
                "WAL checkpoint blocked by another connection".into(),
            ));
        }
        Ok(())
    }
    let before = size(conn)?;
    checkpoint(conn)?;
    conn.execute_batch("VACUUM")?;
    checkpoint(conn)?;
    Ok(json!({"ok":true,"before":before,"after":size(conn)?}))
}
