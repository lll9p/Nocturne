use crate::error::{Error, RelatedChangeset, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

struct Schema {
    columns: &'static [&'static str],
    keys: &'static [&'static str],
}
fn schema(table: &str) -> Result<Schema> {
    let columns: &[&str] = match table {
        "nodes" => &["uuid", "created_at"],
        "node_attrs" => &["node_uuid", "data_json"],
        "memories" => &[
            "id",
            "node_uuid",
            "content_id",
            "deprecated",
            "migrated_to",
            "created_at",
        ],
        "edges" => &[
            "id",
            "parent_uuid",
            "child_uuid",
            "name",
            "priority",
            "disclosure",
            "created_at",
        ],
        "paths" => &[
            "namespace",
            "domain",
            "path",
            "edge_id",
            "node_uuid",
            "created_at",
        ],
        "glossary_keywords" => {
            &["id", "keyword", "node_uuid", "namespace", "created_at"]
        }
        "presets" => &[
            "id",
            "name",
            "boot_uris",
            "path_masks",
            "is_active",
            "created_at",
            "updated_at",
        ],
        _ => return Err(Error::Invalid("invalid audit table".into())),
    };
    Ok(Schema {
        columns,
        keys: match table {
            "nodes" => &["uuid"],
            "node_attrs" => &["node_uuid"],
            "paths" => &["namespace", "domain", "path"],
            _ => &["id"],
        },
    })
}
fn normalize(table: &str, mut value: Value) -> Result<Value> {
    if value.is_null() {
        return Ok(value);
    }
    let s = schema(table)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("invalid audit row".into()))?;
    if object.len() != s.columns.len()
        || s.columns.iter().any(|k| !object.contains_key(*k))
    {
        return Err(Error::Invalid(
            "incomplete or unknown audit columns".into(),
        ));
    }
    for key in s.keys {
        if object[*key].is_null() {
            return Err(Error::Invalid("missing audit primary key".into()));
        }
    }
    for key in ["deprecated", "is_active"] {
        if let Some(v) = object.get_mut(key)
            && let Some(b) = v.as_bool()
        {
            *v = json!(i64::from(b));
        }
    }
    Ok(value)
}
#[derive(Clone)]
struct Row {
    table: String,
    before: Value,
    after: Value,
}
impl Row {
    fn key(&self) -> &Value {
        if self.after.is_null() {
            &self.before
        } else {
            &self.after
        }
    }
    fn predicate(&self) -> Result<String> {
        Ok(schema(&self.table)?
            .keys
            .iter()
            .map(|k| format!("{k} IS json_extract(?1,'$.{k}')"))
            .collect::<Vec<_>>()
            .join(" AND "))
    }
    fn current(&self, c: &Connection) -> Result<Value> {
        let s = schema(&self.table)?;
        let expr = s
            .columns
            .iter()
            .map(|k| format!("'{k}',{k}"))
            .collect::<Vec<_>>()
            .join(",");
        let value = c
            .query_row(
                &format!(
                    "SELECT json_object({expr}) FROM {} WHERE {}",
                    self.table,
                    self.predicate()?
                ),
                [self.key().to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        normalize(
            &self.table,
            value
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null),
        )
    }
}
fn net_rows(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<Vec<Row>> {
    let mut q = c.prepare("SELECT table_name,before_row,after_row FROM change_rows WHERE changeset_id IN (SELECT id FROM changesets WHERE namespace=?3 AND ((NOT ?2 AND id=?1) OR (?2 AND batch=?1))) ORDER BY id")?;
    let records = q
        .query_map(params![id, batch, ns], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut rows: Vec<Row> = Vec::new();
    let mut positions = BTreeMap::new();
    for (table, b, a) in records {
        schema(&table)?;
        let before = normalize(
            &table,
            b.map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null),
        )?;
        let after = normalize(
            &table,
            a.map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null),
        )?;
        if before.is_null() && after.is_null() {
            return Err(Error::Invalid("empty audit row".into()));
        }
        let row = Row {
            table,
            before,
            after,
        };
        let identity = |v: &Value| -> Result<String> {
            Ok(json!([
                row.table,
                schema(&row.table)?
                    .keys
                    .iter()
                    .map(|k| v[*k].clone())
                    .collect::<Vec<_>>()
            ])
            .to_string())
        };
        if !row.before.is_null()
            && !row.after.is_null()
            && identity(&row.before)? != identity(&row.after)?
        {
            return Err(Error::Invalid("audit primary key changed".into()));
        }
        let key = identity(row.key())?;
        if let Some(&index) = positions.get(&key) {
            let previous: &mut Row = &mut rows[index];
            if previous.after != row.before {
                return Err(Error::Conflict(
                    "discontinuous audit history".into(),
                ));
            }
            previous.after = row.after;
        } else {
            positions.insert(key, rows.len());
            rows.push(row);
        }
    }
    rows.retain(|r| r.before != r.after);
    Ok(rows)
}
pub fn list(c: &Connection, ns: &str) -> Result<Value> {
    list_filtered(c, ns, None, None, 0)
}
pub fn list_filtered(
    c: &Connection, ns: &str, status: Option<&str>, limit: Option<usize>,
    offset: usize,
) -> Result<Value> {
    if let Some(s) = status
        && !matches!(s, "pending" | "accepted" | "rolled_back")
    {
        return Err(Error::Invalid(format!("invalid review status: {s:?}")));
    }
    let offset = i64::try_from(offset).map_err(|_| {
        Error::Invalid("review offset exceeds SQLite integer range".into())
    })?;
    let limit = match limit {
        Some(v) => i64::try_from(v).map_err(|_| {
            Error::Invalid("review limit exceeds SQLite integer range".into())
        })?,
        None => -1,
    };
    let mut q = c.prepare(
        "SELECT id,batch,status,namespace,created_at FROM changesets
         WHERE namespace=?1 AND (?2 IS NULL OR status=?2)
         ORDER BY rowid DESC LIMIT ?3 OFFSET ?4",
    )?;
    let rows = q
        .query_map(params![ns, status, limit, offset], |r| {
            Ok(json!({
                "id":r.get::<_,String>(0)?,
                "batch":r.get::<_,Option<String>>(1)?,
                "status":r.get::<_,String>(2)?,
                "namespace":r.get::<_,String>(3)?,
                "created_at":r.get::<_,String>(4)?
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!(rows))
}
pub(crate) fn has_changes(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<bool> {
    Ok(!net_rows(c, ns, id, batch)?.is_empty())
}
pub fn diff(c: &Connection, ns: &str, id: &str, batch: bool) -> Result<Value> {
    states(c, ns, id, batch)?;
    Ok(json!(
        net_rows(c, ns, id, batch)?
            .into_iter()
            .map(|r| Ok(json!({"table":r.table,"before":crate::content::expand(c,r.before)?,"after":crate::content::expand(c,r.after)?})))
            .collect::<Result<Vec<_>>>()?
    ))
}
/// Human-readable review; `diff` remains the complete machine-readable audit snapshot.
pub fn diff_text(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<String> {
    states(c, ns, id, batch)?;
    let mut rows = net_rows(c, ns, id, batch)?;
    for row in &mut rows {
        row.before = crate::content::expand(c, row.before.take())?;
        row.after = crate::content::expand(c, row.after.take())?;
    }
    let mut bodies: BTreeMap<String, [Option<&Value>; 2]> = BTreeMap::new();
    for row in rows.iter().filter(|r| r.table == "memories") {
        for (side, value) in [&row.before, &row.after].into_iter().enumerate() {
            if value["deprecated"].as_i64() == Some(0)
                && let Some(node) = value["node_uuid"].as_str()
            {
                bodies.entry(node.to_owned()).or_default()[side] = Some(value);
            }
        }
    }
    let mut output = String::new();
    for (node, sides) in bodies {
        fn text(v: Option<&Value>) -> &str {
            v.and_then(|v| v["content"].as_str()).unwrap_or("")
        }
        let old = text(sides[0]);
        let new = text(sides[1]);
        // Historical routes survive deletion; current routes must be namespace-scoped.
        let mut uris = std::collections::BTreeSet::new();
        for row in rows.iter().filter(|r| r.table == "paths") {
            for value in [&row.before, &row.after] {
                if value["namespace"].as_str() == Some(ns)
                    && value["node_uuid"].as_str() == Some(&node)
                    && let (Some(domain), Some(path)) =
                        (value["domain"].as_str(), value["path"].as_str())
                {
                    uris.insert(format!("{domain}://{path}"));
                }
            }
        }
        let mut q = c.prepare("SELECT domain,path FROM paths WHERE namespace=?1 AND node_uuid=?2 ORDER BY domain,path")?;
        for uri in q.query_map(params![ns, node], |r| {
            Ok(format!(
                "{}://{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?
            ))
        })? {
            uris.insert(uri?);
        }
        let title = format!(
            "{} [node_uuid={node}]",
            uris.into_iter().collect::<Vec<_>>().join(", ")
        );
        let label = |side: Option<&Value>| match side {
            Some(v) => format!("{title} memoryID={}", v["id"]),
            None => "/dev/null".to_owned(),
        };
        output.push_str(&format!(
            "content: {title}\nversions: {} -> {}\n",
            sides[0]
                .map(|v| v["id"].to_string())
                .unwrap_or_else(|| "(none)".into()),
            sides[1]
                .map(|v| v["id"].to_string())
                .unwrap_or_else(|| "(none)".into())
        ));
        let diff = crate::text_diff::compare(
            old,
            new,
            &label(sides[0]),
            &label(sides[1]),
        )?;
        output.push_str(diff["unified"].as_str().unwrap_or(""));
    }
    // Version bookkeeping and intermediate bodies remain available in the JSON snapshots.
    for row in rows.into_iter().filter(|row| row.table != "memories") {
        output.push_str(&format!("table: {}\n", row.table));
        for (name, value) in [("before", row.before), ("after", row.after)] {
            output.push_str(&format!("{name}:\n"));
            for line in serde_json::to_string_pretty(&value)?.lines() {
                output.push_str("  ");
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    Ok(output)
}
fn states(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<Vec<String>> {
    let mut q =
        c.prepare("SELECT status FROM changesets WHERE namespace=?3 AND ((NOT ?2 AND id=?1) OR (?2 AND batch=?1))")?;
    let rows = q
        .query_map(params![id, batch, ns], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    if rows.is_empty() {
        let message = if batch {
            format!("batch {id:?} has no changesets in namespace {ns:?}")
        } else {
            format!("changeset {id:?} not found in namespace {ns:?}")
        };
        return Err(Error::NotFound(message));
    }
    Ok(rows)
}
pub fn accept(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<Value> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let _guard = crate::store::Rollback(c);
    if states(c, ns, id, batch)?.iter().any(|s| s != "pending") {
        return Err(Error::Conflict(
            "only pending changes can be accepted".into(),
        ));
    }
    let n = c.execute(
        "UPDATE changesets SET status='accepted' WHERE namespace=?3 AND ((NOT ?2 AND id=?1) OR (?2 AND batch=?1))",
        params![id, batch, ns],
    )?;
    c.execute_batch("COMMIT")?;
    Ok(json!({"accepted":n}))
}
pub fn accept_all(
    c: &Connection, ns: &str, all_namespaces: bool,
) -> Result<Value> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let _guard = crate::store::Rollback(c);
    let accepted = c.execute(
        "UPDATE changesets SET status='accepted' WHERE status='pending' AND (?1 OR namespace=?2)",
        params![all_namespaces, ns],
    )?;
    c.execute_batch("COMMIT")?;
    Ok(json!({"accepted":accepted}))
}

fn constraint(e: rusqlite::Error) -> Error {
    match &e {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Error::Conflict(e.to_string())
        }
        _ => e.into(),
    }
}
// Called only after a failed attempt has restored the pre-rollback state.
fn rollback_context(
    c: &Connection, ns: &str, id: &str, batch: bool, rows: &[Row], reason: &str,
) -> Result<Error> {
    use std::collections::BTreeSet;
    fn nodes(row: &Row) -> BTreeSet<String> {
        [&row.before, &row.after]
            .into_iter()
            .flat_map(|v| {
                ["uuid", "node_uuid", "parent_uuid", "child_uuid"]
                    .into_iter()
                    .filter_map(move |k| {
                        v[k].as_str()
                            .filter(|s| *s != crate::store::ROOT)
                            .map(str::to_owned)
                    })
            })
            .collect()
    }
    let affected: BTreeSet<String> = rows.iter().flat_map(nodes).collect();
    let mut uris = BTreeSet::new();
    for row in rows.iter().filter(|row| row.table == "paths") {
        for value in [&row.before, &row.after] {
            if value["namespace"].as_str() == Some(ns)
                && let (Some(domain), Some(path)) =
                    (value["domain"].as_str(), value["path"].as_str())
            {
                uris.insert(format!("{domain}://{path}"));
            }
        }
    }
    let mut related = BTreeSet::new();
    let mut blocked = BTreeSet::new();
    let mut q = c.prepare("SELECT cr.table_name,cr.before_row,cr.after_row,cs.id,cs.batch FROM change_rows cr JOIN changesets cs ON cs.id=cr.changeset_id WHERE cs.namespace=?1 AND cs.status<>'rolled_back' AND cr.id > (SELECT min(cr2.id) FROM change_rows cr2 JOIN changesets cs2 ON cs2.id=cr2.changeset_id WHERE cs2.namespace=?1 AND ((NOT ?3 AND cs2.id=?2) OR (?3 AND cs2.batch IS ?2))) AND NOT ((NOT ?3 AND cs.id=?2) OR (?3 AND cs.batch IS ?2))")?;
    let records = q
        .query_map(params![ns, id, batch], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (table, b, a, change, batch_name) in records {
        let row = Row {
            table,
            before: b
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null),
            after: a
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(Value::Null),
        };
        let linked = nodes(&row);
        if linked.is_disjoint(&affected) || row.current(c)? != row.after {
            continue;
        }
        blocked.extend(linked);
        for v in [&row.before, &row.after] {
            if row.table == "paths"
                && v["namespace"].as_str() == Some(ns)
                && let (Some(d), Some(p)) =
                    (v["domain"].as_str(), v["path"].as_str())
            {
                uris.insert(format!("{d}://{p}"));
            }
        }
        related.insert(RelatedChangeset {
            id: change,
            batch: batch_name,
        });
    }
    if blocked.is_empty() {
        blocked = affected;
    }
    let mut foreign = false;
    for node in blocked {
        let mut q = c.prepare(
            "SELECT namespace,domain,path FROM paths WHERE node_uuid=?1",
        )?;
        for path in q.query_map([&node], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (namespace, domain, path) = path?;
            if namespace == ns {
                uris.insert(format!("{domain}://{path}"));
            } else {
                foreign = true;
            }
        }
    }
    let mut message = reason.to_owned();
    for uri in &uris {
        message.push_str(&format!("\nblocking URI: {uri}"));
    }
    for change in &related {
        message.push_str(&format!(
            "\nrelated changeset {}{}",
            change.id,
            change
                .batch
                .as_ref()
                .map(|b| format!(" (batch {b})"))
                .unwrap_or_default()
        ));
    }
    if foreign {
        message.push_str(
            "\nReferences exist in another namespace (details hidden).",
        );
    }
    message.push_str("\nInspect the blocking references and related changesets before retrying.");
    Ok(Error::RollbackConflict {
        message,
        blocking_uris: uris.into_iter().collect(),
        related_changesets: related.into_iter().collect(),
        foreign_references: foreign,
    })
}
pub fn rollback(
    c: &Connection, ns: &str, id: &str, batch: bool,
) -> Result<Value> {
    c.execute_batch("BEGIN IMMEDIATE; PRAGMA defer_foreign_keys=ON")?;
    let _guard = crate::store::Rollback(c);
    if states(c, ns, id, batch)?.iter().any(|s| s == "rolled_back") {
        return Err(Error::Conflict("already rolled back".into()));
    }
    let rows = net_rows(c, ns, id, batch)?;
    for row in &rows {
        let current = row.current(c)?;
        if current != row.after {
            let evidence = [
                row.clone(),
                Row {
                    table: row.table.clone(),
                    before: Value::Null,
                    after: current,
                },
            ];
            return Err(rollback_context(
                c,
                ns,
                id,
                batch,
                &evidence,
                &format!("later changes to {}", row.table),
            )?);
        }
    }
    c.execute_batch("SAVEPOINT rollback_attempt")?;
    // Suspend only changed rows before restoring partial-unique active flags.
    for row in &rows {
        let flag = match row.table.as_str() {
            "memories" => Some(("deprecated", 1)),
            "presets" => Some(("is_active", 0)),
            _ => None,
        };
        if let Some((flag, value)) = flag {
            c.execute(
                &format!(
                    "UPDATE {} SET {flag}={value} WHERE {}",
                    row.table,
                    row.predicate()?
                ),
                [row.key().to_string()],
            )
            .map_err(constraint)?;
        }
    }
    for row in rows.iter().filter(|r| r.before.is_null()) {
        if row.table == "nodes" {
            c.execute(
                "DELETE FROM memory_access_logs WHERE node_uuid=?1",
                [row.key()["uuid"].as_str().ok_or_else(|| {
                    Error::Invalid("invalid node UUID".into())
                })?],
            )?;
        }
        c.execute(
            &format!("DELETE FROM {} WHERE {}", row.table, row.predicate()?),
            [row.key().to_string()],
        )
        .map_err(constraint)?;
    }
    for row in rows.iter().filter(|r| !r.before.is_null()) {
        let columns = schema(&row.table)?.columns;
        let assignments = columns
            .iter()
            .map(|k| format!("{k}=json_extract(?2,'$.{k}')"))
            .collect::<Vec<_>>()
            .join(",");
        let n = c
            .execute(
                &format!(
                    "UPDATE {} SET {assignments} WHERE {}",
                    row.table,
                    row.predicate()?
                ),
                params![row.key().to_string(), row.before.to_string()],
            )
            .map_err(constraint)?;
        if n == 0 {
            let values = columns
                .iter()
                .map(|k| format!("json_extract(?1,'$.{k}')"))
                .collect::<Vec<_>>()
                .join(",");
            c.execute(
                &format!(
                    "INSERT INTO {}({}) SELECT {values}",
                    row.table,
                    columns.join(",")
                ),
                [row.before.to_string()],
            )
            .map_err(constraint)?;
        }
    }
    let bad_fk = c
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some();
    let mut q=c.prepare("SELECT p.namespace,p.domain,p.path,e.parent_uuid FROM paths p LEFT JOIN edges e ON e.id=p.edge_id WHERE e.id IS NULL OR e.child_uuid<>p.node_uuid OR NOT EXISTS(SELECT 1 FROM memories m WHERE m.node_uuid=p.node_uuid AND m.deprecated=0)")?;
    let mut bad_paths = q.query([])?.next()?.is_some();
    if !bad_paths {
        let mut q=c.prepare("SELECT p.namespace,p.domain,p.path,e.parent_uuid FROM paths p JOIN edges e ON e.id=p.edge_id")?;
        let paths = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (namespace, domain, path, parent_uuid) in paths {
            let valid=match path.rsplit_once('/') {
                None => parent_uuid==crate::store::ROOT,
                Some((parent_path,_)) => parent_uuid!=crate::store::ROOT && c.query_row("SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3 AND node_uuid=?4)",params![namespace,domain,parent_path,parent_uuid],|r|r.get::<_,bool>(0))?,
            };
            if !valid {
                bad_paths = true;
                break;
            }
        }
    }
    let restores_topology = rows.iter().any(|row| {
        row.table == "edges"
            && !row.before.is_null()
            && (row.after.is_null()
                || row.before["parent_uuid"] != row.after["parent_uuid"]
                || row.before["child_uuid"] != row.after["child_uuid"])
    });
    let cyclic = restores_topology
        && c.query_row(crate::graph::CYCLE_COUNT_SQL, [], |r| {
            r.get::<_, i64>(0)
        })? > 0;
    if bad_fk || bad_paths || cyclic {
        // Capture only violating objects before restoring deleted routes for diagnostics.
        let mut evidence = Vec::new();
        if bad_fk {
            let mut q = c.prepare(
                "SELECT \"table\", rowid FROM pragma_foreign_key_check",
            )?;
            let failures = q
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (table, rowid) in failures {
                let columns = schema(&table)?.columns;
                let expr = columns
                    .iter()
                    .map(|k| format!("'{k}',{k}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let value: String = c.query_row(
                    &format!(
                        "SELECT json_object({expr}) FROM {table} WHERE rowid=?1"
                    ),
                    [rowid],
                    |r| r.get(0),
                )?;
                evidence.push(Row {
                    table,
                    before: Value::Null,
                    after: serde_json::from_str(&value)?,
                });
            }
        }
        if bad_paths {
            let mut q = c.prepare("SELECT p.namespace,p.domain,p.path,p.node_uuid,e.parent_uuid FROM paths p LEFT JOIN edges e ON e.id=p.edge_id WHERE e.id IS NULL OR e.child_uuid<>p.node_uuid OR NOT EXISTS(SELECT 1 FROM memories m WHERE m.node_uuid=p.node_uuid AND m.deprecated=0) OR (instr(p.path,'/')=0 AND e.parent_uuid<>?1) OR (instr(p.path,'/')>0 AND NOT EXISTS(SELECT 1 FROM paths parent WHERE parent.namespace=p.namespace AND parent.domain=p.domain AND parent.node_uuid=e.parent_uuid AND substr(p.path,1,length(parent.path)+1)=parent.path||'/' AND instr(substr(p.path,length(parent.path)+2),'/')=0))")?;
            evidence.extend(q.query_map([crate::store::ROOT], |r| Ok(Row {
                table: "paths".into(), before: Value::Null,
                after: json!({"namespace":r.get::<_,String>(0)?,"domain":r.get::<_,String>(1)?,"path":r.get::<_,String>(2)?,"node_uuid":r.get::<_,String>(3)?,"parent_uuid":r.get::<_,Option<String>>(4)?}),
            }))?.collect::<std::result::Result<Vec<_>,_>>()?);
        }
        if cyclic {
            let mut q = c.prepare("WITH RECURSIVE reach(a,b) AS (SELECT parent_uuid,child_uuid FROM edges UNION SELECT r.a,e.child_uuid FROM reach r JOIN edges e ON e.parent_uuid=r.b) SELECT e.parent_uuid,e.child_uuid FROM edges e WHERE EXISTS(SELECT 1 FROM reach WHERE a=e.child_uuid AND b=e.parent_uuid)")?;
            evidence.extend(q.query_map([], |r| Ok(Row {
                table: "edges".into(), before: Value::Null,
                after: json!({"parent_uuid":r.get::<_,String>(0)?,"child_uuid":r.get::<_,String>(1)?}),
            }))?.collect::<std::result::Result<Vec<_>,_>>()?);
        }
        c.execute_batch("ROLLBACK TO rollback_attempt")?;
        return Err(rollback_context(
            c,
            ns,
            id,
            batch,
            &evidence,
            "rollback would break surviving references, parent routes, or graph acyclicity",
        )?);
    }
    crate::search::rebuild(c, None)?;
    c.execute(
        "UPDATE changesets SET status='rolled_back' WHERE namespace=?3 AND ((NOT ?2 AND id=?1) OR (?2 AND batch=?1))",
        params![id, batch, ns],
    )?;
    c.execute_batch("COMMIT").map_err(constraint)?;
    Ok(json!({"rolled_back":id}))
}
