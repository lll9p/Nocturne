use crate::error::{Error, Result};
use jieba_rs::Jieba;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff |
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0xac00..=0xd7af)
}

pub(crate) fn is_cjk_dictionary_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(cjk)
}

fn is_non_cjk_word_char(c: char) -> bool {
    !cjk(c)
        && (c.is_alphanumeric()
            || c == '_'
            || unicode_normalization::char::is_combining_mark(c))
}

fn dictionary(conn: &Connection, namespace: &str) -> Result<Jieba> {
    let mut jieba = Jieba::new();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT keyword FROM glossary_keywords WHERE namespace=?1 ORDER BY keyword",
    )?;
    for word in stmt.query_map([namespace], |r| r.get::<_, String>(0))? {
        let word = word?;
        if is_cjk_dictionary_word(&word) {
            jieba.add_word(&word, None, None);
        }
    }
    Ok(jieba)
}

fn tokens(jieba: &Jieba, text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let chinese = cjk(chars[i]);
        if !chinese && !is_non_cjk_word_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len()
            && if chinese {
                cjk(chars[i])
            } else {
                is_non_cjk_word_char(chars[i])
            }
        {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let words: Vec<String> = if chinese {
            jieba
                .cut_for_search(&run, true)
                .iter()
                .map(|s| s.word.nfc().collect::<String>())
                .collect()
        } else {
            vec![run.nfc().collect()]
        };
        for word in words {
            if !word.is_empty() && seen.insert(word.clone()) {
                result.push(word);
            }
        }
    }
    result
}

fn insert(
    conn: &Connection, namespace: Option<&str>, node: Option<&str>,
    dictionaries: &mut HashMap<String, Jieba>,
) -> Result<()> {
    let mut stmt = conn.prepare("SELECT p.namespace,p.domain,p.path,p.node_uuid,m.id,m.content,e.disclosure,e.priority
        FROM paths p JOIN edges e ON e.id=p.edge_id JOIN memory_rows m ON m.node_uuid=p.node_uuid AND m.deprecated=0
        WHERE (?1 IS NULL OR p.namespace=?1) AND (?2 IS NULL OR p.node_uuid=?2)
        ORDER BY p.namespace,p.domain,p.path")?;
    let rows = stmt.query_map(params![namespace, node], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, i64>(7)?,
        ))
    })?;
    for row in rows {
        let (ns, domain, path, node, id, content, disclosure, priority) = row?;
        if !dictionaries.contains_key(&ns) {
            dictionaries.insert(ns.clone(), dictionary(conn, &ns)?);
        }
        let jieba = &dictionaries[&ns];
        let uri = format!("{domain}://{path}");
        let mut kw_stmt = conn.prepare("SELECT keyword FROM glossary_keywords WHERE namespace=?1 AND node_uuid=?2 ORDER BY keyword")?;
        let keywords = kw_stmt
            .query_map(params![ns, node], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let glossary = keywords.join(" ");
        let mut terms = keywords;
        for text in [
            &path,
            &uri,
            &content,
            disclosure.as_deref().unwrap_or(""),
            &glossary,
        ] {
            terms.extend(tokens(jieba, text));
        }
        let mut seen = HashSet::new();
        terms.retain(|t| seen.insert(t.clone()));
        let terms = terms.join(" ");
        conn.execute("INSERT INTO search_documents(namespace,domain,path,node_uuid,memory_id,uri,disclosure,search_terms,priority) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![ns,domain,path,node,id,uri,disclosure,terms,priority])?;
    }
    Ok(())
}

pub fn rebuild(conn: &Connection, namespace: Option<&str>) -> Result<()> {
    conn.execute(
        "DELETE FROM search_documents WHERE ?1 IS NULL OR namespace=?1",
        [namespace],
    )?;
    insert(conn, namespace, None, &mut HashMap::new())
}

pub fn refresh(conn: &Connection, node_uuids: &[String]) -> Result<()> {
    let mut dictionaries = HashMap::new();
    for node in node_uuids.iter().collect::<HashSet<_>>() {
        conn.execute(
            "DELETE FROM search_documents WHERE node_uuid=?1",
            [node],
        )?;
        insert(conn, None, Some(node), &mut dictionaries)?;
    }
    Ok(())
}

// Stream lowercase expansions; retain only query-sized state, never a copy of the document.
fn snippet(content: &str, query: &str, terms: &[String]) -> String {
    let hit = std::iter::once(query)
        .chain(terms.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .find_map(|needle| {
            let pattern: Vec<char> =
                needle.chars().flat_map(char::to_lowercase).collect();
            let mut prefix = vec![0; pattern.len()];
            for i in 1..pattern.len() {
                let mut j = prefix[i - 1];
                while j > 0 && pattern[i] != pattern[j] {
                    j = prefix[j - 1];
                }
                if pattern[i] == pattern[j] {
                    j += 1;
                }
                prefix[i] = j;
            }
            let mut positions = vec![0; pattern.len()];
            let mut matched = 0;
            let mut count = 0;
            for (original, ch) in content.chars().enumerate() {
                for lower in ch.to_lowercase() {
                    positions[count % pattern.len()] = original;
                    count += 1;
                    while matched > 0 && lower != pattern[matched] {
                        matched = prefix[matched - 1];
                    }
                    if lower == pattern[matched] {
                        matched += 1;
                    }
                    if matched == pattern.len() {
                        return Some((
                            positions[(count - pattern.len()) % pattern.len()],
                            original.saturating_add(1),
                        ));
                    }
                }
            }
            None
        });
    let (start, end) = hit
        .map(|(start, end)| (start.saturating_sub(30), end.saturating_add(30)))
        .unwrap_or((0, 80));
    let mut chars = content.chars().skip(start);
    let text: String = chars.by_ref().take(end - start).collect();
    format!(
        "{}{}{}",
        if start > 0 { "..." } else { "" },
        text,
        if chars.next().is_some() { "..." } else { "" }
    )
}

pub fn search(
    conn: &Connection, namespace: &str, query: &str, domain: Option<&str>,
    limit: usize,
) -> Result<Value> {
    search_with_or(conn, namespace, query, domain, limit, false)
}

pub fn search_with_or(
    conn: &Connection, namespace: &str, query: &str, domain: Option<&str>,
    limit: usize, or_mode: bool,
) -> Result<Value> {
    search_filtered(conn, namespace, Some(query), domain, limit, or_mode, &[])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    All,
    Any,
    Contains,
}

pub fn search_filtered(
    conn: &Connection, namespace: &str, query: Option<&str>,
    domain: Option<&str>, limit: usize, or_mode: bool,
    filters: &[(String, String)],
) -> Result<Value> {
    search_with_mode(
        conn,
        namespace,
        query,
        domain,
        limit,
        if or_mode {
            MatchMode::Any
        } else {
            MatchMode::All
        },
        filters,
    )
}

pub fn search_with_mode(
    conn: &Connection, namespace: &str, query: Option<&str>,
    domain: Option<&str>, limit: usize, mode: MatchMode,
    filters: &[(String, String)],
) -> Result<Value> {
    search_page(conn, namespace, query, domain, (limit, 0), mode, filters)
}

/// Page is (limit, offset), applied after node deduplication.
pub fn search_page(
    conn: &Connection, namespace: &str, query: Option<&str>,
    domain: Option<&str>, page: (usize, usize), mode: MatchMode,
    filters: &[(String, String)],
) -> Result<Value> {
    let (limit, offset) = page;
    // Legacy zero-limit calls need validation, but never need to walk an offset.
    if limit == 0 {
        i64::try_from(offset).map_err(|_| {
            Error::Invalid("Search offset exceeds SQLite integer range".into())
        })?;
    }
    let request = PageRequest {
        limit,
        offset: if limit == 0 { 0 } else { offset },
        with_total: false,
    };
    Ok(search_page_with_info(
        conn, namespace, query, domain, request, mode, filters,
    )?
    .get_mut("results")
    .unwrap()
    .take())
}

#[derive(Clone, Copy, Debug)]
pub struct PageRequest {
    pub limit: usize,
    pub offset: usize,
    pub with_total: bool,
}

/// Page metadata uses the same filtered, node-deduplicated candidate stream.
/// With totals disabled, scan at most one distinct node beyond the requested page.
pub fn search_page_with_info(
    conn: &Connection, namespace: &str, query: Option<&str>,
    domain: Option<&str>, page: PageRequest, mode: MatchMode,
    filters: &[(String, String)],
) -> Result<Value> {
    let PageRequest {
        limit,
        offset,
        with_total,
    } = page;
    i64::try_from(limit).map_err(|_| {
        Error::Invalid("Search limit exceeds SQLite integer range".into())
    })?;
    i64::try_from(offset).map_err(|_| {
        Error::Invalid("Search offset exceeds SQLite integer range".into())
    })?;
    if mode == MatchMode::Contains && query.is_none() {
        return Err(Error::Invalid("Contains search requires a query".into()));
    }
    let or_mode = mode == MatchMode::Any;
    if query.is_none() && (filters.is_empty() || or_mode) {
        return Err(Error::Invalid(
            "Search requires a query or attrs filters; --or requires a query"
                .into(),
        ));
    }
    let mut terms = Vec::new();
    let mut expression = String::new();
    if let Some(query) = query
        && query.trim().is_empty()
    {
        return Err(Error::Invalid("Search query cannot be empty".into()));
    }
    if let Some(query) = query.filter(|_| mode != MatchMode::Contains) {
        let words: Vec<_> = query.split_whitespace().collect();
        if query
            .chars()
            .any(|c| matches!(c, '(' | ')' | '"' | '*' | ':' | '^' | '{' | '}'))
            || (words.len() > 1
                && words
                    .iter()
                    .any(|word| matches!(*word, "AND" | "OR" | "NOT" | "NEAR")))
        {
            return Err(Error::Invalid(
            "Search does not support raw FTS expressions; terms use AND by default; use --or for OR matching".into(),
        ));
        }
        terms = tokens(&dictionary(conn, namespace)?, query);
        if terms.is_empty() {
            return Err(Error::Invalid(
                "Search query must contain searchable terms".into(),
            ));
        }
        expression = terms
            .iter()
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(if or_mode { " OR " } else { " AND " });
    }
    // json_each compares literal top-level keys, not JSON paths or array elements.
    let mut predicates = String::new();
    let mut parameters: Vec<rusqlite::types::Value> = vec![
        namespace.to_owned().into(),
        domain
            .map(|v| v.to_owned().into())
            .unwrap_or(rusqlite::types::Value::Null),
    ];
    for (key, value) in filters {
        predicates.push_str(" AND EXISTS (SELECT 1 FROM node_attrs a, json_each(a.data_json) j WHERE a.node_uuid=sd.node_uuid AND j.key=? AND j.type='text' AND j.value=?)");
        parameters.push(key.clone().into());
        parameters.push(value.clone().into());
    }
    if mode == MatchMode::Contains {
        parameters.push(query.unwrap().to_owned().into());
        let parameter = parameters.len();
        predicates.push_str(&format!(
            " AND (instr(lower(m.content),lower(?{parameter}))>0
            OR instr(lower(sd.path),lower(?{parameter}))>0
            OR instr(lower(sd.domain||'://' ||sd.path),lower(?{parameter}))>0
            OR instr(lower(e.disclosure),lower(?{parameter}))>0
            OR EXISTS (SELECT 1 FROM glossary_keywords g
                WHERE g.namespace=sd.namespace AND g.node_uuid=sd.node_uuid
                AND instr(lower(g.keyword),lower(?{parameter}))>0))"
        ));
    }
    let sql = if query.is_some() && mode != MatchMode::Contains {
        parameters.push(expression.into());
        format!("SELECT sd.domain,sd.path,sd.node_uuid,sd.uri,sd.priority,sd.memory_id,sd.disclosure,
        bm25(search_documents_fts,0.0,0.0,2.5,0.0,2.0,1.0,1.0,0.75) AS score
        FROM search_document_rows sd JOIN search_documents_fts ON sd.id=search_documents_fts.rowid
        WHERE sd.namespace=? AND (?2 IS NULL OR sd.domain=?2){predicates} AND search_documents_fts MATCH ?
        ORDER BY score,sd.priority,length(sd.path),sd.domain,sd.path")
    } else {
        format!("SELECT sd.domain,sd.path,sd.node_uuid,sd.domain||'://' ||sd.path,e.priority,m.id,e.disclosure
        FROM paths sd JOIN edges e ON e.id=sd.edge_id
        JOIN memory_rows m ON m.node_uuid=sd.node_uuid AND m.deprecated=0
        WHERE sd.namespace=? AND (?2 IS NULL OR sd.domain=?2){predicates}
        ORDER BY e.priority,length(sd.path),sd.domain,sd.path")
    };
    // A nested savepoint pins both phases to one snapshot, including empty/exhausted
    // candidate scans. RELEASE never rolls back a caller's business transaction.
    conn.execute_batch("SAVEPOINT search_page_snapshot")?;
    let result = (|| -> Result<Value> {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(parameters))?;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        let mut count = 0usize;
        let mut has_more = false;
        let mut exhausted = true;
        let mut body =
            conn.prepare("SELECT content FROM memory_rows WHERE id=?1")?;
        while let Some(row) = rows.next()? {
            if !seen.insert(row.get::<_, String>(2)?) {
                continue;
            }
            count += 1;
            if count <= offset {
                continue;
            }
            if result.len() == limit {
                has_more = true;
                if !with_total {
                    exhausted = false;
                    break;
                }
                continue;
            }
            let path: String = row.get(1)?;
            let content: String =
                body.query_row([row.get::<_, i64>(5)?], |r| r.get(0))?;
            result.push(json!({"domain":row.get::<_,String>(0)?,"name":path.rsplit('/').next().unwrap_or(""),"path":path,
            "node_uuid":row.get::<_,String>(2)?,"uri":row.get::<_,String>(3)?,"priority":row.get::<_,i64>(4)?,"disclosure":row.get::<_,Option<String>>(6)?,
            "snippet":snippet(&content,query.unwrap_or(""),&terms)}));
        }
        Ok(json!({
            "total": if with_total || exhausted { Some(count) } else { None },
            "offset": offset, "limit": limit, "has_more": has_more,
            "results": result,
        }))
    })();
    let released = conn.execute_batch("RELEASE search_page_snapshot");
    let value = result?;
    released?;
    Ok(value)
}
