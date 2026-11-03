use nocturne::Store;
use serde_json::Value;
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
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
    if !args.contains(&"--json") {
        cmd.arg("--json");
    }
    cmd.args(args);
    cmd
}
fn output(db: &Path, args: &[&str]) -> Output {
    command(db, args).output().unwrap()
}
fn input(db: &Path, args: &[&str], bytes: &[u8]) -> Output {
    let mut child = command(db, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
fn text(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    String::from_utf8(out.stdout).unwrap()
}
fn value(db: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&text(output(db, args))).unwrap()
}
fn put(db: &Path, args: &[&str], body: &str) -> Value {
    serde_json::from_str(&text(input(db, args, body.as_bytes()))).unwrap()
}
fn code(out: Output, expected: i32) {
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(
        error["error"]["code"],
        if expected == 2 { "invalid" } else { "storage" }
    );
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn create(db: &Path, name: &str, body: &str) -> Value {
    put(
        db,
        &[
            "--json", "create", "core://", "--title", name, "--file", "-",
        ],
        body,
    )
}
fn count(store: &Store, table: &str) -> i64 {
    store
        .connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

// File and stdin decoding failures must be classified differently from filesystem failures without writes.
#[test]
fn invalid_utf8_files_and_stdin_are_invalid_but_missing_files_are_io() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let out = output(&db, &["read", "core://missing"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(error["error"]["code"], "not_found");
    assert!(!db.exists());
    let store = Store::create(&db).unwrap();
    create(&db, "note", "unchanged");
    let bad = dir.path().join("bad.txt");
    std::fs::write(&bad, [0xff, 0xfe]).unwrap();
    let missing = dir.path().join("missing.txt");
    let before = count(&store, "changesets");
    for prefix in [
        vec!["create", "core://", "--title", "invalid"],
        vec!["attrs", "core://note"],
    ] {
        for (file, expected) in [
            (bad.to_str().unwrap(), 2),
            ("-", 2),
            (missing.to_str().unwrap(), 5),
        ] {
            let mut args = prefix.clone();
            args.extend(["--file", file]);
            code(
                if file == "-" {
                    input(&db, &args, &[0xff, 0xfe])
                } else {
                    output(&db, &args)
                },
                expected,
            );
        }
    }
    // Diff is dispatched before opening a database; exercise that distinct input route once.
    code(
        output(
            &db,
            &["diff", "--old-file", bad.to_str().unwrap(), "--new", "ok"],
        ),
        2,
    );
    code(
        input(&db, &["diff", "--old-file", "-", "--new", "ok"], &[0xff]),
        2,
    );
    code(
        output(
            &db,
            &[
                "diff",
                "--old-file",
                missing.to_str().unwrap(),
                "--new",
                "ok",
            ],
        ),
        5,
    );
    assert_eq!(count(&store, "changesets"), before);
    assert_eq!(
        value(&db, &["--json", "read", "core://note", "--no-track"])["content"],
        "unchanged"
    );
}

// A corrupt search index must make doctor fail even though its structured report is returned on stdout.
#[test]
fn doctor_corrupt_index_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let store = Store::create(&db).unwrap();
    create(&db, "note", "indexed body");
    store.connection().execute("INSERT INTO search_documents_fts(search_documents_fts) VALUES('delete-all')", []).unwrap();
    let out = output(&db, &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["check"] == "fts_integrity")
    );
}
