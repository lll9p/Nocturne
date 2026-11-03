use crate::{
    error::{Error, Result},
    store::ROOT,
    uri::{Uri, validate_name},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub(crate) const CYCLE_COUNT_SQL: &str = "WITH RECURSIVE reach(a,b) AS (SELECT parent_uuid,child_uuid FROM edges UNION SELECT r.a,e.child_uuid FROM reach r JOIN edges e ON e.parent_uuid=r.b) SELECT count(*) FROM reach WHERE a=b";
pub fn resolve(c: &Connection, ns: &str, uri: &str) -> Result<String> {
    let u = Uri::parse(uri)?;
    if u.path.is_empty() {
        return Ok(ROOT.into());
    }
    c.query_row(
        "SELECT node_uuid FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3",
        params![ns, u.domain, u.path],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("URI {uri} not found in namespace {ns:?}")))
}
pub fn read(c: &Connection, ns: &str, uri: &str, track: bool) -> Result<Value> {
    read_with_children(c, ns, uri, track, true)
}
pub fn read_with_children(
    c: &Connection, ns: &str, uri: &str, track: bool, include_children: bool,
) -> Result<Value> {
    let id = resolve(c, ns, uri)?;
    let mut v=c.query_row("SELECT id,content FROM memories WHERE node_uuid=?1 AND deprecated=0",[&id],|r|Ok(json!({"node_uuid":id,"version":r.get::<_,i64>(0)?,"content":r.get::<_,String>(1)?,"uri":uri}))).optional()?.ok_or_else(||Error::NotFound(format!("URI {uri} not found in namespace {ns:?}")))?;
    let u = Uri::parse(uri)?;
    let metadata=c.query_row("SELECT e.priority,e.disclosure FROM paths p JOIN edges e ON e.id=p.edge_id WHERE p.namespace=?1 AND p.domain=?2 AND p.path=?3",params![ns,u.domain,u.path],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?))).optional()?;
    if let Some((priority, disclosure)) = metadata {
        v["priority"] = json!(priority);
        v["disclosure"] = json!(disclosure);
    }
    v["attrs"] = crate::attrs::for_node(c, &id)?;
    if include_children {
        v["children"] = crate::views::children(c, ns, uri)?;
    }
    v["associations"] =
        crate::glossary::scan(c, ns, v["content"].as_str().unwrap())?;
    if track {
        c.execute_batch("BEGIN IMMEDIATE")?;
        let _guard = crate::store::Rollback(c);
        let result = (|| -> Result<()> {
            c.execute(
                "UPDATE nodes SET last_accessed_at=CURRENT_TIMESTAMP WHERE uuid=?1",
                [&id],
            )?;
            c.execute(
                "INSERT INTO memory_access_logs(node_uuid,namespace,context) VALUES(?1,?2,?3)",
                params![id, ns, uri],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => c.execute_batch("COMMIT")?,
            Err(e) => {
                c.execute_batch("ROLLBACK")?;
                return Err(e);
            }
        }
    }
    Ok(v)
}
pub fn create(
    c: &Connection, ns: &str, parent: &str, title: &str, content: &str,
    priority: i64, disclosure: Option<&str>,
) -> Result<Value> {
    let record =
        create_record(c, ns, parent, title, content, priority, disclosure)?;
    read(c, ns, record["uri"].as_str().unwrap(), false)
}
pub(crate) fn create_record(
    c: &Connection, ns: &str, parent: &str, title: &str, content: &str,
    priority: i64, disclosure: Option<&str>,
) -> Result<Value> {
    validate_name(title, "title")?;
    validate_priority(priority)?;
    if Uri::parse(parent)?.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let p = resolve(c, ns, parent)?;
    let u = Uri::parse(parent)?;
    let path = if u.path.is_empty() {
        title.into()
    } else {
        format!("{}/{}", u.path, title)
    };
    require_unused_path(c, ns, &u.domain, &path)?;
    let id = uuid::Uuid::new_v4().to_string();
    c.execute("INSERT INTO nodes(uuid) VALUES(?1)", [&id])?;
    c.execute(
        "INSERT INTO memories(node_uuid,content) VALUES(?1,?2)",
        params![id, content],
    )?;
    let version = c.last_insert_rowid();
    c.execute(
        "INSERT INTO edges(parent_uuid,child_uuid,name,priority,disclosure) VALUES(?1,?2,?3,?4,?5)",
        params![p, id, title, priority, disclosure],
    )?;
    let edge = c.last_insert_rowid();
    c.execute(
        "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",
        params![ns, u.domain, path, edge, id],
    )?;
    Ok(
        json!({"uri":format!("{}://{}", u.domain, path),"node_uuid":id,"version":version}),
    )
}
pub fn update(
    c: &Connection, ns: &str, uri: &str, content: &str, expect: Option<i64>,
) -> Result<Value> {
    edit(c, ns, uri, Some(content), None, None, expect)
}

pub fn edit(
    c: &Connection, ns: &str, uri: &str, content: Option<&str>,
    priority: Option<i64>, disclosure: Option<&str>, expected: Option<i64>,
) -> Result<Value> {
    if content.is_none() && priority.is_none() && disclosure.is_none() {
        return Err(Error::Invalid("edit requires a field".into()));
    }
    if let Some(priority) = priority {
        validate_priority(priority)?;
    }
    if Uri::parse(uri)?.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let id = resolve(c, ns, uri)?;
    let old: i64 = c.query_row(
        "SELECT id FROM memories WHERE node_uuid=?1 AND deprecated=0",
        [&id],
        |r| r.get(0),
    )?;
    if let Some(expected) = expected.filter(|e| *e != old) {
        return Err(Error::VersionConflict {
            uri: uri.into(),
            namespace: ns.into(),
            expected,
            current: old,
        });
    }
    if priority.is_some() || disclosure.is_some() {
        meta(c, ns, uri, priority, disclosure)?;
    }
    let Some(content) = content else {
        return read(c, ns, uri, false);
    };
    let previous: String =
        c.query_row("SELECT content FROM memories WHERE id=?1", [old], |r| {
            r.get(0)
        })?;
    if previous == content {
        return read(c, ns, uri, false);
    }
    c.execute("UPDATE memories SET deprecated=1 WHERE id=?1", [old])?;
    c.execute(
        "INSERT INTO memories(node_uuid,content) VALUES(?1,?2)",
        params![id, content],
    )?;
    let new = c.last_insert_rowid();
    c.execute(
        "UPDATE memories SET migrated_to=?1 WHERE id=?2",
        params![new, old],
    )?;
    read(c, ns, uri, false)
}
pub fn history(c: &Connection, ns: &str, uri: &str) -> Result<Value> {
    let id = resolve(c, ns, uri)?;
    let mut q=c.prepare("SELECT id,content,deprecated,migrated_to FROM memories WHERE node_uuid=?1 ORDER BY id DESC")?;
    let rows=q.query_map([id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"content":r.get::<_,String>(1)?,"deprecated":r.get::<_,bool>(2)?,"migrated_to":r.get::<_,Option<i64>>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    Ok(json!(rows))
}
pub fn meta(
    c: &Connection, ns: &str, uri: &str, priority: Option<i64>,
    disclosure: Option<&str>,
) -> Result<Value> {
    if priority.is_none() && disclosure.is_none() {
        return Err(Error::Invalid("meta requires a field".into()));
    }
    if let Some(priority) = priority {
        validate_priority(priority)?;
    }
    let u = Uri::parse(uri)?;
    if u.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let n=c.execute("UPDATE edges SET priority=coalesce(?4,priority),disclosure=coalesce(?5,disclosure) WHERE id=(SELECT edge_id FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",params![ns,u.domain,u.path,priority,disclosure])?;
    if n == 0 {
        return Err(Error::NotFound(format!(
            "URI {uri} not found in namespace {ns:?}"
        )));
    }
    Ok(json!({"updated":uri}))
}
pub fn alias(
    c: &Connection, ns: &str, destination: &str, source: &str,
) -> Result<Value> {
    alias_with_meta(c, ns, destination, source, None, None)
}

pub fn alias_with_meta(
    c: &Connection, ns: &str, destination: &str, source: &str,
    priority: Option<i64>, disclosure: Option<&str>,
) -> Result<Value> {
    if let Some(priority) = priority {
        validate_priority(priority)?;
    }
    if Uri::parse(source)?.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let node = resolve(c, ns, source)?;
    let dst = Uri::parse(destination)?;
    if dst.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    if dst.path.is_empty() {
        return Err(Error::Invalid("alias needs a name".into()));
    }
    if source == destination {
        return Err(Error::Conflict(format!(
            "cannot alias URI {source} to itself"
        )));
    }
    require_unused_path(c, ns, &dst.domain, &dst.path)?;
    let (parent_path, name) =
        dst.path.rsplit_once('/').unwrap_or(("", &dst.path));
    validate_name(name, "alias name")?;
    let parent = resolve(c, ns, &format!("{}://{}", dst.domain, parent_path))?;
    let cycle:bool=c.query_row("WITH RECURSIVE descendants(uuid) AS (SELECT ?1 UNION SELECT e.child_uuid FROM edges e JOIN descendants d ON e.parent_uuid=d.uuid) SELECT EXISTS(SELECT 1 FROM descendants WHERE uuid=?2)",params![node,parent],|r|r.get(0))?;
    if cycle {
        return Err(Error::Conflict("alias would create a cycle".into()));
    }
    c.execute("INSERT INTO edges(parent_uuid,child_uuid,name) VALUES(?1,?2,?3) ON CONFLICT(parent_uuid,child_uuid) DO NOTHING",params![parent,node,name])?;
    let edge: i64 = c.query_row(
        "SELECT id FROM edges WHERE parent_uuid=?1 AND child_uuid=?2",
        params![parent, node],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",
        params![ns, dst.domain, dst.path, edge, node],
    )?;
    if priority.is_some() || disclosure.is_some() {
        meta(c, ns, destination, priority, disclosure)?;
    }
    derive(c, ns, &dst.domain, &dst.path, &node)?;
    Ok(json!({"uri":destination,"node_uuid":node}))
}
fn derive(
    c: &Connection, ns: &str, domain: &str, path: &str, node: &str,
) -> Result<()> {
    let mut q =
        c.prepare("SELECT e.id,e.child_uuid,e.name FROM edges e WHERE e.parent_uuid=?1 AND EXISTS(SELECT 1 FROM paths p WHERE p.edge_id=e.id AND p.namespace=?2) AND EXISTS(SELECT 1 FROM memories m WHERE m.node_uuid=e.child_uuid AND m.deprecated=0) ORDER BY e.id")?;
    let rows = q
        .query_map(params![node, ns], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (edge, child, name) in rows {
        let base = format!("{path}/{name}");
        let mut p = base.clone();
        let mut suffix = 2;
        while c.query_row(
            "SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",
            params![ns, domain, p],
            |r| r.get::<_, bool>(0),
        )? {
            p = format!("{base}-{suffix}");
            suffix += 1;
        }
        c.execute(
            "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",
            params![ns, domain, p, edge, child],
        )?;
        derive(c, ns, domain, &p, &child)?;
    }
    Ok(())
}
pub fn restore(
    c: &Connection, ns: &str, uri: &str, version: i64, expect: Option<i64>,
) -> Result<Value> {
    let node = resolve(c, ns, uri)?;
    let current: Option<i64> = c.query_row(
        "SELECT max(id) FROM memories WHERE node_uuid=?1 AND deprecated=0",
        [&node],
        |r| r.get(0),
    )?;
    let text = c
        .query_row(
            "SELECT content FROM memories WHERE id=?1 AND node_uuid=?2",
            params![version, node],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            Error::NotFound(format!("URI {uri} in namespace {ns:?}: requested version {version} not found; current version {current:?}"))
        })?;
    update(c, ns, uri, &text, expect)
}
pub fn unlink(c: &Connection, ns: &str, uri: &str) -> Result<Value> {
    let u = Uri::parse(uri)?;
    if u.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    if u.path.is_empty() {
        return Err(Error::Invalid("cannot unlink root".into()));
    }
    let target = resolve(c, ns, uri)?;
    let pattern = format!("{}/", u.path);
    let mut q=c.prepare("SELECT node_uuid FROM paths WHERE namespace=?1 AND domain=?2 AND (path=?3 OR substr(path,1,length(?4))=?4) GROUP BY node_uuid ORDER BY min(length(path))")?;
    let nodes = q
        .query_map(params![ns, u.domain, u.path, pattern], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    c.execute("DELETE FROM paths WHERE namespace=?1 AND domain=?2 AND (path=?3 OR substr(path,1,length(?4))=?4)",params![ns,u.domain,u.path,pattern])?;
    for node in &nodes {
        let remaining: i64 = c.query_row(
            "SELECT count(*) FROM paths WHERE node_uuid=?1",
            [node],
            |r| r.get(0),
        )?;
        if remaining == 0 {
            let mut q=c.prepare("SELECT p.namespace,p.domain,p.path,e.id,e.name FROM edges e JOIN paths p ON p.node_uuid=e.parent_uuid WHERE e.child_uuid=?1 AND p.namespace=?2 ORDER BY p.namespace,p.domain,p.path,e.id")?;
            let routes = q
                .query_map(params![node, ns], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (n, d, p, e, name) in routes {
                if node == &target {
                    continue;
                }
                let base = format!("{p}/{name}");
                let mut route = base.clone();
                let mut suffix = 2;
                while c.query_row("SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",params![n,d,route],|r|r.get::<_,bool>(0))?{route=format!("{base}-{suffix}");suffix+=1;}
                c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",params![n,d,route,e,node])?;
            }
            let remains: i64 = c.query_row(
                "SELECT count(*) FROM paths WHERE node_uuid=?1",
                [node],
                |r| r.get(0),
            )?;
            if remains == 0 && node != &target {
                return Err(Error::Conflict(
                    "unlink would orphan a child".into(),
                ));
            }
            if remains == 0 {
                c.execute(
                    "UPDATE memories SET deprecated=1 WHERE node_uuid=?1 AND deprecated=0",
                    [node],
                )?;
            }
        }
    }
    c.execute("DELETE FROM edges WHERE NOT EXISTS(SELECT 1 FROM paths p WHERE p.edge_id=edges.id) AND (parent_uuid=?1 OR child_uuid=?1)",[&target])?;
    Ok(json!({"unlinked":uri}))
}

fn require_unused_path(
    c: &Connection, ns: &str, domain: &str, path: &str,
) -> Result<()> {
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",
        params![ns, domain, path],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict(format!(
            "URI already exists: {domain}://{path}"
        )));
    }
    Ok(())
}

pub fn validate_priority(priority: i64) -> Result<()> {
    if !(0..=999).contains(&priority) {
        return Err(Error::Invalid("priority must be in 0..=999".into()));
    }
    Ok(())
}
