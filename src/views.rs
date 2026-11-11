use crate::{
    error::{Error, Result},
    graph,
    store::ROOT,
    uri::Uri,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashSet;

fn snippet(text: &str) -> String {
    let short: String = text
        .chars()
        .take(100)
        .collect::<String>()
        .replace('\n', " ");
    format!(
        "{short}{}",
        if text.chars().count() > 100 {
            "..."
        } else {
            ""
        }
    )
}

pub fn index(c: &Connection, ns: &str, domain: Option<&str>) -> Result<Value> {
    index_page(c, ns, domain, None, 0)
}

// A UTF-8 scalar uses at most four bytes. Keep 101 scalars for ellipsis detection,
// using BLOB substr because SQLite TEXT substr stops at embedded NUL.
fn blob_snippet(bytes: Vec<u8>) -> String {
    let end = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) => error.valid_up_to(),
    };
    snippet(std::str::from_utf8(&bytes[..end]).unwrap())
}

pub fn index_page(
    c: &Connection, ns: &str, domain: Option<&str>, limit: Option<usize>,
    offset: usize,
) -> Result<Value> {
    let mut page = index_page_with_info(c, ns, domain, limit, offset, false)?;
    Ok(page.get_mut("results").unwrap().take())
}

pub fn index_page_with_info(
    c: &Connection, ns: &str, domain: Option<&str>, limit: Option<usize>,
    offset: usize, with_total: bool,
) -> Result<Value> {
    let offset_i64 = i64::try_from(offset).map_err(|_| {
        Error::Invalid("Index offset exceeds SQLite integer range".into())
    })?;
    let limit_i64 = match limit {
        Some(v) => Some(i64::try_from(v).map_err(|_| {
            Error::Invalid("Index limit exceeds SQLite integer range".into())
        })?),
        None => None,
    };
    c.execute_batch("SAVEPOINT index_page_snapshot")?;
    let result = (|| -> Result<Value> {
        let total_opt = if with_total {
            let count: i64 = c.query_row(
                "SELECT count(*) FROM paths p
                JOIN edges e ON e.id=p.edge_id AND e.child_uuid=p.node_uuid
                JOIN memory_rows m ON m.node_uuid=p.node_uuid AND m.deprecated=0
                WHERE p.namespace=?1 AND (?2 IS NULL OR p.domain=?2)",
                params![ns, domain],
                |r| r.get(0),
            )?;
            Some(count as usize)
        } else {
            None
        };
        if limit == Some(0) {
            let has_more = if let Some(total) = total_opt {
                offset < total
            } else {
                c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM paths p
                    JOIN edges e ON e.id=p.edge_id AND e.child_uuid=p.node_uuid
                    JOIN memory_rows m ON m.node_uuid=p.node_uuid AND m.deprecated=0
                    WHERE p.namespace=?1 AND (?2 IS NULL OR p.domain=?2)
                    ORDER BY p.domain,p.path LIMIT 1 OFFSET ?3)",
                    params![ns, domain, offset_i64],
                    |r| r.get(0),
                )?
            };
            return Ok(json!({
                "total": total_opt,
                "offset": offset,
                "limit": limit,
                "has_more": has_more,
                "results": [],
            }));
        }
        let query_limit = match limit_i64 {
            Some(v) => v.saturating_add(1),
            None => -1,
        };
        let mut q = c.prepare(
            "SELECT p.domain,p.path,p.node_uuid,e.priority,e.disclosure,coalesce(substr(CAST(m.content AS BLOB),1,404),x'') FROM paths p
            JOIN edges e ON e.id=p.edge_id AND e.child_uuid=p.node_uuid
            JOIN memory_rows m ON m.node_uuid=p.node_uuid AND m.deprecated=0
            WHERE p.namespace=?1 AND (?2 IS NULL OR p.domain=?2) ORDER BY p.domain,p.path LIMIT ?3 OFFSET ?4",
        )?;
        let mut rows = q
            .query_map(params![ns, domain, query_limit, offset_i64], |r| {
                Ok(json!({
                    "uri":format!("{}://{}",r.get::<_,String>(0)?,r.get::<_,String>(1)?),
                    "node_uuid":r.get::<_,String>(2)?,"priority":r.get::<_,i64>(3)?,
                    "disclosure":r.get::<_,Option<String>>(4)?,"snippet":blob_snippet(r.get::<_,Vec<u8>>(5)?)
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = limit.is_some_and(|lim| rows.len() > lim);
        if let Some(lim) = limit {
            rows.truncate(lim);
        }
        let total = total_opt.or_else(|| {
            if !has_more && (!rows.is_empty() || offset == 0) {
                Some(offset + rows.len())
            } else {
                None
            }
        });
        Ok(json!({
            "total": total,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
            "results": rows,
        }))
    })();
    let released = c.execute_batch("RELEASE index_page_snapshot");
    let value = result?;
    released?;
    Ok(value)
}

fn child_rows(
    c: &Connection, ns: &str, uri: &str, node: &str,
) -> Result<Vec<Value>> {
    let context = Uri::parse(uri)?;
    let mut q = c.prepare(
        "SELECT e.id,e.child_uuid,e.name,e.priority,e.disclosure,p.domain,p.path,m.content
        FROM edges e JOIN paths p ON p.edge_id=e.id AND p.node_uuid=e.child_uuid AND p.namespace=?2
        JOIN memory_rows m ON m.node_uuid=e.child_uuid AND m.deprecated=0
        WHERE e.parent_uuid=?1 AND (?1<>?3 OR (p.domain=?4 AND instr(p.path,'/')=0))
        ORDER BY e.priority,e.name,p.domain,p.path,e.id",
    )?;
    let mut rows = q.query(params![node, ns, ROOT, context.domain])?;
    let mut choices: Vec<(i64, bool, Value)> = Vec::new();
    while let Some(r) = rows.next()? {
        let edge: i64 = r.get(0)?;
        let name: String = r.get(2)?;
        let domain: String = r.get(5)?;
        let path: String = r.get(6)?;
        let desired = if context.path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", context.path, name)
        };
        let preferred = domain == context.domain && path == desired;
        let value = json!({"node_uuid":r.get::<_,String>(1)?,"name":name,"priority":r.get::<_,i64>(3)?,
            "disclosure":r.get::<_,Option<String>>(4)?,"uri":format!("{domain}://{path}"),"domain":domain,"path":path,
            "snippet":snippet(&r.get::<_,String>(7)?)});
        if let Some(existing) =
            choices.iter_mut().find(|(id, _, _)| *id == edge)
        {
            if preferred && !existing.1 {
                *existing = (edge, true, value);
            }
        } else {
            choices.push((edge, preferred, value));
        }
    }
    Ok(choices.into_iter().map(|(_, _, v)| v).collect())
}

pub fn children(c: &Connection, ns: &str, uri: &str) -> Result<Value> {
    let node = graph::resolve(c, ns, uri)?;
    Ok(json!({"context":uri,"children":child_rows(c,ns,uri,&node)?}))
}

// Tree rows never fetch content. Rank aliases before applying the remaining budget.
fn tree_rows(
    c: &Connection, ns: &str, uri: &str, node: &str, limit: usize,
) -> Result<Vec<Value>> {
    let context = Uri::parse(uri)?;
    let mut q = c.prepare(
        "WITH ranked AS (
            SELECT e.id,e.child_uuid,e.name,e.priority,e.disclosure,p.domain,p.path,
            row_number() OVER (PARTITION BY e.id ORDER BY
                (p.domain=?4 AND p.path=CASE WHEN ?5='' THEN e.name ELSE ?5||'/'||e.name END) DESC,
                p.domain,p.path) AS rank
            FROM edges e JOIN paths p ON p.edge_id=e.id AND p.node_uuid=e.child_uuid AND p.namespace=?2
            JOIN memory_rows m ON m.node_uuid=e.child_uuid AND m.deprecated=0
            WHERE e.parent_uuid=?1 AND (?1<>?3 OR (p.domain=?4 AND instr(p.path,'/')=0))
        ) SELECT child_uuid,name,priority,disclosure,domain,path FROM ranked WHERE rank=1
        ORDER BY priority,name,domain,path,id LIMIT ?6",
    )?;
    let rows = q
        .query_map(
            params![
                node,
                ns,
                ROOT,
                context.domain,
                context.path,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |r| {
                let domain: String = r.get(4)?;
                let path: String = r.get(5)?;
                Ok(
                    json!({"node_uuid":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,
            "priority":r.get::<_,i64>(2)?,"disclosure":r.get::<_,Option<String>>(3)?,
            "uri":format!("{domain}://{path}"),"domain":domain,"path":path}),
                )
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn tree(c: &Connection, ns: &str, uri: &str) -> Result<Value> {
    tree_limited(c, ns, uri, None, None)
}

/// Limits count descendants, excluding the context root. None uses safety caps
/// of 128 edges deep and 10,000 descendants; explicit limits cannot raise them.
pub fn tree_limited(
    c: &Connection, ns: &str, uri: &str, depth: Option<usize>,
    limit: Option<usize>,
) -> Result<Value> {
    struct Traversal {
        remaining: usize,
        depth_truncated: bool,
        limit_truncated: bool,
        ancestors: HashSet<String>,
    }
    fn walk(
        c: &Connection, ns: &str, uri: &str, node: &str, depth: usize,
        state: &mut Traversal,
    ) -> Result<Value> {
        if !state.ancestors.insert(node.into()) {
            return Ok(json!([]));
        }
        let rows = tree_rows(
            c,
            ns,
            uri,
            node,
            if depth == 0 {
                1
            } else {
                state.remaining.saturating_add(1)
            },
        )?;
        let mut out = Vec::new();
        for mut child in rows {
            if depth == 0 || state.remaining == 0 {
                state.depth_truncated |= depth == 0;
                state.limit_truncated |= state.remaining == 0;
                break;
            }
            state.remaining -= 1;
            let child_node = child["node_uuid"].as_str().unwrap().to_owned();
            let child_uri = child["uri"].as_str().unwrap().to_owned();
            child["children"] =
                walk(c, ns, &child_uri, &child_node, depth - 1, state)?;
            out.push(child);
        }
        state.ancestors.remove(node);
        Ok(json!(out))
    }
    let node = graph::resolve(c, ns, uri)?;
    let limit = limit.unwrap_or(10000).min(10000);
    let depth = depth.unwrap_or(128).min(128);
    let mut state = Traversal {
        remaining: limit,
        depth_truncated: false,
        limit_truncated: false,
        ancestors: HashSet::new(),
    };
    let children = walk(c, ns, uri, &node, depth, &mut state)?;
    let mut reasons = Vec::new();
    if state.depth_truncated {
        reasons.push("depth");
    }
    if state.limit_truncated {
        reasons.push("limit");
    }
    Ok(
        json!({"context":uri,"children":children,"truncated":!reasons.is_empty(),
        "truncation_reasons":reasons,"depth":depth,"limit":limit,
        "node_count":limit-state.remaining,"count_includes_root":false}),
    )
}

fn configured(
    c: &Connection, include_default: bool,
) -> Result<Option<(i64, Value)>> {
    let row = c
        .query_row(
            "SELECT id,boot_uris FROM presets WHERE is_active=1 OR (?1 AND name='default') ORDER BY is_active DESC LIMIT 1",
            [include_default],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    row.map(|(id, text)| -> Result<_> {
        let value: Value = serde_json::from_str(&text)?;
        if !value.is_object() {
            return Err(Error::Invalid(
                "preset boot_uris must be a namespace map".into(),
            ));
        }
        Ok((id, value))
    })
    .transpose()
}

pub fn boot_uris(c: &Connection, ns: &str) -> Result<Value> {
    let Some((_, map)) = configured(c, false)? else {
        return Ok(json!([]));
    };
    let value = map.get(ns).cloned().unwrap_or(json!([]));
    let uris: Vec<String> = serde_json::from_value(value)?;
    Ok(json!(uris))
}

pub fn set_boot(c: &Connection, ns: &str, uris: &[String]) -> Result<Value> {
    for uri in uris {
        graph::read(c, ns, uri, false)?;
    }
    if let Some((id, mut map)) = configured(c, true)? {
        map[ns] = json!(uris);
        c.execute(
            "UPDATE presets SET boot_uris=?1,is_active=1,updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND (boot_uris<>?1 OR is_active<>1)",
            params![serde_json::to_string(&map)?, id],
        )?;
    } else {
        let mut map = serde_json::Map::new();
        map.insert(ns.into(), json!(uris));
        c.execute(
            "INSERT INTO presets(name,boot_uris,path_masks,is_active) VALUES('default',?1,'[]',1)",
            [serde_json::to_string(&map)?],
        )?;
    }
    Ok(json!({"namespace":ns,"boot_uris":uris}))
}

pub fn boot(c: &Connection, ns: &str) -> Result<Value> {
    boot_with_tracking(c, ns, true)
}

pub fn boot_with_tracking(
    c: &Connection, ns: &str, track: bool,
) -> Result<Value> {
    let uris: Vec<String> = serde_json::from_value(boot_uris(c, ns)?)?;
    let mut loaded = Vec::new();
    let mut failed = Vec::new();
    for uri in uris {
        match graph::read(c, ns, &uri, track) {
            Ok(value) => loaded.push(value),
            Err(Error::NotFound(message)) => {
                failed.push(json!({"uri":uri,"error":message}))
            }
            Err(error) => return Err(error),
        }
    }
    Ok(json!({"loaded":loaded,"failed":failed}))
}
