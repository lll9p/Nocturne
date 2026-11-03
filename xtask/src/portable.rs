use crate::support::*;
use serde_json::{Value, json};
use std::fs;
pub fn run(h: &Harness) -> Result<Value> {
    let db = "memory.db";
    macro_rules! r { ($($a:expr),+ $(,)?) => { h.run(db, &[$($a),+], None)?.0 }; }
    macro_rules! w { ($body:expr; $($a:expr),+ $(,)?) => { h.run(db, &[$($a),+], Some($body))?.0 }; }
    r!("init");
    let original = "中文记忆架构 GraphService alpha_knowledge";
    let first =
        w!(original; "create", "core://", "--title", "agent", "--file", "-");
    assert_eq!(
        r!("read", "core://agent", "--no-track")["content"],
        original
    );
    let human = h
        .raw(
            db,
            "",
            &["read", "core://agent", "--no-track"],
            None,
            false,
            0,
        )?
        .0;
    let human = String::from_utf8(human.stdout)?;
    assert!(human.starts_with(&format!(
        "uri: core://agent\nversion: {}\n",
        first["version"]
    )));
    assert!(!human.contains("children:\n  children:"));
    let reviews = r!("review", "list");
    assert_eq!(reviews[0]["id"], first["change_id"]);
    assert!(!reviews[0]["created_at"].as_str().unwrap().is_empty());
    r!("alias", "add", "notes://agent", "core://agent");
    r!("trigger", "add", "core://agent", "记忆架构");
    assert_eq!(
        r!("search", "记忆架构 GraphService")[0]["node_uuid"],
        first["node_uuid"]
    );
    let changed = r!(
        "patch",
        "notes://agent",
        "--old",
        "GraphService",
        "--new",
        "RustEngine"
    );
    assert!(
        r!("read", "core://agent", "--no-track")["content"]
            .as_str()
            .unwrap()
            .contains("RustEngine")
    );
    let blocked = h
        .raw(
            db,
            "",
            &["review", "rollback", first["change_id"].as_str().unwrap()],
            None,
            false,
            4,
        )?
        .0;
    assert!(blocked.stdout.is_empty());
    let error: Value = serde_json::from_slice(&blocked.stderr)?;
    let error = &error["error"];
    assert_eq!(error["reason"], "rollback_conflict");
    assert!(
        error["blocking_uris"]
            .as_array()
            .unwrap()
            .contains(&json!("core://agent"))
    );
    assert!(
        error["related_changesets"]
            .as_array()
            .unwrap()
            .contains(&json!({"id":changed["change_id"],"batch":null}))
    );
    assert_eq!(error["foreign_references"], false);
    r!("review", "rollback", changed["change_id"].as_str().unwrap());
    assert_eq!(
        r!("read", "notes://agent", "--no-track")["content"],
        original
    );
    r!("boot", "set", "core://agent");
    assert_eq!(r!("boot")["loaded"].as_array().unwrap().len(), 1);
    let renamed = r!("rename", "core://agent", "assistant");
    assert_eq!(r!("boot", "list"), json!(["core://assistant"]));
    let moved = r!("move", "core://assistant", "archive://assistant");
    for uri in ["archive://assistant", "notes://agent"] {
        assert_eq!(r!("read", uri, "--no-track")["content"], original);
    }
    for change in [&moved, &renamed] {
        r!("review", "rollback", change["change_id"].as_str().unwrap());
    }
    assert_eq!(r!("boot", "list"), json!(["core://agent"]));
    let deleted = w!("Archived memory"; "create","core://","--title","deleted","--file","-");
    let unlinked = r!("unlink", "core://deleted");
    let id = deleted["version"].to_string();
    assert_eq!(r!("orphans", "list")[0]["memory_id"], deleted["version"]);
    assert_eq!(r!("orphans", "show", &id)["content"], "Archived memory");
    let restored = r!("orphans", "restore", &id, "notes://recovered");
    assert_eq!(restored["node_uuid"], deleted["node_uuid"]);
    for change in [&restored, &unlinked, &deleted] {
        r!("review", "rollback", change["change_id"].as_str().unwrap());
    }
    let capacity = r!("stats");
    assert_eq!(capacity["scope"], "database");
    assert_eq!(capacity["counts"]["memory_nodes"], 1);
    assert_eq!(
        capacity["storage"]["allocated_bytes"].as_u64().unwrap(),
        capacity["storage"]["used_bytes"].as_u64().unwrap()
            + capacity["storage"]["free_bytes"].as_u64().unwrap()
    );
    {
        let mut c = rusqlite::Connection::open(h.path(db))?;
        let tx = c.transaction()?;
        for i in 0..99 {
            let node = format!("portable-fixture-{i}");
            tx.execute("INSERT INTO nodes(uuid) VALUES(?)", [&node])?;
            tx.execute(
                "INSERT INTO memories(node_uuid,content) VALUES(?,?)",
                [node.clone(), format!("中文基准记忆 {i} isolated_entry_{i}")],
            )?;
            tx.execute(
                "INSERT INTO edges(parent_uuid,child_uuid,name) VALUES(?,?,?)",
                [
                    "00000000-0000-0000-0000-000000000000".to_string(),
                    node.clone(),
                    format!("entry_{i}"),
                ],
            )?;
            tx.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES('','core',?,?,?)",rusqlite::params![format!("entry_{i}"),tx.last_insert_rowid(),node])?;
        }
        tx.commit()?;
    }
    r!("reindex");
    assert_eq!(r!("doctor")["ok"], true);
    let backup = h.path("backup.db");
    r!("backup", "--output", text(&backup));
    assert_eq!(h.run("backup.db", &["doctor"], None)?.0["ok"], true);
    let mut timings = json!({});
    for (name, args) in [
        ("read", vec!["read", "core://agent", "--no-track"]),
        ("chinese_search", vec!["search", "记忆架构 GraphService"]),
        ("write", vec!["update", "core://agent", "--file", "-"]),
    ] {
        let mut samples = Vec::new();
        for i in 0..7 {
            let body = format!("{original} write_{i}");
            samples.push(
                h.run(
                    db,
                    &args,
                    if name == "write" {
                        Some(body.as_str())
                    } else {
                        None
                    },
                )?
                .1,
            );
        }
        timings[format!("{name}_median_ms")] = json!(median(&mut samples));
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = std::time::Instant::now();
        assert!(h.command().arg("--version").output()?.status.success());
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    timings["process_start_median_ms"] = json!(median(&mut samples));
    w!("portable alpha"; "update","core://agent","--file","-","--priority","3","--disclosure","portable");
    r!(
        "patch",
        "core://agent",
        "--old",
        "alpha",
        "--new",
        "beta",
        "--priority",
        "4"
    );
    w!(" gamma"; "append","core://agent","--stdin","--disclosure","compound");
    assert_eq!(
        r!("read", "core://agent", "--no-track")["content"],
        "portable beta gamma"
    );
    r!(
        "trigger",
        "add",
        "core://agent",
        "portable_one",
        "portable_two"
    );
    r!(
        "trigger",
        "edit",
        "core://agent",
        "--add",
        "portable_three",
        "--remove",
        "portable_one"
    );
    r!("preset", "create", "portable", "--activate");
    r!("boot", "set", "core://agent");
    r!("preset", "duplicate", "portable", "portable_copy");
    r!("preset", "activate", "portable_copy");
    assert_eq!(r!("boot", "list"), json!(["core://agent"]));
    h.ns(
        db,
        "other",
        &["create", "core://", "--title", "other", "--file", "-"],
        Some("private body"),
        0,
    )?;
    assert!(!r!("namespaces").to_string().contains("private body"));
    assert!(
        !r!("paths", "--all-namespaces")
            .to_string()
            .contains("private body")
    );
    r!("diagnostic");
    r!("random", "--no-track");
    r!("random");
    let diff = h
        .run(
            "diff-must-not-create.db",
            &["diff", "--old-file", "-", "--new", "after\n"],
            Some("before\n"),
        )?
        .0;
    assert!(diff["unified"].as_str().unwrap().contains("-before"));
    assert!(diff["unified"].as_str().unwrap().contains("+after"));
    assert!(!h.path("diff-must-not-create.db").exists());
    let obsolete =
        w!("old"; "create","core://","--title","retention","--file","-");
    w!("new"; "update","core://retention","--file","-");
    assert!(!r!("deprecated").as_array().unwrap().is_empty());
    r!("review", "accept", "--all", "--all-namespaces");
    let id = obsolete["version"].to_string();
    r!("purge", &id);
    r!("purge", &id, "--apply");
    assert_eq!(
        r!("read", "core://retention", "--no-track")["content"],
        "new"
    );
    r!("logs", "clear", "--keep-days", "0");
    r!("logs", "clear", "--keep-days", "0", "--apply");
    assert_eq!(r!("doctor")["ok"], true);
    let attrs = h.path("attrs.json");
    fs::write(&attrs, r#"{"source":"paper","tags":["math"]}"#)?;
    let unicode = w!("unicode_probe"; "create","知识://","--title","向量","--file","-","--attrs-file",text(&attrs));
    r!("alias", "add", "笔记://别名", "知识://向量");
    let reduced = r!("read", "笔记://别名", "--no-track", "--no-children");
    assert!(reduced.get("children").is_none());
    assert_eq!(reduced["attrs"]["source"], "paper");
    assert_eq!(r!("attrs", "知识://向量")["attrs"], reduced["attrs"]);
    assert!(
        r!("search", "unicode_probe absent_probe")
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        r!("search", "unicode_probe absent_probe", "--or")[0]["node_uuid"],
        unicode["node_uuid"]
    );
    let edited = r!(
        "attrs",
        "知识://向量",
        "--set",
        "source=GB/T 21372",
        "--unset",
        "tags"
    );
    for args in [
        vec!["search", "--attr", "source=GB/T 21372"],
        vec!["search", "unicode_probe", "--attr", "source=GB/T 21372"],
    ] {
        assert_eq!(
            h.run(db, &args, None)?.0[0]["node_uuid"],
            unicode["node_uuid"]
        );
    }
    r!("review", "rollback", edited["change_id"].as_str().unwrap());
    assert_eq!(r!("attrs", "知识://向量")["attrs"], reduced["attrs"]);
    let messy = h.path("messy-notes");
    fs::create_dir(&messy)?;
    for (name, body) in [
        ("01 概述.md", "导入概述"),
        ("02.标准.txt", "导入标准"),
        ("03, (文献) °C.md", "标点文献"),
    ] {
        fs::write(messy.join(name), body)?;
    }
    let invalid = h
        .ns(
            db,
            "",
            &["import-dir", text(&messy), "--into", "知识://", "--dry-run"],
            None,
            2,
        )?
        .0;
    assert_eq!(invalid["valid"], false);
    assert_eq!(invalid["problem_source_count"], 3);
    let sanitized = r!(
        "import-dir",
        text(&messy),
        "--into",
        "知识://",
        "--dry-run",
        "--sanitize"
    );
    assert_eq!(sanitized["valid"], true);
    assert_eq!(sanitized["mappings"].as_array().unwrap().len(), 3);
    let misleading = h.path("custom-metadata.json");
    fs::write(&misleading, r#"{"priority":1,"disclosure":"custom"}"#)?;
    let imported = r!(
        "--batch",
        "sanitized",
        "import-dir",
        text(&messy),
        "--into",
        "知识://",
        "--sanitize",
        "--priority",
        "2",
        "--disclosure",
        "imported",
        "--attrs-file",
        text(&misleading)
    );
    assert_eq!(imported["warnings"].as_array().unwrap().len(), 2);
    let node = r!("read", "知识://01-概述", "--no-track");
    assert_eq!(node["priority"], 2);
    assert_eq!(node["disclosure"], "imported");
    assert_eq!(node["attrs"], json!({"priority":1,"disclosure":"custom"}));
    r!("review", "rollback", "sanitized", "--batch");
    assert_eq!(r!("doctor")["ok"], true);
    let short = w!("阿利特晶型受氧化镁影响"; "create","测试://","--title","中文","--file","-");
    for query in ["阿利特", "晶型"] {
        assert_eq!(
            r!("search", query, "--contains")[0]["node_uuid"],
            short["node_uuid"]
        );
    }
    let raw = b"a\x00b\r\ne\xcc\x81 without final newline";
    let rawfile = h.path("original-utf8.txt");
    fs::write(&rawfile, raw)?;
    r!(
        "create",
        "测试://",
        "--title",
        "保真",
        "--file",
        text(&rawfile)
    );
    assert_eq!(
        r!("read", "测试://保真", "--no-track")["content"]
            .as_str()
            .unwrap()
            .as_bytes(),
        raw
    );
    let noop = r!(
        "--batch",
        "empty-write",
        "update",
        "测试://保真",
        "--file",
        text(&rawfile)
    );
    assert_eq!(noop["changed"], false);
    assert!(noop.get("change_id").is_none());
    let modified =
        w!("阿利特晶型的新解释"; "update","测试://中文","--file","-");
    let args = ["review", "diff", modified["change_id"].as_str().unwrap()];
    let human =
        String::from_utf8(h.raw(db, "", &args, None, false, 0)?.0.stdout)?;
    assert!(
        human.contains("-阿利特晶型受氧化镁影响")
            && human.contains("+阿利特晶型的新解释")
            && !human.contains("table: memories")
    );
    assert!(!h.run(db, &args, None)?.0.is_null());
    let export = h.path("readable-export");
    let before = r!("review", "list");
    assert_eq!(
        r!("export-dir", "--output", text(&export))["scope"],
        "namespace"
    );
    assert_eq!(fs::read(export.join("测试/保真.md"))?, raw);
    assert!(!export.join("core/other.md").exists());
    assert_eq!(r!("review", "list"), before);
    assert_eq!(r!("doctor")["ok"], true);
    batch(h, &attrs, &mut timings)?;
    let mut report = h.report()?;
    report["portable_smoke"] = json!("passed");
    report["timing_dataset"] =
        json!("100 active nodes, 101 URI routes; median of 7 fresh processes");
    report["timings_ms"] = timings;
    Ok(report)
}
fn batch(
    h: &Harness, attrs: &std::path::Path, timings: &mut Value,
) -> Result<()> {
    let notes = h.path("batch-notes");
    fs::create_dir(&notes)?;
    for i in 0..59 {
        fs::write(
            notes.join(format!("条目_{i:02}.md")),
            format!("中文批量知识 {i} import_entry_{i}"),
        )?;
    }
    for db in ["single59.db", "bulk59.db"] {
        h.run(db, &["init"], None)?;
    }
    let mut elapsed = 0.;
    for i in 0..59 {
        elapsed += h
            .run(
                "single59.db",
                &[
                    "create",
                    "知识://",
                    "--title",
                    &format!("条目_{i:02}"),
                    "--file",
                    text(&notes.join(format!("条目_{i:02}.md"))),
                    "--attrs-file",
                    text(attrs),
                ],
                None,
            )?
            .1;
    }
    timings["single_create_59_total_ms"] = json!(elapsed);
    macro_rules! r { ($($a:expr),+) => { h.run("bulk59.db",&[$($a),+],None)?.0 }; }
    let preview = r!(
        "import-dir",
        text(&notes),
        "--into",
        "知识://",
        "--dry-run",
        "--attrs-file",
        text(attrs)
    );
    assert_eq!(preview["mappings"].as_array().unwrap().len(), 59);
    assert_eq!(r!("paths"), json!([]));
    let (imported, ms) = h.run(
        "bulk59.db",
        &[
            "--batch",
            "batch59",
            "import-dir",
            text(&notes),
            "--into",
            "知识://",
            "--attrs-file",
            text(attrs),
        ],
        None,
    )?;
    timings["import_dir_59_total_ms"] = json!(ms);
    assert_eq!(imported["imported"], 59);
    assert_eq!(imported["files"], 59);
    let hits = r!("search", "中文", "--contains", "--limit", "100");
    assert_eq!(
        r!(
            "search",
            "中文",
            "--contains",
            "--limit",
            "10",
            "--offset",
            "10"
        ),
        json!(hits.as_array().unwrap()[10..20])
    );
    let routes = r!("index", "--domain", "知识");
    assert_eq!(
        r!(
            "index", "--domain", "知识", "--limit", "10", "--offset", "10"
        ),
        json!(routes.as_array().unwrap()[10..20])
    );
    let tree = r!("tree", "知识://", "--depth", "1", "--limit", "3");
    assert_eq!(tree["node_count"], 3);
    assert_eq!(tree["truncated"], true);
    assert_eq!(r!("review", "list").as_array().unwrap().len(), 1);
    assert_eq!(
        h.run("single59.db", &["review", "list"], None)?
            .0
            .as_array()
            .unwrap()
            .len(),
        59
    );
    assert_eq!(
        r!("read", "知识://条目_58", "--no-track")["attrs"]["source"],
        "paper"
    );
    assert_eq!(r!("doctor")["ok"], true);
    r!("review", "rollback", "batch59", "--batch");
    assert_eq!(r!("paths"), json!([]));
    assert_eq!(r!("stats")["node_attrs"]["rows"], 0);
    assert_eq!(r!("doctor")["ok"], true);
    let reviews = r!("review", "list");
    assert_eq!(r!("vacuum")["ok"], true);
    assert_eq!(r!("review", "list"), reviews);
    Ok(())
}
