use nocturne::{error::Error, graph, import_dir, store::Store};
use std::{fs, path::PathBuf};

struct Source(PathBuf);
impl Source {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .join(format!("nocturne-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn count(s: &Store, table: &str) -> i64 {
    s.connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

// A stale parent or a failure after the first insertion must roll back the entire import.
#[test]
fn parent_replacement_and_midway_failure_are_atomic() {
    let source = Source::new();
    fs::write(source.0.join("a.txt"), "a").unwrap();
    fs::write(source.0.join("b.txt"), "b").unwrap();
    let s = Store::memory().unwrap();
    s.write("", None, |c| {
        graph::create(c, "", "core://", "parent", "", 0, None)
    })
    .unwrap();
    let plan = import_dir::plan(s.connection(), "", &source.0, "core://parent")
        .unwrap();
    s.connection().execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON edges WHEN NEW.name='b' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = count(&s, "nodes");
    assert!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None))
            .is_err()
    );
    assert_eq!(count(&s, "nodes"), before);
    assert!(graph::resolve(s.connection(), "", "core://parent/a").is_err());
    s.connection()
        .execute_batch("DROP TRIGGER fail_import")
        .unwrap();
    s.write("", None, |c| {
        graph::create(c, "", "other://", "replacement", "", 0, None)
    })
    .unwrap();
    let replacement =
        graph::resolve(s.connection(), "", "other://replacement").unwrap();
    // Simulate replacement independently of the derived search index.
    s.connection()
        .execute(
            "UPDATE paths SET node_uuid=?1 WHERE namespace='' AND domain='core' AND path='parent'",
            [&replacement],
        )
        .unwrap();
    assert!(matches!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None)),
        Err(Error::Conflict(_))
    ));
}

// Preflight must collect errors below invalid parents and reject the whole tree without audit writes.
#[test]
fn preflight_collects_invalid_descendants_collisions_and_encoding() {
    let source = Source::new();
    fs::create_dir(source.0.join("bad dir")).unwrap();
    fs::write(source.0.join("bad dir/child!.txt"), "x").unwrap();
    fs::write(source.0.join("bad dir/child.txt"), "x").unwrap();
    fs::write(source.0.join("bad name.txt"), "x").unwrap();
    fs::write(source.0.join("broken.txt"), [255]).unwrap();
    for name in ["one", "two"] {
        fs::create_dir(source.0.join(name)).unwrap();
        fs::write(source.0.join(format!("{name}.md")), "x").unwrap();
    }
    fs::write(source.0.join("good.txt"), "x").unwrap();
    let s = Store::memory().unwrap();
    let plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "core://",
        false,
    )
    .unwrap();
    assert!(!plan.is_valid());
    let preview = plan.preview();
    assert!(preview["issue_count"].as_u64().unwrap() >= 8);
    let errors = preview["issues"].to_string();
    for name in [
        "bad dir", "child!", "bad name", "broken", "one.md", "two.md",
    ] {
        assert!(errors.contains(name), "{errors}");
    }
    assert!(
        !preview["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["uri"].as_str().unwrap().contains("child"))
    );
    assert_eq!(count(&s, "changesets"), 0);
    let before = count(&s, "nodes");
    assert!(
        s.write("", None, |c| import_dir::apply(c, "", &plan, None))
            .is_err()
    );
    assert_eq!(count(&s, "nodes"), before);
    assert_eq!(count(&s, "changesets"), 0);
}

// Sanitization must report every original source that maps to the same destination.
#[test]
fn sanitize_preserves_each_original_collision_source() {
    let source = Source::new();
    for name in ["a b.txt", "a.b.md", "a-b.txt", "x!.txt"] {
        fs::write(source.0.join(name), "x").unwrap();
    }
    let s = Store::memory().unwrap();
    let plan = import_dir::plan_with_sanitize(
        s.connection(),
        "",
        &source.0,
        "core://",
        true,
    )
    .unwrap();
    let preview = plan.preview();
    assert_eq!(preview["issue_count"], 3);
    let message = preview["issues"].to_string();
    for name in ["a b.txt", "a.b.md", "a-b.txt"] {
        assert!(message.contains(name), "{message}");
    }
    assert_eq!(preview["mappings"].as_array().unwrap().len(), 4);
    assert_eq!(count(&s, "changesets"), 0);
}
