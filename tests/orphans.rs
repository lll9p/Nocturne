use nocturne::{error, graph, orphans, store::Store};
use serde_json::{Value, json};

fn create(s: &Store) -> Value {
    s.write("a", None, |c| {
        graph::create(c, "a", "core://", "one", "first token", 0, None)
    })
    .unwrap()
}
fn unlink(s: &Store, ns: &str, path: &str) {
    s.write(ns, None, |c| graph::unlink(c, ns, path)).unwrap();
}
fn restore(
    s: &Store, ns: &str, id: i64, dest: &str, all: bool,
) -> error::Result<Value> {
    s.write(ns, None, |c| {
        orphans::restore(c, ns, id, dest, 3, Some("restored"), all)
    })
}
fn share(s: &Store, node: &str) {
    s.write("b", None, |c| {
        c.execute(
            "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid)
            SELECT 'b','core','shared',id,child_uuid FROM edges WHERE child_uuid=?1",
            [node],
        )?;
        Ok(json!({}))
    })
    .unwrap();
}

// Prevent stale owners from reading or restoring private history after another namespace takes ownership.
#[test]
fn latest_delete_ownership_and_global_alias_exclusion() {
    let s = Store::memory().unwrap();
    let v = create(&s);
    let id = v["version"].as_i64().unwrap();
    share(&s, v["node_uuid"].as_str().unwrap());
    graph::read(s.connection(), "a", "core://one", true).unwrap();
    unlink(&s, "a", "core://one");
    for all in [false, true] {
        assert!(
            orphans::list(s.connection(), "a", all)
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            orphans::show(s.connection(), "a", id, all),
            Err(error::Error::NotFound(_))
        ));
        assert!(restore(&s, "a", id, "core://again", all).is_err());
    }
    s.write("b", None, |c| {
        graph::update(c, "b", "core://shared", "future private content", None)
    })
    .unwrap();
    unlink(&s, "b", "core://shared");
    assert!(
        orphans::list(s.connection(), "a", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        orphans::show(s.connection(), "a", id, false),
        Err(error::Error::NotFound(_))
    ));
    assert!(matches!(
        restore(&s, "a", id, "core://again", false),
        Err(error::Error::NotFound(_))
    ));
    assert_eq!(
        orphans::show(s.connection(), "b", id, false).unwrap()["history"][0]["content"],
        "future private content"
    );
    assert!(orphans::show(s.connection(), "a", id, true).is_ok());
    restore(&s, "a", id, "core://again", true).unwrap();
}
