use nocturne::{error::Error, export_dir::export, graph, store::Store};
use std::{fs, path::Path};

fn create(s: &Store, ns: &str, parent: &str, name: &str, body: &str) {
    s.write(ns, None, |c| {
        graph::create(c, ns, parent, name, body, 0, None)
    })
    .unwrap();
}

fn names(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

// Export must neither commit nor roll back the caller's pending writes when rejecting a nested snapshot.
#[test]
fn caller_owned_transaction_is_untouched() {
    let s = Store::memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    s.connection()
        .execute_batch("BEGIN; INSERT INTO nodes(uuid) VALUES('pending')")
        .unwrap();
    assert!(export(s.connection(), "", &tmp.path().join("new")).is_err());
    assert!(!s.connection().is_autocommit());
    assert_eq!(
        s.connection()
            .query_row(
                "SELECT count(*) FROM nodes WHERE uuid='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    s.connection().execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        s.connection()
            .query_row(
                "SELECT count(*) FROM nodes WHERE uuid='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

// Case-distinct legal URIs must not silently overwrite one another in a portable export.
#[test]
fn case_collisions_are_rejected_before_writing() {
    let s = Store::memory().unwrap();
    create(&s, "", "core://", "Name", "first body");
    create(&s, "", "core://", "name", "second body");
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(
        export(s.connection(), "", &tmp.path().join("out")),
        Err(Error::Conflict(_))
    ));
    assert!(names(tmp.path()).is_empty());
}

// Segment-aware selection must not include underscore lookalikes, sibling prefixes or other namespaces.
#[test]
fn subtree_is_a_namespace_local_path_view() {
    use nocturne::export_dir::export_with_options;
    let s = Store::memory().unwrap();
    create(&s, "a", "lit://", "doc_s", "parent");
    create(&s, "a", "lit://doc_s", "child", "child");
    create(&s, "a", "lit://", "docXs", "wrong");
    create(&s, "a", "lit://", "doc_s_extra", "wrong");
    create(&s, "b", "lit://", "doc_s", "other namespace");
    s.write("a", None, |c| {
        graph::alias(c, "a", "lit://doc_s", "other://alias")
    })
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    export_with_options(s.connection(), "a", &out, Some("lit://doc_s"), true)
        .unwrap();
    assert_eq!(fs::read(out.join("lit/doc_s.md")).unwrap(), b"parent");
    assert_eq!(names(&out.join("lit")), vec!["doc_s", "doc_s.md"]);
    assert!(!out.join("other").exists());
}
