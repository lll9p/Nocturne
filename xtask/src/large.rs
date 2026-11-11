use crate::support::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
fn body(i: usize, size: usize) -> Vec<u8> {
    let segment = format!(
        "Paper {i}: 阿利特晶型受氧化镁影响 Rietveld quenching alite.\r\n{}\n",
        (0..128)
            .map(|j| format!("term_{i}_{j}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut bytes = segment.repeat(size / segment.len() + 1).into_bytes();
    bytes.truncate(size);
    while std::str::from_utf8(&bytes).is_err() {
        bytes.pop();
    }
    bytes
}
fn physical(path: &Path) -> Result<Value> {
    let c = Connection::open(path)?;
    let scalar = |sql: &str| -> rusqlite::Result<i64> {
        c.query_row(sql, [], |r| r.get(0))
    };
    let mut tables = BTreeMap::new();
    let mut stmt =
        c.prepare("SELECT name,sum(pgsize) FROM dbstat GROUP BY name")?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
    {
        let (name, size) = row?;
        tables.insert(name, size);
    }
    let mut categories = BTreeMap::<&str, i64>::new();
    for (name, size) in &tables {
        let category = if name.starts_with("search_documents_fts") {
            "fts"
        } else if name.contains("search_documents") {
            "search"
        } else if name == "contents"
            || name == "memories"
            || name == "active_memory"
            || name == "memory_content"
        {
            "body"
        } else if name == "change_rows"
            || name.contains("changesets")
            || name.starts_with("audit_")
        {
            "audit"
        } else {
            "other"
        };
        *categories.entry(category).or_default() += size;
    }
    Ok(json!({
        "schema":scalar("PRAGMA user_version")?,
        "allocated_bytes":scalar("PRAGMA page_size")?*scalar("PRAGMA page_count")?,
        "free_bytes":scalar("PRAGMA page_size")?*scalar("PRAGMA freelist_count")?,
        "tables":tables,
        "physical_categories_bytes":categories,
        "logical_body_bytes": {
            "stored":scalar("SELECT coalesce(sum(length(CAST(content AS BLOB))),0) FROM contents")?,
            "memory_versions":scalar("SELECT coalesce(sum(length(CAST(content AS BLOB))),0) FROM memory_rows")?,
            "audit_references":scalar("SELECT coalesce(sum(length(CAST(b.content AS BLOB))),0)+coalesce(sum(length(CAST(a.content AS BLOB))),0) FROM change_rows r LEFT JOIN contents b ON b.id=r.before_content_id LEFT JOIN contents a ON a.id=r.after_content_id")?
        }
    }))
}
fn business_hash(path: &Path) -> Result<String> {
    let c = Connection::open(path)?;
    use sha2::{Digest, Sha256};
    let mut bytes = Sha256::new();
    for table in [
        "nodes",
        "memories",
        "contents",
        "edges",
        "paths",
        "node_attrs",
        "glossary_keywords",
        "presets",
        "changesets",
        "change_rows",
        "memory_access_logs",
    ] {
        bytes.update(table.as_bytes());
        let order = if table == "paths" { "1,2,3" } else { "1" };
        // Include expanded bodies, not only immutable references, in audit/business checks.
        let sql = match table {
            "memories" => "SELECT * FROM memory_rows ORDER BY 1".to_string(),
            "change_rows" => "SELECT r.*,b.content,a.content FROM change_rows r LEFT JOIN contents b ON b.id=r.before_content_id LEFT JOIN contents a ON a.id=r.after_content_id ORDER BY r.id".to_string(),
            _ => format!("SELECT * FROM {table} ORDER BY {order}"),
        };
        let mut stmt = c.prepare(&sql)?;
        let columns = stmt.column_count();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            for i in 0..columns {
                use rusqlite::types::ValueRef;
                match row.get_ref(i)? {
                    ValueRef::Null => bytes.update([0]),
                    ValueRef::Integer(n) => {
                        bytes.update([1]);
                        bytes.update(n.to_le_bytes());
                    }
                    ValueRef::Real(n) => {
                        bytes.update([2]);
                        bytes.update(n.to_le_bytes());
                    }
                    ValueRef::Text(s) | ValueRef::Blob(s) => {
                        bytes.update([
                            if matches!(row.get_ref(i)?, ValueRef::Text(_)) {
                                3
                            } else {
                                4
                            },
                        ]);
                        bytes.update((s.len() as u64).to_le_bytes());
                        bytes.update(s);
                    }
                }
            }
            bytes.update([255]);
        }
    }
    Ok(hex(&bytes.finalize()))
}
pub fn run(h: &Harness) -> Result<Value> {
    let source = h.path("documents");
    let curated = h.path("curated");
    fs::create_dir(&source)?;
    fs::create_dir(&curated)?;
    let empty = h.path("empty.txt");
    fs::write(&empty, [])?;
    for i in 0..59 {
        fs::write(
            curated.join(format!("note_{i:02}.md")),
            format!(
                "Curated knowledge {i}: 阿利特晶型 Rietveld quenching alite\n"
            ),
        )?;
    }
    let big = 3_600_000;
    let normal = (110 * 1024 * 1024 - big) / 1577;
    let mut expected = BTreeMap::new();
    let mut total = 0;
    for i in 0..1578 {
        let bucket = format!("bucket_{:02}", i % 35);
        fs::create_dir_all(source.join(&bucket))?;
        let name = format!("paper_{i:04}.md");
        let bytes = body(i, if i == 0 { big } else { normal });
        total += bytes.len();
        fs::write(source.join(&bucket).join(&name), &bytes)?;
        expected.insert(format!("lit/docs/{bucket}/{name}"), hash(&bytes));
    }
    let db = "library.db";
    let mut timings = json!({});
    let mut peaks = json!({});
    macro_rules! r { ($label:expr; $($a:expr),+) => {{ let (v,ms)=h.run(db,&[$($a),+],None)?; timings[$label]=json!(ms); if let Some(peak) = h.last_peak.get() { peaks[$label]=json!(peak); } v }}; }
    r!("init_ms";"init");
    r!("curated_import_ms";"import-dir",text(&curated),"--into","clinker://");
    r!("root_create_ms";"create","lit://","--title","docs","--file",text(&empty));
    let imported = r!("corpus_import_ms";"--batch","corpus","import-dir",text(&source),"--into","lit://docs","--priority","1");
    assert_eq!(imported["files"], 1578);
    assert_eq!(imported["directories"], 35);
    let before = physical(&h.path(db))?;
    let fingerprint = business_hash(&h.path(db))?;
    let mut results = BTreeMap::new();
    for query in ["Rietveld", "quenching alite", "阿利特晶型"] {
        let hits =
            r!(&format!("search_{query}_ms");"search",query,"--limit","30");
        assert!(!hits.as_array().unwrap().is_empty());
        results.insert(query, hits);
    }
    let hits = r!("search_1000_ms";"search","Rietveld","--domain","lit","--limit","1000");
    assert_eq!(hits.as_array().unwrap().len(), 1000);
    assert_eq!(
        r!("search_page2_ms";"search","Rietveld","--domain","lit","--limit","30","--offset","30"),
        json!(hits.as_array().unwrap()[30..60])
    );
    assert!(
        !r!("contains_ms";"search","阿利特","--contains","--domain","lit")
            .as_array()
            .unwrap()
            .is_empty()
    );
    let index = r!("index_all_ms";"index","--domain","lit","--limit","2000");
    assert_eq!(
        r!("index_page_ms";"index","--domain","lit","--limit","100","--offset","100"),
        json!(index.as_array().unwrap()[100..200])
    );
    let tree =
        r!("tree_bounded_ms";"tree","lit://docs","--depth","2","--limit","100");
    assert_eq!(tree["truncated"], true);
    assert_eq!(tree["node_count"], 100);
    assert_eq!(tree["truncation_reasons"], json!(["limit"]));
    let export = h.path("export");
    r!("export_ms";"export-dir","--output",text(&export));
    for (relative, digest) in &expected {
        assert_eq!(
            &hash(&fs::read(export.join(relative))?),
            digest,
            "{relative}"
        );
    }
    let backup = h.path("backup.db");
    r!("backup_ms";"backup","--output",text(&backup));
    assert_eq!(h.run("backup.db", &["doctor"], None)?.0["ok"], true);
    assert_eq!(business_hash(&backup)?, fingerprint);
    for (query, expected) in &results {
        assert_eq!(
            &h.run("backup.db", &["search", query, "--limit", "30"], None)?
                .0,
            expected
        );
    }
    assert_eq!(
        h.run(
            "backup.db",
            &["search", "Rietveld", "--domain", "lit", "--limit", "1000"],
            None
        )?
        .0,
        hits
    );
    // Existing outputs must never be replaced, on every platform.
    let protected = h.path("protected.db");
    let sentinel = b"do not replace user output";
    fs::write(&protected, sentinel)?;
    let failure = h.raw(
        db,
        "",
        &["backup", "--output", text(&protected)],
        None,
        true,
        4,
    )?;
    assert!(!failure.0.status.success());
    assert_eq!(fs::read(&protected)?, sentinel);
    let export_before =
        fs::read(export.join("lit/docs/bucket_00/paper_0000.md"))?;
    let failed = h.raw(
        db,
        "",
        &["export-dir", "--output", text(&export)],
        None,
        true,
        4,
    )?;
    assert!(!failed.0.status.success());
    assert_eq!(
        fs::read(export.join("lit/docs/bucket_00/paper_0000.md"))?,
        export_before
    );
    // A late invalid import must not leave partially inserted nodes or audit rows.
    let bad = h.path("invalid-import");
    fs::create_dir(&bad)?;
    fs::write(bad.join("a.md"), "valid first file")?;
    fs::write(bad.join("z.md"), [0xff])?;
    let out = h.raw(
        db,
        "",
        &["import-dir", text(&bad), "--into", "lit://docs"],
        None,
        true,
        2,
    )?;
    assert!(!out.0.status.success());
    assert_eq!(business_hash(&h.path(db))?, fingerprint);
    assert_eq!(r!("vacuum_ms";"vacuum")["ok"], true);
    assert_eq!(r!("doctor_ms";"doctor")["ok"], true);
    assert_eq!(business_hash(&h.path(db))?, fingerprint);
    for (query, expected) in results {
        assert_eq!(
            r!(&format!("post_vacuum_search_{query}_ms");"search",query,"--limit","30"),
            expected
        );
    }
    let mut report = h.report()?;
    report["fixture"] = json!({"documents":1578,"buckets":35,"curated_nodes":59,"source_bytes":total,"largest_document_bytes":big});
    report["before_vacuum"] = before;
    report["after_vacuum"] = physical(&h.path(db))?;
    report["timings_ms"] = timings;
    report["peak_working_set_bytes_windows"] =
        if cfg!(windows) { peaks } else { Value::Null };
    Ok(report)
}
