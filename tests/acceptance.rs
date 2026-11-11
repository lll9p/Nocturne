//! Process-level acceptance: every database is private to a temporary directory.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use tempfile::TempDir;

struct Sandbox {
    _dir: TempDir,
    db: PathBuf,
    body: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("memory.db");
        let body = dir.path().join("body.txt");
        std::fs::write(&body, "Original acceptance memory").unwrap();
        let s = Self {
            _dir: dir,
            db,
            body,
        };
        s.ok(&["init"]);
        s
    }
    fn command(&self) -> Command {
        command(&self.db)
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        success(self.run(args))
    }
    fn create(&self, title: &str) -> Value {
        self.ok(&[
            "create",
            "core://",
            "--title",
            title,
            "--file",
            self.body.to_str().unwrap(),
        ])
    }
    fn spawn(&self, args: &[&str]) -> Child {
        self.command()
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
}
fn command(db: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nocturne"));
    c.env_remove("NOCTURNE_DB")
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(db)
        .args(["--namespace", "", "--json"]);
    c
}
fn success(out: Output) -> Value {
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("JSON stdout")
}
fn failure(out: Output) -> Value {
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "errors must not use stdout");
    let v: Value =
        serde_json::from_slice(&out.stderr).expect("JSON error on stderr");
    assert!(v["error"]["code"].as_str().is_some());
    v
}

// Prevent concurrent stale writers from both committing and losing an acknowledged content update.
#[test]
fn competing_expected_versions_have_one_winner() {
    let s = Sandbox::new();
    let original = s.create("agent");
    let version = original["version"].as_i64().unwrap().to_string();
    std::fs::write(&s.body, "First competing content").unwrap();
    let other = s._dir.path().join("other.txt");
    std::fs::write(&other, "Competing content").unwrap();
    let a = s.spawn(&[
        "update",
        "core://agent",
        "--file",
        s.body.to_str().unwrap(),
        "--expect-version",
        &version,
    ]);
    let b = s.spawn(&[
        "update",
        "core://agent",
        "--file",
        other.to_str().unwrap(),
        "--expect-version",
        &version,
    ]);
    let outputs =
        [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
    assert_eq!(outputs.iter().filter(|o| o.status.success()).count(), 1);
    for out in outputs {
        if out.status.success() {
            success(out);
        } else {
            assert_eq!(failure(out)["error"]["code"], "conflict");
        }
    }
    assert_eq!(
        s.ok(&["history", "core://agent"]).as_array().unwrap().len(),
        2
    );
    assert_eq!(s.ok(&["review", "list"]).as_array().unwrap().len(), 2);
}

// Prevent a rejected batch undo from reverting unaffected members before discovering a conflict.
#[test]
fn conflicting_batch_rollback_does_not_partially_revert_other_members() {
    let s = Sandbox::new();
    s.create("first");
    s.create("second");
    let updated = s._dir.path().join("updated.txt");
    std::fs::write(&updated, "Batch update").unwrap();
    for uri in ["core://first", "core://second"] {
        s.ok(&[
            "--batch",
            "atomic-review",
            "update",
            uri,
            "--file",
            updated.to_str().unwrap(),
        ]);
    }
    s.ok(&["update", "core://first", "--file", s.body.to_str().unwrap()]);
    assert_eq!(
        failure(s.run(&["review", "rollback", "atomic-review", "--by-batch"]))
            ["error"]["code"],
        "conflict"
    );
    assert_eq!(
        s.ok(&["read", "core://second", "--no-track"])["content"],
        "Batch update"
    );
    assert_eq!(
        s.ok(&["read", "core://first", "--no-track"])["content"],
        "Original acceptance memory"
    );
}

const CRASH_HELPER_DIR: &str = "NOCTURNE_ACCEPTANCE_CRASH_HELPER_DIR";

// Also reap on timeout/assertion failure, before TempDir attempts Windows cleanup.
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Support the hard-kill test inside an open real write transaction; not an independent test.
#[test]
#[ignore = "spawned only by killed_writer_leaves_no_partial_transaction"]
fn transaction_crash_helper() {
    let dir = std::env::var_os(CRASH_HELPER_DIR)
        .expect("crash fixture must be launched by its parent test");
    let dir = PathBuf::from(dir);
    let store = nocturne::store::Store::open(&dir.join("memory.db")).unwrap();
    store
        .write("", Some("interrupted-batch"), |c| {
            assert!(!c.is_autocommit());
            let created = nocturne::graph::create(
                c,
                "",
                "core://",
                "interrupted",
                "crashonlytoken",
                0,
                None,
            )?;
            // Exercise real uncommitted FTS writes too, before Store::write can commit.
            nocturne::search::refresh(c, &[created["node_uuid"].as_str().unwrap().to_owned()])?;
            let pending: i64 = c.query_row(
                "SELECT count(*) FROM changesets WHERE batch='interrupted-batch'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(pending, 1);
            let indexed: i64 = c.query_row(
                "SELECT count(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'crashonlytoken'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(indexed, 1);
            std::fs::write(dir.join("transaction-ready"), b"ready")?;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
        .unwrap();
}

fn transaction_table_counts(db: &Path) -> Vec<(&'static str, i64)> {
    let store = nocturne::store::Store::open(db).unwrap();
    [
        "nodes",
        "paths",
        "edges",
        "memories",
        "contents",
        "changesets",
        "change_rows",
        "audit_context",
        "search_documents",
        "search_documents_fts",
    ]
    .into_iter()
    .map(|table| {
        let count = store
            .connection()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get(0)
            })
            .unwrap();
        (table, count)
    })
    .collect()
}

// Prevent hard process termination from persisting partial graph, audit, or FTS writes.
#[test]
fn killed_writer_leaves_no_partial_transaction() {
    let s = Sandbox::new();
    s.create("survivor");
    let baseline = transaction_table_counts(&s.db);
    {
        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "transaction_crash_helper",
                    "--nocapture",
                ])
                .env(CRASH_HELPER_DIR, s._dir.path())
                .env_remove("NOCTURNE_DB")
                .env_remove("NOCTURNE_NAMESPACE")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "helper exited before termination"
            );
            if s._dir.path().join("transaction-ready").exists() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "helper never reached the open transaction"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        assert!(
            !status.success(),
            "helper must be forcibly terminated, not commit normally"
        );
    }
    assert_eq!(transaction_table_counts(&s.db), baseline);
    assert_eq!(
        failure(s.run(&["read", "core://interrupted", "--no-track"]))["error"]
            ["code"],
        "not_found"
    );
    assert!(
        s.ok(&["search", "crashonlytoken"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        s.ok(&["read", "core://survivor", "--no-track"])["content"],
        "Original acceptance memory"
    );
    assert_eq!(s.ok(&["review", "list"]).as_array().unwrap().len(), 1);
    s.ok(&["doctor"]);
    // A new writer must also be able to acquire the lock after the killed process.
    s.create("after-recovery");
}
