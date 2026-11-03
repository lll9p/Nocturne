use nocturne::{Store, graph};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(db: &Path, ns: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nocturne"))
        .env_remove("NOCTURNE_DB")
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(db)
        .args(["--namespace", ns, "--json"])
        .args(args)
        .output()
        .unwrap()
}
fn ok(db: &Path, ns: &str, args: &[&str]) -> Value {
    let out = run(db, ns, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    serde_json::from_slice(&out.stdout).unwrap()
}
fn race(db: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_nocturne"))
        .env_remove("NOCTURNE_DB")
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(db)
        .args(["--namespace", "one", "--json"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
fn one_winner(a: std::process::Child, b: std::process::Child) {
    let outputs =
        [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
    assert_eq!(outputs.iter().filter(|out| out.status.success()).count(), 1);
    let loser = outputs.iter().find(|out| !out.status.success()).unwrap();
    assert_eq!(
        loser.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&loser.stderr)
    );
}
fn changes(store: &Store) -> i64 {
    store
        .connection()
        .query_row("SELECT count(*) FROM changesets", [], |r| r.get(0))
        .unwrap()
}

// Prevent competing processes from duplicating a moved route or restoring the same orphan twice.
#[test]
fn concurrent_moves_and_orphan_restores_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let store = Store::create(&db).unwrap();
    let created = store
        .write("one", None, |c| {
            graph::create(
                c,
                "one",
                "core://",
                "original",
                "Concurrent memory",
                1,
                None,
            )
        })
        .unwrap();
    let before = changes(&store);
    one_winner(
        race(&db, &["move", "core://original", "notes://first"]),
        race(&db, &["move", "core://original", "notes://second"]),
    );
    assert_eq!(changes(&store), before + 1);
    let uri: String = store
        .connection()
        .query_row("SELECT domain||'://' ||path FROM paths", [], |r| r.get(0))
        .unwrap();
    ok(&db, "one", &["unlink", &uri]);
    let id = created["version"].as_i64().unwrap().to_string();
    let before = changes(&store);
    one_winner(
        race(&db, &["orphans", "restore", &id, "core://recovered1"]),
        race(&db, &["orphans", "restore", &id, "core://recovered2"]),
    );
    assert_eq!(changes(&store), before + 1);
    assert_eq!(ok(&db, "one", &["index"]).as_array().unwrap().len(), 1);
    assert!(
        ok(&db, "one", &["orphans", "list"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(ok(&db, "one", &["doctor"])["ok"], true);
}
