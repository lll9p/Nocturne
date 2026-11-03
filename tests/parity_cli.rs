use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(db: &Path, ns: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nocturne"));
    c.env_remove("NOCTURNE_DB")
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(db)
        .args(["--namespace", ns, "--json"])
        .args(args);
    c
}
fn output(db: &Path, ns: &str, args: &[&str], body: Option<&str>) -> Output {
    let mut child = command(db, ns, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(body) = body {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
fn ok(db: &Path, ns: &str, args: &[&str], body: Option<&str>) -> Value {
    let out = output(db, ns, args, body);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stderr.is_empty());
    serde_json::from_slice(&out.stdout).unwrap()
}
fn denied(db: &Path, ns: &str, args: &[&str], code: &str) {
    let out = output(db, ns, args, None);
    assert!(!out.status.success(), "{args:?}");
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(error["error"]["code"], code, "{args:?}");
}
// Standalone diff must reject two stdin consumers and render plain output without opening a database.
#[test]
fn standalone_diff_never_creates_database_and_supports_stdin_and_plain_output()
{
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("must-not-exist.db");
    let diff = ok(
        &db,
        "",
        &["diff", "--old-file", "-", "--new", "after\n"],
        Some("before\n"),
    );
    assert!(diff["unified"].as_str().unwrap().contains("-before"));
    assert!(diff["unified"].as_str().unwrap().contains("+after"));
    denied(
        &db,
        "",
        &["diff", "--old-file", "-", "--new-file", "-"],
        "invalid",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_nocturne"))
        .env_remove("NOCTURNE_NAMESPACE")
        .arg("--db")
        .arg(&db)
        .args(["diff", "--old", "before\n", "--new", "after\n"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("---"));
    assert!(text.contains("+++"));
    assert!(text.contains("@@"));
    assert!(text.contains("-before"));
    assert!(text.contains("+after"));
    assert!(!db.exists());
}
