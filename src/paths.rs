use crate::{
    error::{Error, Result},
    graph,
    store::ROOT,
    uri::{Uri, validate_name},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Execute inside Store::write; this function does not own the transaction.
pub fn rename(
    c: &Connection, ns: &str, source: &str, new_name: &str,
) -> Result<Value> {
    validate_name(new_name, "name")?;
    let src = Uri::parse(source)?;
    let parent = src
        .path
        .rsplit_once('/')
        .map(|(p, _)| format!("{p}/"))
        .unwrap_or_default();
    move_path(
        c,
        ns,
        source,
        &format!("{}://{parent}{new_name}", src.domain),
    )
}

pub fn move_path(
    c: &Connection, ns: &str, source: &str, destination: &str,
) -> Result<Value> {
    let src = Uri::parse(source)?;
    let dst = Uri::parse(destination)?;
    if src.domain == "system" || dst.domain == "system" {
        return Err(Error::Invalid("cannot move system paths".into()));
    }
    if src.path.is_empty() {
        return Err(Error::Invalid(format!(
            "cannot move source root {source}"
        )));
    }
    if dst.path.is_empty() {
        let basename = src.path.rsplit('/').next().unwrap();
        return Err(Error::Invalid(format!(
            "destination {destination} is a root; include a name, e.g. archive://{basename}"
        )));
    }
    validate_name(dst.path.rsplit('/').next().unwrap(), "destination name")?;
    let node = graph::resolve(c, ns, source)?;
    if node == ROOT {
        return Err(Error::Invalid("cannot move root".into()));
    }
    if source == destination {
        return Ok(
            json!({"source":source,"uri":destination,"node_uuid":node,"moved_paths":0,"boot_presets_updated":0}),
        );
    }
    let (parent_path, name) =
        dst.path.rsplit_once('/').unwrap_or(("", &dst.path));
    let parent =
        graph::resolve(c, ns, &format!("{}://{parent_path}", dst.domain))?;
    let cycle: bool = c.query_row("WITH RECURSIVE descendants(uuid) AS (SELECT ?1 UNION SELECT e.child_uuid FROM edges e JOIN descendants d ON e.parent_uuid=d.uuid) SELECT EXISTS(SELECT 1 FROM descendants WHERE uuid=?2)", params![node,parent], |r| r.get(0))?;
    if cycle {
        return Err(Error::Conflict("move would create a cycle".into()));
    }
    let prefix = format!("{}/", src.path);
    let mut q = c.prepare("SELECT path,edge_id,node_uuid,created_at FROM paths WHERE namespace=?1 AND domain=?2 AND (path=?3 OR substr(path,1,length(?4))=?4) ORDER BY length(path),path")?;
    let rows = q
        .query_map(params![ns, src.domain, src.path, prefix], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut routes = BTreeMap::new();
    for (path, _, _, _) in &rows {
        let new_path = format!("{}{}", dst.path, &path[src.path.len()..]);
        let exists: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",
            params![ns, dst.domain, new_path],
            |r| r.get(0),
        )?;
        if exists {
            return Err(Error::Conflict(format!(
                "URI already exists: {}://{new_path}",
                dst.domain
            )));
        }
        routes.insert(
            format!("{}://{path}", src.domain),
            format!("{}://{new_path}", dst.domain),
        );
    }
    let old_edge = rows[0].1;
    let (_, priority, disclosure): (String, i64, Option<String>) = c
        .query_row(
            "SELECT parent_uuid,priority,disclosure FROM edges WHERE id=?1",
            [old_edge],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
    // Plan all preset edits before changing graph rows, including inactive presets.
    let boot = boot_updates(c, ns, &routes)?;
    let references: i64 = c.query_row(
        "SELECT count(*) FROM paths WHERE edge_id=?1",
        [old_edge],
        |r| r.get(0),
    )?;
    let moved = rows.iter().filter(|r| r.1 == old_edge).count() as i64;
    let edge = if references == moved {
        c.execute(
            "UPDATE edges SET parent_uuid=?1,name=?2 WHERE id=?3",
            params![parent, name, old_edge],
        )?;
        old_edge
    } else {
        c.execute(
            "INSERT INTO edges(parent_uuid,child_uuid,name,priority,disclosure) VALUES(?1,?2,?3,?4,?5)",
            params![parent, node, name, priority, disclosure],
        )?;
        c.last_insert_rowid()
    };
    for (path, _, _, _) in &rows {
        c.execute(
            "DELETE FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3",
            params![ns, src.domain, path],
        )?;
    }
    for (path, child_edge, child, created) in &rows {
        let new_path = format!("{}{}", dst.path, &path[src.path.len()..]);
        let route_edge = if path == &src.path { edge } else { *child_edge };
        c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![ns,dst.domain,new_path,route_edge,child,created])?;
    }
    c.execute(
        "DELETE FROM edges WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM paths WHERE edge_id=?1)",
        [old_edge],
    )?;
    for (id, text) in &boot {
        c.execute(
            "UPDATE presets SET boot_uris=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2",
            params![text, id],
        )?;
    }
    Ok(
        json!({"source":source,"uri":destination,"node_uuid":node,"moved_paths":rows.len(),"boot_presets_updated":boot.len()}),
    )
}

fn boot_updates(
    c: &Connection, ns: &str, routes: &BTreeMap<String, String>,
) -> Result<Vec<(i64, String)>> {
    let mut q = c.prepare("SELECT id,boot_uris FROM presets ORDER BY id")?;
    let presets = q
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut updates = Vec::new();
    for (id, text) in presets {
        let mut map: serde_json::Map<String, Value> =
            serde_json::from_str(&text)?;
        for value in map.values() {
            let _: Vec<String> = serde_json::from_value(value.clone())?;
        }
        let Some(original) = map.get(ns).cloned() else {
            continue;
        };
        let uris: Vec<String> = serde_json::from_value(original)?;
        let rewritten: Vec<String> = uris
            .iter()
            .map(|u| routes.get(u).unwrap_or(u).clone())
            .collect();
        if uris == rewritten {
            continue;
        }
        map.insert(ns.into(), json!(rewritten));
        updates.push((id, serde_json::to_string(&map)?));
    }
    Ok(updates)
}
