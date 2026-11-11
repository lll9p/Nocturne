use nocturne::{changes, graph, store::Store};
use serde_json::Value;

fn create(s: &Store, ns: &str) -> Value {
    s.write(ns, None, |c| {
        graph::create(c, ns, "core://", "one", "initial", 3, Some("original"))
    })
    .unwrap()
}

// Prevent rollback from restoring content while leaving edited metadata or the wrong active version.
#[test]
fn composite_edit_is_one_reversible_changeset() {
    let s = Store::memory().unwrap();
    let initial = create(&s, "a");
    let before = changes::list(s.connection(), "a")
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    let edited = s
        .write("a", None, |c| {
            graph::edit(
                c,
                "a",
                "core://one",
                Some("e\u{301}"),
                Some(7),
                Some("new"),
                initial["version"].as_i64(),
            )
        })
        .unwrap();
    assert_eq!(edited["content"], "e\u{301}");
    assert_eq!(edited["priority"], 7);
    assert_eq!(
        changes::list(s.connection(), "a")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        before + 1
    );
    changes::rollback(
        s.connection(),
        "a",
        edited["change_id"].as_str().unwrap(),
        false,
    )
    .unwrap();
    let restored =
        graph::read(s.connection(), "a", "core://one", false).unwrap();
    assert_eq!(restored["content"], "initial");
    assert_eq!(restored["priority"], 3);
    assert_eq!(restored["disclosure"], "original");
}

// Literal patches with "..." or "\neq" must not wildcard-match or translate escapes, and CAS must precede matching.
#[test]
fn patch_is_strictly_literal_and_version_check_precedes_match() {
    use nocturne::{error::Error, patch};
    let body = "header\nrange 0...999 and \\neq formula\ntrailer\nfooter)\n";
    let patched = patch::apply(
        body,
        "range 0...999 and \\neq formula\ntrailer)",
        "replaced",
    );
    assert!(matches!(patched, Err(Error::PatchNoMatch)));
    let exact = patch::apply(
        body,
        "range 0...999 and \\neq formula",
        "range 1...100 and \\newcommand{\\x}{\\neq}",
    )
    .unwrap();
    assert_eq!(
        exact,
        "header\nrange 1...100 and \\newcommand{\\x}{\\neq}\ntrailer\nfooter)\n"
    );

    let s = Store::memory().unwrap();
    let v1 = create(&s, "a");
    s.write("a", None, |c| {
        graph::update(c, "a", "core://one", "later content", None)
    })
    .unwrap();
    let stale = v1["version"].as_i64();
    let err = s
        .write("a", None, |c| {
            graph::check_version(c, "a", "core://one", stale)?;
            let current = graph::read(c, "a", "core://one", false)?;
            let next = patch::apply(
                current["content"].as_str().unwrap(),
                "initial",
                "updated",
            )?;
            graph::update(c, "a", "core://one", &next, stale)
        })
        .unwrap_err();
    assert!(matches!(err, Error::VersionConflict { .. }));
}
