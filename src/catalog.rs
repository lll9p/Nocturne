use crate::error::{Error, Result};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub fn namespaces(c: &Connection) -> Result<Value> {
    let mut q = c.prepare("WITH namespaces(namespace) AS (
        SELECT '' UNION SELECT namespace FROM paths
        UNION SELECT namespace FROM glossary_keywords
        UNION SELECT namespace FROM memory_access_logs
        UNION SELECT namespace FROM changesets
        UNION SELECT j.key FROM presets p,json_each(p.boot_uris) j WHERE json_type(p.boot_uris)='object'
    ), counts AS (SELECT namespace,count(*) AS path_count FROM paths GROUP BY namespace)
    SELECT n.namespace,coalesce(c.path_count,0) FROM namespaces n
    LEFT JOIN counts c ON c.namespace=n.namespace ORDER BY n.namespace")?;
    let rows = q
        .query_map([], |r| {
            Ok(json!({"namespace":r.get::<_,String>(0)?,"path_count":r.get::<_,i64>(1)?}))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!({"namespaces":rows}))
}

pub fn paths(
    c: &Connection, ns: &str, all_namespaces: bool, domain: Option<&str>,
    node: Option<&str>,
) -> Result<Value> {
    if ns.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid namespace".into()));
    }
    let node = node
        .map(|n| {
            uuid::Uuid::parse_str(n)
                .map(|u| u.to_string())
                .map_err(|_| Error::Invalid("invalid node UUID".into()))
        })
        .transpose()?;
    let mut q = c.prepare("SELECT p.namespace,p.domain,p.path,p.node_uuid,p.edge_id,e.priority,e.disclosure,m.id
        FROM paths p LEFT JOIN edges e ON e.id=p.edge_id AND e.child_uuid=p.node_uuid
        LEFT JOIN memories m ON m.node_uuid=p.node_uuid AND m.deprecated=0
        WHERE (?2 OR p.namespace=?1) AND (?3 IS NULL OR p.domain=?3) AND (?4 IS NULL OR p.node_uuid=?4)
        ORDER BY p.namespace,p.domain,p.path")?;
    let rows = q.query_map(params![ns,all_namespaces,domain,node], |r| Ok(json!({
        "namespace":r.get::<_,String>(0)?,"domain":r.get::<_,String>(1)?,"path":r.get::<_,String>(2)?,
        "uri":format!("{}://{}",r.get::<_,String>(1)?,r.get::<_,String>(2)?),
        "node_uuid":r.get::<_,String>(3)?,"edge_id":r.get::<_,i64>(4)?,
        "priority":r.get::<_,Option<i64>>(5)?,"disclosure":r.get::<_,Option<String>>(6)?,
        "current_memory_id":r.get::<_,Option<i64>>(7)?
    })))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!(rows))
}
