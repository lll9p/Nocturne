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

// User objects resembling memory records must retain nested fields in human output.
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
    ] {
        assert_eq!(human.matches(marker).count(), 2, "{human}");
    }
}
