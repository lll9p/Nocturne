use nocturne::{Store, graph};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn command(db: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nocturne"));
    cmd.env_remove("NOCTURNE_DB")
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(db);
    if !args.contains(&"--namespace") {
        cmd.args(["--namespace", ""]);
    }
    cmd.args(args);
    cmd
}

fn output(db: &Path, args: &[&str]) -> Output {
    command(db, args).output().unwrap()
}

fn text(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n")
}

fn value(db: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&text(output(db, args))).unwrap()
}

// Human formatting may hide stats definitions, but must preserve JSON and
// user attributes that resemble built-in records or metadata.
#[test]
fn human_output_preserves_nested_user_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let store = Store::create(&db).unwrap();
    store
        .write("", None, |c| {
            graph::create(c, "", "core://", "note", "body", 7, Some("visible"))
        })
        .unwrap();
    // User attributes may resemble result records; neither objects nor arrays may lose fields.
    let lookalike = json!({"uri":"custom://item", "version":1, "content":"user data",
        "semantics":{"statistics_only":false},
        "children":{"context":"custom://item", "children":[], "custom":"must remain visible"}});
    let mut attrs = lookalike.clone();
    attrs["items"] = json!([lookalike]);
    store
        .write("", None, |c| {
            nocturne::attrs::set(c, "", "core://note", &attrs)
        })
        .unwrap();
    assert_eq!(
        value(&db, &["--json", "read", "core://note", "--no-track"])["attrs"],
        attrs
    );
    let human = text(output(&db, &["read", "core://note", "--no-track"]));
    for marker in [
        "custom: must remain visible",
        "context: custom://item",
        "content: user data",
        "statistics_only: false",
    ] {
        assert_eq!(human.matches(marker).count(), 2, "{human}");
    }
    let stats = value(&db, &["--json", "stats"]);
    assert_eq!(stats["semantics"]["statistics_only"], true);
    assert_eq!(stats["semantics"]["purge_candidates"], false);
    assert_eq!(stats["semantics"]["reclaimable_bytes_estimated"], false);
    assert!(stats["semantics"]["text_bytes"].is_string());
    let human = text(output(&db, &["stats"]));
    assert!(!human.contains("semantics:"), "{human}");
    assert!(human.contains("memory_nodes: 1"), "{human}");
    assert!(human.contains("content_bytes: 4"), "{human}");
    // An empty namespace must remain visible without merging adjacent review records.
    let reviews = text(output(&db, &["review", "list"]));
    let records: Vec<_> = reviews.trim_end().split("\n\n").collect();
    assert_eq!(records.len(), 2, "{reviews}");
    for record in records {
        assert!(record.starts_with("id: "), "{record}");
        assert!(record.ends_with("namespace: \"\""), "{record}");
    }
    assert!(
        value(&db, &["--json", "review", "list"])
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["namespace"] == "")
    );
}

// Page metadata must be opt-in for JSON, and human output must distinguish an
// exhausted offset from no matches instead of silently inviting an early stop.
#[test]
fn search_page_metadata_preserves_json_and_explains_empty_pages() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let store = Store::create(&db).unwrap();
    for name in ["a", "b"] {
        store
            .write("", None, |c| {
                graph::create(c, "", "core://", name, "needle", 0, None)
            })
            .unwrap();
    }
    let legacy = value(&db, &["--json", "search", "needle", "--limit", "1"]);
    assert!(legacy.is_array());
    let first = value(
        &db,
        &["--json", "search", "needle", "--with-total", "--limit", "1"],
    );
    assert_eq!(
        first,
        json!({"total":2,"offset":0,"limit":1,"has_more":true,"results":legacy})
    );
    let count = value(
        &db,
        &["--json", "search", "needle", "--with-total", "--limit", "0"],
    );
    assert_eq!(
        count,
        json!({"total":2,"offset":0,"limit":0,"has_more":true,"results":[]})
    );
    let first = text(output(&db, &["search", "needle", "--limit", "1"]));
    assert!(first.contains("--offset 1"), "{first}");
    assert!(!first.contains("Total matching nodes:"), "{first}");
    let last = text(output(
        &db,
        &["search", "needle", "--limit", "1", "--offset", "1"],
    ));
    assert!(last.contains("No more results."), "{last}");
    let exhausted = text(output(&db, &["search", "needle", "--offset", "9"]));
    assert!(exhausted.contains("before this offset"), "{exhausted}");
    let missing = text(output(&db, &["search", "absent", "--offset", "9"]));
    assert_eq!(missing, "No matching nodes.\n");
    let count = text(output(
        &db,
        &["search", "needle", "--with-total", "--limit", "0"],
    ));
    assert_eq!(
        count,
        "Total matching nodes: 2; no results requested (--limit 0).\n"
    );
    let uncounted = text(output(&db, &["search", "needle", "--limit", "0"]));
    assert_eq!(
        uncounted,
        "Matching nodes exist; no results requested (--limit 0).\n"
    );
}
