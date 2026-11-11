use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

// An unlinked node belongs to its last effective route deletion, not its logs.
const VISIBLE: &str =
    "(?2 OR EXISTS(SELECT 1 FROM paths p WHERE p.node_uuid=m.node_uuid AND p.namespace=?1)
 OR (NOT EXISTS(SELECT 1 FROM paths p WHERE p.node_uuid=m.node_uuid)
 AND ?1=(SELECT json_extract(r.before_row,'$.namespace') FROM change_rows r
 JOIN changesets s ON s.id=r.changeset_id WHERE r.table_name='paths' AND r.after_row IS NULL
 AND json_extract(r.before_row,'$.node_uuid')=m.node_uuid AND s.status<>'rolled_back'
 ORDER BY r.id DESC LIMIT 1)))";

fn transaction(
    c: &Connection, apply: bool, f: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    c.execute_batch("BEGIN IMMEDIATE")?;
    let _guard = crate::store::Rollback(c);
    let value = f()?;
    c.execute_batch(if apply { "COMMIT" } else { "ROLLBACK" })?;
    Ok(value)
}

pub fn deprecated(
    c: &Connection, ns: &str, all_namespaces: bool,
) -> Result<Value> {
    transaction(c, false, || {
        let mut q = c.prepare(&format!("SELECT m.id,m.node_uuid,length(CAST(m.content AS BLOB)),m.migrated_to,m.created_at FROM memory_rows m WHERE m.deprecated=1 AND {VISIBLE} ORDER BY m.id DESC"))?;
        let rows = q
            .query_map(params![ns, all_namespaces], |r| {
                Ok(json!({
                    "memory_id":r.get::<_,i64>(0)?,"node_uuid":r.get::<_,String>(1)?,
                    "bytes":r.get::<_,i64>(2)?,"migrated_to":r.get::<_,Option<i64>>(3)?,
                    "created_at":r.get::<_,String>(4)?
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    })
}

struct Audit {
    set: String,
    batch: Option<String>,
    namespace: String,
    status: String,
    table: String,
    before: Value,
    after: Value,
}
fn nodes(row: &Value) -> BTreeSet<String> {
    ["node_uuid", "uuid", "parent_uuid", "child_uuid"]
        .iter()
        .filter_map(|key| row[*key].as_str())
        .filter(|node| *node != crate::store::ROOT)
        .map(str::to_owned)
        .collect()
}
fn identity(table: &str, row: &Value) -> Option<String> {
    if row.is_null() {
        return None;
    }
    Some(
        match table {
            "paths" => {
                json!([table, row["namespace"], row["domain"], row["path"]])
            }
            "nodes" => json!([table, row["uuid"]]),
            "node_attrs" => json!([table, row["node_uuid"]]),
            _ => json!([table, row["id"]]),
        }
        .to_string(),
    )
}

pub fn purge(
    c: &Connection, ns: &str, memory_id: i64, all_namespaces: bool, apply: bool,
) -> Result<Value> {
    transaction(c, apply, || {
        let (node, bytes, old, successor): (String,i64,bool,Option<i64>) = c.query_row(
            &format!("SELECT m.node_uuid,length(CAST(m.content AS BLOB)),m.deprecated,m.migrated_to FROM memory_rows m WHERE m.id=?3 AND {VISIBLE}"),
            params![ns,all_namespaces,memory_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            .optional()?.ok_or_else(|| Error::NotFound(if all_namespaces {
                format!("memory version {memory_id} not found across all namespaces")
            } else {
                format!("memory version {memory_id} not found in namespace {ns:?}")
            }))?;
        if !old {
            return Err(Error::Invalid(
                "only deprecated memories can be purged".into(),
            ));
        }
        let shared: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM paths WHERE node_uuid=?1 AND namespace<>?2)",
            params![node, ns],
            |r| r.get(0),
        )?;
        if shared && !all_namespaces {
            return Err(Error::Conflict(
                "node is shared across namespaces; purge requires --all-namespaces".into(),
            ));
        }
        let mut q = c.prepare(
            "SELECT id FROM memory_rows WHERE migrated_to=?1 ORDER BY id",
        )?;
        let predecessors = q
            .query_map([memory_id], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut protected =
            predecessors.iter().copied().collect::<BTreeSet<_>>();
        protected.insert(memory_id);
        let mut q = c.prepare("SELECT s.id,s.batch,s.status,r.table_name,r.before_row,r.after_row,s.namespace FROM changesets s JOIN change_rows r ON r.changeset_id=s.id ORDER BY r.id")?;
        let raw = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut audit = Vec::new();
        for (set, batch, status, table, before, after, namespace) in raw {
            audit.push(Audit {
                set,
                batch,
                namespace,
                status,
                table,
                before: before
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?
                    .unwrap_or(Value::Null),
                after: after
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?
                    .unwrap_or(Value::Null),
            });
        }
        let node_gc: bool = c.query_row("SELECT NOT EXISTS(SELECT 1 FROM memory_rows WHERE node_uuid=?1 AND id<>?2) AND NOT EXISTS(SELECT 1 FROM paths WHERE node_uuid=?1)", params![node,memory_id], |r|r.get(0))?;
        let mut gc_nodes = BTreeSet::from([node.clone()]);
        let mut edge_count = 0i64;
        let mut keyword_count = 0i64;
        let mut keyword_namespaces = Vec::new();
        let mut log_count = 0i64;
        let mut attrs_count = 0i64;
        if node_gc {
            attrs_count = c.query_row(
                "SELECT count(*) FROM node_attrs WHERE node_uuid=?1",
                [&node],
                |r| r.get(0),
            )?;
            let used: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM paths p JOIN edges e ON e.id=p.edge_id WHERE e.parent_uuid=?1 OR e.child_uuid=?1)",[&node],|r|r.get(0))?;
            if used {
                return Err(Error::Conflict(
                    "node GC would affect a routed edge".into(),
                ));
            }
            let mut q = c.prepare(
                "SELECT parent_uuid,child_uuid FROM edges WHERE parent_uuid=?1 OR child_uuid=?1",
            )?;
            let endpoints = q
                .query_map([&node], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            edge_count = endpoints.len() as i64;
            for (parent, child) in endpoints {
                if parent != crate::store::ROOT {
                    gc_nodes.insert(parent);
                }
                if child != crate::store::ROOT {
                    gc_nodes.insert(child);
                }
            }
            let mut q = c.prepare("SELECT namespace,keyword FROM glossary_keywords WHERE node_uuid=?1 ORDER BY namespace")?;
            let mut cjk_namespaces = BTreeSet::new();
            for row in q.query_map([&node], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })? {
                let (kw_ns, kw) = row?;
                if crate::search::is_cjk_dictionary_word(&kw) {
                    cjk_namespaces.insert(kw_ns);
                }
            }
            keyword_namespaces = cjk_namespaces.into_iter().collect();
            keyword_count = c.query_row(
                "SELECT count(*) FROM glossary_keywords WHERE node_uuid=?1",
                [&node],
                |r| r.get(0),
            )?;
            log_count = c.query_row(
                "SELECT count(*) FROM memory_access_logs WHERE node_uuid=?1",
                [&node],
                |r| r.get(0),
            )?;
        }
        let relevant = |a: &Audit| {
            [&a.before, &a.after].iter().any(|v| {
                !nodes(v).is_disjoint(&gc_nodes)
                    || (a.table == "memories"
                        && (v["id"]
                            .as_i64()
                            .is_some_and(|id| protected.contains(&id))
                            || v["migrated_to"].as_i64() == Some(memory_id)))
            })
        };
        if audit.iter().any(|a| a.status == "pending" && relevant(a)) {
            return Err(Error::Conflict(format!(
                "cannot purge memory version {memory_id}: pending changes reference this version, its version chain, or related nodes"
            )));
        }
        // Remove complete handled groups, including predecessor snapshots that could undo repair.
        let pruned: BTreeSet<_> = audit
            .iter()
            .filter(|a| {
                a.status != "pending"
                    && ((node_gc
                        && [&a.before, &a.after]
                            .iter()
                            .any(|v| nodes(v).contains(&node)))
                        || (a.table == "memories"
                            && [&a.before, &a.after].iter().any(|v| {
                                v["id"]
                                    .as_i64()
                                    .is_some_and(|id| protected.contains(&id))
                                    || v["migrated_to"].as_i64()
                                        == Some(memory_id)
                            })))
            })
            .map(|a| a.set.clone())
            .collect();
        // Whole audit groups may also carry the ownership evidence of other orphans.
        let mut owner_nodes: BTreeSet<String> = audit
            .iter()
            .filter(|a| {
                pruned.contains(&a.set)
                    && a.table == "paths"
                    && a.after.is_null()
            })
            .filter_map(|a| a.before["node_uuid"].as_str().map(str::to_owned))
            .collect();
        if node_gc {
            owner_nodes.remove(&node);
        } else {
            owner_nodes.insert(node.clone());
        }
        for owner_node in owner_nodes {
            let unlinked_with_remaining: bool = c.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM paths WHERE node_uuid=?1) AND EXISTS(SELECT 1 FROM memory_rows WHERE node_uuid=?1 AND id<>?2)",
                params![owner_node,memory_id], |r|r.get(0))?;
            if !unlinked_with_remaining {
                continue;
            }
            let latest_owner = |exclude_pruned: bool| {
                audit
                    .iter()
                    .rev()
                    .find(|a| {
                        a.table == "paths"
                            && a.status != "rolled_back"
                            && a.after.is_null()
                            && a.before["node_uuid"].as_str()
                                == Some(owner_node.as_str())
                            && (!exclude_pruned || !pruned.contains(&a.set))
                    })
                    .and_then(|a| a.before["namespace"].as_str())
            };
            if latest_owner(false).is_some()
                && latest_owner(false) != latest_owner(true)
            {
                return Err(Error::Conflict(format!(
                    "purge would change the namespace ownership of remaining versions for node {owner_node} by removing their audit records"
                )));
            }
        }
        if !all_namespaces {
            let foreign_audit = audit
                .iter()
                .any(|a| pruned.contains(&a.set) && a.namespace != ns);
            let foreign_gc: bool = node_gc && c.query_row(
                "SELECT EXISTS(SELECT 1 FROM glossary_keywords WHERE node_uuid=?1 AND namespace<>?2) OR EXISTS(SELECT 1 FROM memory_access_logs WHERE node_uuid=?1 AND namespace<>?2)",
                params![node,ns], |r|r.get(0))?;
            if foreign_audit || foreign_gc {
                return Err(Error::Conflict("purge affects another namespace's review history or node data; requires --all-namespaces".into()));
            }
        }
        let mut affected_nodes = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut batches = BTreeSet::new();
        for a in audit.iter().filter(|a| pruned.contains(&a.set)) {
            if let Some(b) = &a.batch {
                batches.insert((a.namespace.clone(), b.clone()));
            }
            for v in [&a.before, &a.after] {
                affected_nodes.extend(nodes(v));
                if let Some(key) = identity(&a.table, v) {
                    identities.insert(key);
                }
            }
        }
        let pending_batch: bool = {
            let mut q = c.prepare(
                "SELECT namespace,batch FROM changesets WHERE status='pending' AND batch IS NOT NULL",
            )?;
            let values = q
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            values.iter().any(|b| batches.contains(b))
        };
        if pending_batch
            || audit.iter().filter(|a| a.status == "pending").any(|a| {
                [&a.before, &a.after].iter().any(|v| {
                    !nodes(v).is_disjoint(&affected_nodes)
                        || identity(&a.table, v)
                            .is_some_and(|k| identities.contains(&k))
                })
            })
        {
            return Err(Error::Conflict(format!(
                "cannot purge memory version {memory_id}: the review history to be removed is linked to pending changes"
            )));
        }
        let mut groups = Vec::new();
        let mut row_count = 0i64;
        for id in &pruned {
            let count: i64 = c.query_row(
                "SELECT count(*) FROM change_rows WHERE changeset_id=?1",
                [id],
                |r| r.get(0),
            )?;
            row_count += count;
            let namespace: String = c.query_row(
                "SELECT namespace FROM changesets WHERE id=?1",
                [id],
                |r| r.get(0),
            )?;
            groups.push(json!({"id":id,"namespace":namespace,"rows":count}));
        }
        let repairs: Vec<_> = predecessors
            .iter()
            .map(|id| json!({"memory_id":id,"from":memory_id,"to":successor}))
            .collect();
        if apply {
            for id in &pruned {
                c.execute(
                    "DELETE FROM change_rows WHERE changeset_id=?1",
                    [id],
                )?;
                c.execute("DELETE FROM changesets WHERE id=?1", [id])?;
            }
            c.execute(
                "UPDATE memories SET migrated_to=?1 WHERE migrated_to=?2",
                params![successor, memory_id],
            )?;
            c.execute("DELETE FROM memories WHERE id=?1", [memory_id])?;
            if node_gc {
                c.execute(
                    "DELETE FROM edges WHERE parent_uuid=?1 OR child_uuid=?1",
                    [&node],
                )?;
                c.execute(
                    "DELETE FROM glossary_keywords WHERE node_uuid=?1",
                    [&node],
                )?;
                c.execute(
                    "DELETE FROM memory_access_logs WHERE node_uuid=?1",
                    [&node],
                )?;
                c.execute(
                    "DELETE FROM node_attrs WHERE node_uuid=?1",
                    [&node],
                )?;
                c.execute("DELETE FROM nodes WHERE uuid=?1", [&node])?;
                crate::search::refresh(c, std::slice::from_ref(&node))?;
                // Glossary words shape tokenization of every document in their namespace.
                for namespace in &keyword_namespaces {
                    crate::search::rebuild(c, Some(namespace))?;
                }
            }
        }
        if apply {
            crate::content::collect(c)?;
        }
        Ok(
            json!({"memory_id":memory_id,"node_uuid":node,"bytes":bytes,"chain_repairs":repairs,
            "pruned_changesets":groups,"node_gc":node_gc,"counts":{"node_attrs":attrs_count,"nodes":i64::from(node_gc),"edges":edge_count,"glossary_keywords":keyword_count,"memory_access_logs":log_count,"memories":1,"chain_repairs":predecessors.len(),"changesets":pruned.len(),"change_rows":row_count},"apply":apply}),
        )
    })
}

pub fn clear_logs(
    c: &Connection, ns: &str, keep_days: u64, all_namespaces: bool, apply: bool,
) -> Result<Value> {
    // SQLite's supported calendar is finite; never let an overflowing modifier become NULL.
    if keep_days > 3_000_000 {
        return Err(Error::Invalid(
            "keep_days exceeds supported calendar range".into(),
        ));
    }
    transaction(c, apply, || {
        let modifier = format!("-{keep_days} days");
        let cutoff: Option<String> =
            c.query_row("SELECT datetime('now',?1)", [&modifier], |r| {
                r.get(0)
            })?;
        let cutoff = cutoff
            .ok_or_else(|| Error::Invalid("invalid retention cutoff".into()))?;
        let predicate = "(?1 OR namespace=?2) AND (?3=0 OR datetime(accessed_at)<datetime(?4))";
        let (count,bytes):(i64,i64)=c.query_row(&format!("SELECT count(*),coalesce(sum(length(CAST(node_uuid AS BLOB))+length(CAST(namespace AS BLOB))+length(CAST(accessed_at AS BLOB))+coalesce(length(CAST(context AS BLOB)),0)+8),0) FROM memory_access_logs WHERE {predicate}"),params![all_namespaces,ns,keep_days as i64,cutoff],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if apply {
            c.execute(
                &format!("DELETE FROM memory_access_logs WHERE {predicate}"),
                params![all_namespaces, ns, keep_days as i64, cutoff],
            )?;
        }
        Ok(
            json!({"count":count,"bytes":bytes,"bytes_approximate":true,"cutoff":cutoff,"keep_days":keep_days,"apply":apply}),
        )
    })
}
