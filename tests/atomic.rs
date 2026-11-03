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
