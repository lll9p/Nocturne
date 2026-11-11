use crate::{
    error::{Error, Result},
    graph,
    store::ROOT,
    uri::Uri,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

// Namespace ownership follows the last effective route deletion, never old access.
const ELIGIBLE: &str = "m.node_uuid<>?3
 AND NOT EXISTS(SELECT 1 FROM paths p WHERE p.node_uuid=m.node_uuid)
 AND NOT EXISTS(SELECT 1 FROM memory_rows a WHERE a.node_uuid=m.node_uuid AND a.deprecated=0)
 AND (?2 OR ?1=(SELECT json_extract(r.before_row,'$.namespace')
 FROM change_rows r JOIN changesets s ON s.id=r.changeset_id
 WHERE r.table_name='paths' AND r.after_row IS NULL
 AND json_extract(r.before_row,'$.node_uuid')=m.node_uuid
 AND s.status<>'rolled_back' ORDER BY r.id DESC LIMIT 1))";

fn snapshot(
    c: &Connection, f: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    if !c.is_autocommit() {
        return f();
    }
    c.execute_batch("BEGIN")?;
    let result = f();
    match result {
        Ok(value) => {
            c.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(e) => {
            c.execute_batch("ROLLBACK")?;
            Err(e)
        }
    }
}

fn last_unlinked(c: &Connection, node: &str) -> Result<Value> {
    let record = c
        .query_row(
            "SELECT json_extract(r.before_row,'$.namespace'),json_extract(r.before_row,'$.domain'),
        json_extract(r.before_row,'$.path'),s.created_at FROM change_rows r
        JOIN changesets s ON s.id=r.changeset_id WHERE r.table_name='paths'
        AND r.after_row IS NULL AND json_extract(r.before_row,'$.node_uuid')=?1
        AND s.status<>'rolled_back' ORDER BY r.id DESC LIMIT 1",
            [node],
            |r| {
                Ok(json!({"namespace":r.get::<_,String>(0)?,
            "uri":format!("{}://{}",r.get::<_,String>(1)?,r.get::<_,String>(2)?),
            "recorded_at":r.get::<_,String>(3)?}))
            },
        )
        .optional()?;
    Ok(record.unwrap_or(Value::Null))
}

pub fn list(c: &Connection, ns: &str, all_namespaces: bool) -> Result<Value> {
    snapshot(c, || list_snapshot(c, ns, all_namespaces))
}

fn list_snapshot(
    c: &Connection, ns: &str, all_namespaces: bool,
) -> Result<Value> {
    let mut q = c.prepare(&format!(
        "SELECT m.node_uuid,m.id,substr(m.content,1,200),
        (SELECT count(*) FROM memory_rows v WHERE v.node_uuid=m.node_uuid),m.created_at
        FROM memory_rows m WHERE {ELIGIBLE}
        AND m.id=(SELECT max(v.id) FROM memory_rows v WHERE v.node_uuid=m.node_uuid)
        ORDER BY m.id DESC"
    ))?;
    let mut rows = q
        .query_map(params![ns, all_namespaces, ROOT], |r| {
            Ok(
                json!({"node_uuid":r.get::<_,String>(0)?,"memory_id":r.get::<_,i64>(1)?,
            "content_snippet":r.get::<_,String>(2)?,"version_count":r.get::<_,i64>(3)?,
            "created_at":r.get::<_,String>(4)?}),
            )
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in &mut rows {
        row["last_unlinked"] =
            last_unlinked(c, row["node_uuid"].as_str().unwrap())?;
    }
    Ok(json!(rows))
}

fn selected(
    c: &Connection, ns: &str, id: i64, all: bool,
) -> Result<(String, String)> {
    c.query_row(
        &format!(
            "SELECT m.node_uuid,m.content FROM memory_rows m
        WHERE m.id=?4 AND {ELIGIBLE}"
        ),
        params![ns, all, ROOT, id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| {
        Error::NotFound(if all {
            format!("orphan memory {id} not found across all namespaces")
        } else {
            format!("orphan memory {id} not found in namespace {ns:?}")
        })
    })
}

pub fn show(
    c: &Connection, ns: &str, memory_id: i64, all_namespaces: bool,
) -> Result<Value> {
    snapshot(c, || {
        let (node, content) = selected(c, ns, memory_id, all_namespaces)?;
        let mut q = c.prepare(
            "SELECT id,content,deprecated,migrated_to,created_at
            FROM memory_rows WHERE node_uuid=?1 ORDER BY id DESC",
        )?;
        let history = q
            .query_map([&node], |r| {
                Ok(json!({
                    "id":r.get::<_,i64>(0)?,"content":r.get::<_,String>(1)?,
                    "deprecated":r.get::<_,bool>(2)?,"migrated_to":r.get::<_,Option<i64>>(3)?,
                    "created_at":r.get::<_,String>(4)?
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(
            json!({"node_uuid":node,"memory_id":memory_id,"content":content,
            "version_count":history.len(),"history":history,"attrs":crate::attrs::for_node(c, &node)?,
            "last_unlinked":last_unlinked(c, &node)?}),
        )
    })
}

// Call through Store::write: it owns atomicity, audit capture and FTS refresh.
pub fn restore(
    c: &Connection, ns: &str, memory_id: i64, destination: &str, priority: i64,
    disclosure: Option<&str>, all_namespaces: bool,
) -> Result<Value> {
    let (node, content) = selected(c, ns, memory_id, all_namespaces)?;
    let dst = Uri::parse(destination)?;
    graph::validate_priority(priority)?;
    if dst.path.is_empty() {
        return Err(Error::Invalid(format!(
            "restore destination {destination:?} must have a non-empty path"
        )));
    }
    if dst.domain == "system" {
        return Err(Error::Invalid(format!(
            "restore destination {destination:?} uses the reserved system domain"
        )));
    }
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM paths
        WHERE namespace=?1 AND domain=?2 AND path=?3)",
        params![ns, dst.domain, dst.path],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict("destination already exists".into()));
    }
    let (prefix, name) = dst.path.rsplit_once('/').unwrap_or(("", &dst.path));
    crate::uri::validate_name(name, "restore name")?;
    let parent =
        graph::resolve(c, ns, &format!("{}://{}", dst.domain, prefix))?;
    let cycle: bool = c.query_row("WITH RECURSIVE descendants(uuid) AS
        (SELECT ?1 UNION SELECT e.child_uuid FROM edges e JOIN descendants d ON e.parent_uuid=d.uuid)
        SELECT EXISTS(SELECT 1 FROM descendants WHERE uuid=?2)",
        params![node, parent], |r| r.get(0))?;
    if cycle {
        return Err(Error::Conflict("restore would create a cycle".into()));
    }
    c.execute(
        "DELETE FROM edges WHERE child_uuid=?1 AND NOT EXISTS(SELECT 1 FROM paths WHERE edge_id=edges.id)",
        [&node],
    )?;
    let terminal: Option<i64> = c.query_row(
        "SELECT max(id) FROM memory_rows
        WHERE node_uuid=?1 AND migrated_to IS NULL",
        [&node],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT INTO memories(node_uuid,content_id) VALUES(?1,?2)",
        params![node, crate::content::insert(c, &content)?],
    )?;
    let new_id = c.last_insert_rowid();
    if let Some(old) = terminal {
        c.execute(
            "UPDATE memories SET migrated_to=?1 WHERE id=?2",
            params![new_id, old],
        )?;
    }
    c.execute(
        "INSERT INTO edges(parent_uuid,child_uuid,name,priority,disclosure)
        VALUES(?1,?2,?3,?4,?5)",
        params![parent, node, name, priority, disclosure],
    )?;
    let edge = c.last_insert_rowid();
    c.execute(
        "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",
        params![ns, dst.domain, dst.path, edge, node],
    )?;
    graph::read(c, ns, destination, false)
}
