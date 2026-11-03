use crate::error::{Error, Result};
use aho_corasick::AhoCorasick;
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use unicode_normalization::UnicodeNormalization;

fn normalize_keyword(keyword: &str) -> Result<String> {
    if keyword.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "Glossary keyword cannot contain control characters".into(),
        ));
    }
    let keyword: String = keyword.trim().nfc().collect();
    if keyword.is_empty() {
        return Err(Error::Invalid("Glossary keyword cannot be empty".into()));
    }
    Ok(keyword)
}

pub fn edit(
    c: &Connection, ns: &str, uri: &str, add: &[String], remove: &[String],
) -> Result<Value> {
    let node = crate::graph::resolve(c, ns, uri)?;
    validate_node(c, ns, &node)?;
    let additions = add
        .iter()
        .map(|s| normalize_keyword(s))
        .collect::<Result<BTreeSet<_>>>()?;
    let removals = remove
        .iter()
        .map(|s| normalize_keyword(s))
        .collect::<Result<BTreeSet<_>>>()?;
    if !additions.is_disjoint(&removals) {
        return Err(Error::Invalid(
            "keyword cannot be both added and removed".into(),
        ));
    }
    let mut added = 0;
    let mut removed = 0;
    let mut skipped =
        add.len() + remove.len() - additions.len() - removals.len();
    for keyword in removals {
        let n = c.execute(
            "DELETE FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 AND keyword=?3",
            params![ns, node, keyword],
        )?;
        removed += n;
        skipped += usize::from(n == 0);
    }
    for keyword in additions {
        let exists: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 AND keyword=?3)",
            params![ns, node, keyword], |r| r.get(0),
        )?;
        if exists {
            skipped += 1;
        } else {
            c.execute(
                "INSERT INTO glossary_keywords(namespace,node_uuid,keyword) VALUES(?1,?2,?3)",
                params![ns, node, keyword],
            )?;
            added += 1;
        }
    }
    Ok(
        json!({"node_uuid":node,"keywords":list(c, ns, Some(&node))?,"added":added,"removed":removed,"skipped":skipped}),
    )
}

fn validate(
    conn: &Connection, namespace: &str, node: &str, keyword: &str,
) -> Result<String> {
    let keyword = normalize_keyword(keyword)?;
    validate_node(conn, namespace, node)?;
    Ok(keyword)
}

fn validate_node(conn: &Connection, namespace: &str, node: &str) -> Result<()> {
    let reachable: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND node_uuid=?2)",
        params![namespace, node],
        |r| r.get(0),
    )?;
    if !reachable {
        return Err(Error::NotFound(format!(
            "Node '{node}' is not reachable in this namespace"
        )));
    }
    Ok(())
}

pub fn add(
    conn: &Connection, namespace: &str, node_uuid: &str, keyword: &str,
) -> Result<Value> {
    let keyword = validate(conn, namespace, node_uuid, keyword)?;
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 AND keyword=?3)",params![namespace,node_uuid,keyword],|r| r.get(0))?;
    if exists {
        return Err(Error::Conflict(format!(
            "Keyword '{keyword}' is already bound to this node"
        )));
    }
    conn.execute(
        "INSERT INTO glossary_keywords(namespace,node_uuid,keyword) VALUES(?1,?2,?3)",
        params![namespace, node_uuid, keyword],
    )?;
    let id = conn.last_insert_rowid();
    Ok(json!({"id":id,"keyword":keyword,"node_uuid":node_uuid}))
}

pub fn remove(
    conn: &Connection, namespace: &str, node_uuid: &str, keyword: &str,
) -> Result<Value> {
    let keyword = validate(conn, namespace, node_uuid, keyword)?;
    let count = conn.execute(
        "DELETE FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 AND keyword=?3",
        params![namespace, node_uuid, keyword],
    )?;
    Ok(json!({"success":count>0}))
}

pub fn list(
    conn: &Connection, namespace: &str, node_uuid: Option<&str>,
) -> Result<Value> {
    if let Some(node) = node_uuid {
        let mut stmt = conn.prepare("SELECT keyword FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 ORDER BY keyword")?;
        let words = stmt
            .query_map(params![namespace, node], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        return Ok(json!(words));
    }
    let mut stmt = conn.prepare(
        "SELECT g.keyword,g.node_uuid,p.domain,p.path,m.content FROM glossary_keywords g
        LEFT JOIN paths p ON p.node_uuid=g.node_uuid AND p.namespace=g.namespace
        LEFT JOIN memories m ON m.node_uuid=g.node_uuid AND m.deprecated=0 AND p.node_uuid IS NOT NULL
        WHERE g.namespace=?1 ORDER BY g.keyword,p.domain,p.path,g.node_uuid",
    )?;
    let mut rows = stmt.query([namespace])?;
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let keyword: String = row.get(0)?;
        let node: String = row.get(1)?;
        let domain: Option<String> = row.get(2)?;
        let path: Option<String> = row.get(3)?;
        let content = row.get::<_, Option<String>>(4)?.unwrap_or_default();
        let snippet: String = content
            .chars()
            .take(100)
            .collect::<String>()
            .replace('\n', " ");
        let snippet = format!(
            "{}{}",
            snippet,
            if content.chars().count() > 100 {
                "..."
            } else {
                ""
            }
        );
        let uri = match (domain, path) {
            (Some(d), Some(p)) => format!("{d}://{p}"),
            _ => format!("unlinked://{node}"),
        };
        groups.entry(keyword).or_default().push(
            json!({"node_uuid":node,"namespace":namespace,"uri":uri,"content_snippet":snippet}),
        );
    }
    Ok(json!(
        groups
            .into_iter()
            .map(|(keyword, nodes)| json!({"keyword":keyword,"nodes":nodes}))
            .collect::<Vec<_>>()
    ))
}

pub fn scan(
    conn: &Connection, namespace: &str, content: &str,
) -> Result<Value> {
    let mut stmt = conn.prepare(
        "SELECT g.keyword,g.node_uuid,p.domain,p.path FROM glossary_keywords g
        JOIN paths p ON p.node_uuid=g.node_uuid AND p.namespace=g.namespace
        WHERE g.namespace=?1 ORDER BY g.keyword,p.domain,p.path,g.node_uuid",
    )?;
    let mut rows = stmt.query([namespace])?;
    let mut bindings: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows.next()? {
        let keyword: String = row.get(0)?;
        let node: String = row.get(1)?;
        if keyword.is_empty() || !seen.insert((keyword.clone(), node.clone())) {
            continue;
        }
        let domain: String = row.get(2)?;
        let path: String = row.get(3)?;
        bindings
            .entry(keyword)
            .or_default()
            .push(json!({"node_uuid":node,"uri":format!("{domain}://{path}")}));
    }
    if bindings.is_empty() {
        return Ok(json!({}));
    }
    let keywords: Vec<_> = bindings.keys().cloned().collect();
    let automaton = AhoCorasick::new(&keywords)
        .map_err(|e| Error::Storage(e.to_string()))?;
    let found: HashSet<_> = automaton
        .find_overlapping_iter(content)
        .map(|m| m.pattern().as_usize())
        .collect();
    let mut result = Map::new();
    for index in found {
        result
            .insert(keywords[index].clone(), json!(bindings[&keywords[index]]));
    }
    Ok(Value::Object(result))
}
