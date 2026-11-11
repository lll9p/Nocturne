use crate::{
    error::{Error, Result},
    graph,
    store::{ROOT, TransactionGuard},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

// (domain, parent path, node UUID) -> (memory ID, alias paths).
type AliasGroups =
    BTreeMap<(String, String, String), (Option<i64>, Vec<String>)>;
// Node UUID, best priority, URI, last access timestamp, staleness days.
type RandomCandidateRow = (String, i64, String, Option<String>, f64);

fn threshold(priority: Option<i64>, fallback: i64) -> i64 {
    match priority {
        None | Some(999) => fallback,
        Some(p) => {
            let p = p.clamp(0, 20);
            if p == 0 {
                3
            } else {
                (7 * (1i64 << (p - 1))).min(36500)
            }
        }
    }
}

pub fn diagnostic(
    c: &Connection, ns: &str, domain: Option<&str>, days_stale: i64,
    max_children: usize, longest_limit: usize, min_bytes: usize,
) -> Result<Value> {
    if days_stale < 0 {
        return Err(Error::Invalid("days_stale must be nonnegative".into()));
    }
    let max_children = i64::try_from(max_children).map_err(|_| {
        Error::Invalid("max_children exceeds SQLite integer range".into())
    })?;
    let min_bytes = u64::try_from(min_bytes).map_err(|_| {
        Error::Invalid("min_bytes exceeds byte-size range".into())
    })?;
    // longest_limit stays usize and is used directly by Vec::truncate.
    c.execute_batch("BEGIN")?;
    let _guard = TransactionGuard::new(c);
    let mut q = c.prepare("SELECT p.domain,p.path,n.uuid,n.created_at,n.last_accessed_at,e.priority,m.id,m.content,
        coalesce(n.last_accessed_at,n.created_at,CURRENT_TIMESTAMP),
        julianday('now')-julianday(coalesce(n.last_accessed_at,n.created_at,CURRENT_TIMESTAMP))
        FROM paths p JOIN nodes n ON n.uuid=p.node_uuid LEFT JOIN edges e ON e.id=p.edge_id
        LEFT JOIN memory_rows m ON m.node_uuid=n.uuid AND m.deprecated=0
        WHERE p.namespace=?1 AND (?2 IS NULL OR p.domain=?2) ORDER BY p.domain,p.path")?;
    let rows = q
        .query_map(params![ns, domain], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, f64>(9)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let paths: BTreeSet<_> =
        rows.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
    let mut stale: BTreeMap<String, Value> = BTreeMap::new();
    let mut longest: BTreeMap<String, Value> = BTreeMap::new();
    let mut broken_parent_routes = Vec::new();
    let mut aliases = AliasGroups::new();
    for (d, p, id, created, last, priority, mid, content, effective, age) in
        &rows
    {
        let uri = format!("{d}://{p}");
        let title = p.rsplit('/').next().unwrap_or("");
        let limit = threshold(*priority, days_stale);
        let better = |v: &Value| {
            priority.unwrap_or(999) < v["priority"].as_i64().unwrap_or(999)
        };
        if mid.is_some()
            && *age > limit as f64
            && stale.get(id).is_none_or(better)
        {
            stale.insert(id.clone(),json!({"uuid":id,"uri":uri,"created_at":created,"last_accessed_at":last,"effective_date":effective,"stale_days":(age*10.0).round()/10.0,"threshold_days":limit,"priority":priority,"title":title,"memory_id":mid}));
        }
        if let Some(text) = content
            && longest.get(id).is_none_or(better)
        {
            longest.insert(id.clone(),json!({"uri":uri,"node_uuid":id,"memory_id":mid,"priority":priority,"byte_size":text.len(),"char_count":text.chars().count()}));
        }
        let parent = p.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
        aliases
            .entry((d.clone(), parent.into(), id.clone()))
            .or_default()
            .0 = *mid;
        aliases
            .get_mut(&(d.clone(), parent.into(), id.clone()))
            .unwrap()
            .1
            .push(p.clone());
        if p.contains('/') && !paths.contains(&(d.clone(), parent.to_owned())) {
            let text = content.as_deref().unwrap_or("");
            let snippet =
                text.chars().take(50).collect::<String>().replace('\n', " ")
                    + if text.chars().count() > 50 { "..." } else { "" };
            broken_parent_routes.push(json!({"uuid":id,"uri":uri,"created_at":created,"last_accessed_at":last,"memory_id":mid,"snippet":snippet}));
        }
    }
    let mut duplicates:Vec<_>=aliases.into_iter().filter(|(_,(_,p))|p.len()>1).map(|((d,parent,id),(mid,p))|json!({"uuid":id,"memory_id":mid,"domain":d,"parent_path":parent,"count":p.len(),"paths":p})).collect();
    duplicates.sort_by_key(|v| std::cmp::Reverse(v["count"].as_u64().unwrap()));
    let mut stale: Vec<_> = stale.into_values().collect();
    stale.sort_by_key(|v| {
        v["last_accessed_at"]
            .as_str()
            .or(v["created_at"].as_str())
            .unwrap_or("")
            .to_owned()
    });
    broken_parent_routes
        .sort_by_key(|v| v["created_at"].as_str().unwrap_or("").to_owned());
    let mut longest: Vec<_> = longest
        .into_values()
        .filter(|v| v["byte_size"].as_u64().unwrap() >= min_bytes)
        .collect();
    longest
        .sort_by_key(|v| std::cmp::Reverse(v["byte_size"].as_u64().unwrap()));
    longest.truncate(longest_limit);
    let mut q=c.prepare("SELECT e.parent_uuid,count(DISTINCT e.child_uuid) FROM edges e JOIN paths p ON p.edge_id=e.id WHERE p.namespace=?1 AND (?2 IS NULL OR p.domain=?2) GROUP BY e.parent_uuid HAVING count(DISTINCT e.child_uuid)>?3 ORDER BY 2 DESC,1")?;
    let parents = q
        .query_map(params![ns, domain, max_children], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut crowded = Vec::new();
    for (id, count) in parents {
        if id == ROOT {
            crowded.push(json!({"uuid":id,"uri":format!("{}://",domain.unwrap_or("core")),"title":"(root)","child_count":count}));
        } else if let Some(r) = rows.iter().find(|r| r.2 == id) {
            crowded.push(json!({"uuid":id,"uri":format!("{}://{}",r.0,r.1),"title":r.1.rsplit('/').next(),"child_count":count}));
        }
    }
    c.execute_batch("COMMIT")?;
    Ok(
        json!({"stale_nodes":stale,"crowded_nodes":crowded,"broken_parent_routes":broken_parent_routes,"duplicate_aliases":duplicates,"longest_memories":longest}),
    )
}

pub fn selection_weight(staleness_days: f64, priority: i64) -> f64 {
    staleness_days.max(0.5) * 0.5f64.powf(priority.max(0) as f64).max(1e-12)
}

/// Select a half-open weighted interval using a unit sample in [0, 1).
pub fn select_weighted(weights: &[f64], unit: f64) -> Option<usize> {
    if !(0.0..1.0).contains(&unit)
        || weights.iter().any(|w| !w.is_finite() || *w <= 0.0)
    {
        return None;
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 || !total.is_finite() {
        return None;
    }
    let mut target = unit * total;
    for (i, w) in weights.iter().enumerate() {
        if target < *w {
            return Some(i);
        }
        target -= w;
    }
    weights.len().checked_sub(1)
}
/// Select an alias independently of the node's weight.
pub fn select_uniform(count: usize, unit: f64) -> Option<usize> {
    if count == 0 || !(0.0..1.0).contains(&unit) {
        return None;
    }
    Some(((unit * count as f64) as usize).min(count - 1))
}

fn unit_random() -> f64 {
    // The low 64 UUID bits contain fixed variant bits only at the top; discard those.
    let bits = uuid::Uuid::new_v4().as_u128() as u64 & ((1u64 << 53) - 1);
    bits as f64 / (1u64 << 53) as f64
}

pub fn random(
    c: &Connection, ns: &str, domain: Option<&str>, track: bool,
) -> Result<Value> {
    c.execute_batch(if track { "BEGIN IMMEDIATE" } else { "BEGIN" })?;
    let _guard = TransactionGuard::new(c);
    let mut q=c.prepare("SELECT n.uuid,coalesce(e.priority,2),p.domain||'://'||p.path,n.last_accessed_at,
        julianday('now')-julianday(coalesce(n.last_accessed_at,n.created_at,CURRENT_TIMESTAMP))
        FROM paths p JOIN edges e ON e.id=p.edge_id JOIN nodes n ON n.uuid=p.node_uuid
        JOIN memory_rows m ON m.node_uuid=n.uuid AND m.deprecated=0
        WHERE p.namespace=?1 AND n.uuid<>?3 AND ((?2 IS NOT NULL AND p.domain=?2) OR (?2 IS NULL AND p.domain<>'system')) ORDER BY n.uuid,coalesce(e.priority,2),p.domain,p.path")?;
    let rows = q
        .query_map(params![ns, domain, ROOT], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    for row in &rows {
        if candidates
            .last()
            .is_none_or(|r: &&RandomCandidateRow| r.0 != row.0)
        {
            candidates.push(row);
        }
    }
    let weights: Vec<_> = candidates
        .iter()
        .map(|r| selection_weight(r.4, r.1))
        .collect();
    let index = select_weighted(&weights, unit_random())
        .ok_or_else(|| Error::NotFound("no random memory candidates".into()))?;
    let row = candidates[index];
    let aliases: Vec<_> =
        rows.iter().filter(|r| r.0 == row.0).map(|r| &r.2).collect();
    let uri = aliases[select_uniform(aliases.len(), unit_random()).unwrap()];
    let mut value = graph::read(c, ns, uri, false)?;
    if track {
        c.execute(
            "UPDATE nodes SET last_accessed_at=CURRENT_TIMESTAMP WHERE uuid=?1",
            [&row.0],
        )?;
        c.execute(
            "INSERT INTO memory_access_logs(node_uuid,namespace,context) VALUES(?1,?2,?3)",
            params![row.0, ns, uri],
        )?;
    }
    value["selection"] = json!({"node_uuid":row.0,"uri":uri,"priority":row.1,"last_accessed_at":row.3,"staleness_days":row.4.max(0.5),"weight":weights[index],"candidate_count":candidates.len()});
    c.execute_batch("COMMIT")?;
    Ok(value)
}
