use nocturne::{changes, error::Error, graph, paths, search, store::Store};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn create(s: &Store, ns: &str, parent: &str, name: &str) -> Value {
    s.write(ns, None, |c| {
        graph::create(c, ns, parent, name, "body", 7, Some("hint"))
    })
    .unwrap()
}
fn preset(c: &Connection, name: &str, boot: Value, active: bool) {
    c.execute("INSERT INTO presets(name,boot_uris,path_masks,is_active) VALUES(?1,?2,'[\"untouched\"]',?3)", params![name,boot.to_string(),active]).unwrap();
}
fn boots(c: &Connection, name: &str) -> Value {
    let text: String = c
        .query_row("SELECT boot_uris FROM presets WHERE name=?1", [name], |r| {
            r.get(0)
        })
        .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn count(s: &Store) -> usize {
    changes::list(s.connection(), "")
        .unwrap()
        .as_array()
        .unwrap()
        .len()
}

// Prevent subtree moves and rollback from losing descendants, startup references, or searchable routes.
#[test]
fn cross_parent_domain_inherits_metadata_and_rollback_restores_boot_and_fts() {
    let s = Store::memory().unwrap();
    create(&s, "", "core://", "sourceword");
    create(&s, "", "core://sourceword", "child");
    create(&s, "", "notes://", "parent");
    let c = s.connection();
    preset(
        c,
        "active",
        json!({"":["core://sourceword","core://sourceword/child","core://sourceword/missing"]}),
        true,
    );
    preset(c, "inactive", json!({"":["core://sourceword"]}), false);
    let created: String = c
        .query_row(
            "SELECT created_at FROM paths WHERE path='sourceword'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before = count(&s);
    let v = s
        .write("", None, |c| {
            paths::move_path(
                c,
                "",
                "core://sourceword",
                "notes://parent/targetword",
            )
        })
        .unwrap();
    assert_eq!(count(&s), before + 1);
    assert_eq!(v["boot_presets_updated"], 2);
    let r = graph::read(c, "", "notes://parent/targetword", false).unwrap();
    assert_eq!(r["priority"], 7);
    assert_eq!(r["disclosure"], "hint");
    assert_eq!(
        c.query_row(
            "SELECT created_at FROM paths WHERE path='parent/targetword'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        created
    );
    assert_eq!(boots(c, "active")[""][1], "notes://parent/targetword/child");
    assert_eq!(boots(c, "active")[""][2], "core://sourceword/missing");
    assert!(
        !search::search(c, "", "targetword", None, 10)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let id = v["change_id"].as_str().unwrap();
    assert!(
        changes::diff(c, "", id, false)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["table"] != "memories")
    );
    changes::rollback(c, "", id, false).unwrap();
    assert!(graph::resolve(c, "", "core://sourceword/child").is_ok());
    assert_eq!(boots(c, "inactive")[""][0], "core://sourceword");
    assert!(
        search::search(c, "", "targetword", None, 10)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !search::search(c, "", "sourceword", None, 10)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// Prevent a default-namespace move from silently rewriting another namespace's inherited startup routes.
#[test]
fn boot_fallback_overrides_are_namespace_local_and_default_move_freezes_inheritors()
 {
    let s = Store::memory().unwrap();
    create(&s, "", "core://", "one");
    create(&s, "a", "core://", "one");
    create(&s, "b", "core://", "one");
    let c = s.connection();
    preset(
        c,
        "fallback",
        json!({"":["core://one"],"explicit":["core://one"]}),
        true,
    );
    preset(
        c,
        "inactive",
        json!({"":["core://one"],"a":["core://one"]}),
        false,
    );
    preset(c, "unmatched", json!({"":["core://elsewhere"]}), false);
    s.write("a", None, |c| paths::rename(c, "a", "core://one", "two"))
        .unwrap();
    assert_eq!(
        boots(c, "fallback"),
        json!({"":["core://one"],"a":["core://two"],"explicit":["core://one"]})
    );
    assert_eq!(boots(c, "inactive")["a"][0], "core://two");
    s.write("", None, |c| paths::rename(c, "", "core://one", "three"))
        .unwrap();
    let map = boots(c, "fallback");
    assert_eq!(map[""][0], "core://three");
    assert_eq!(map["b"][0], "core://one");
    assert_eq!(map["a"][0], "core://two");
    assert_eq!(map["explicit"][0], "core://one");
    assert_eq!(boots(c, "unmatched"), json!({"":["core://elsewhere"]}));
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM presets WHERE path_masks='[\"untouched\"]'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
}

// Prevent undo from introducing a graph cycle through a surviving alias.
#[test]
fn rollback_move_rejects_cycle_through_later_alias() {
    let s = Store::memory().unwrap();
    create(&s, "", "core://", "a");
    create(&s, "", "core://a", "b");
    create(&s, "", "core://", "c");
    s.write("", None, |c| graph::alias(c, "", "other://b", "core://a/b"))
        .unwrap();
    let moved = s
        .write("", None, |c| {
            paths::move_path(c, "", "core://a/b", "core://c/b")
        })
        .unwrap();
    s.write("", None, |c| graph::alias(c, "", "other://b/a", "core://a"))
        .unwrap();
    let c = s.connection();
    let before = crate::changes::diff(
        c,
        "",
        moved["change_id"].as_str().unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(nocturne::maintenance::doctor(c).unwrap()["ok"], true);
    assert!(matches!(
        changes::rollback(c, "", moved["change_id"].as_str().unwrap(), false),
        Err(Error::RollbackConflict { .. })
    ));
    assert!(graph::resolve(c, "", "core://c/b").is_ok());
    assert!(graph::resolve(c, "", "other://b/a").is_ok());
    assert!(graph::resolve(c, "", "core://a/b").is_err());
    assert_eq!(
        changes::diff(c, "", moved["change_id"].as_str().unwrap(), false)
            .unwrap(),
        before
    );
    assert_eq!(nocturne::maintenance::doctor(c).unwrap()["ok"], true);
}
