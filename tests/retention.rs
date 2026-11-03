use nocturne::{
    changes, error::Error, graph, maintenance, retention, store::Store,
};
use serde_json::Value;
fn create(s: &Store) -> Value {
    s.write("a", None, |c| {
        graph::create(c, "a", "core://", "one", "secret first", 0, None)
    })
    .unwrap()
}
fn accept(s: &Store) {
    s.connection()
        .execute("UPDATE changesets SET status='accepted'", [])
        .unwrap();
}
// Final GC must not delete another namespace's keywords or access history without explicit authorization.
#[test]
fn foreign_gc_keywords_and_logs_require_global_authorization() {
    for table in ["glossary_keywords", "memory_access_logs"] {
        let s = Store::memory().unwrap();
        let first = create(&s);
        s.write("a", None, |c| graph::unlink(c, "a", "core://one"))
            .unwrap();
        accept(&s);
        let c = s.connection();
        let node = first["node_uuid"].as_str().unwrap();
        let id = first["version"].as_i64().unwrap();
        let sql = if table == "glossary_keywords" {
            "INSERT INTO glossary_keywords(node_uuid,namespace,keyword) VALUES(?1,'b','foreign')"
        } else {
            "INSERT INTO memory_access_logs(node_uuid,namespace) VALUES(?1,'b')"
        };
        c.execute(sql, [node]).unwrap();
        let before = snapshot(&s);
        for apply in [false, true] {
            assert!(matches!(
                retention::purge(c, "a", id, false, apply),
                Err(Error::Conflict(_))
            ));
            assert_eq!(before, snapshot(&s));
        }
        let plan = retention::purge(c, "a", id, true, true).unwrap();
        assert_eq!(plan["counts"][table], 1);
        assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
    }
}

// Prevent permanent deletion from leaving recoverable secret snapshots or broken successor links.
#[test]
fn chain_audit_and_preview() {
    let s = Store::memory().unwrap();
    let first = create(&s);
    let middle = s
        .write("a", None, |c| {
            graph::update(c, "a", "core://one", "secret middle", None)
        })
        .unwrap();
    let last = s
        .write("a", None, |c| {
            graph::update(c, "a", "core://one", "last", None)
        })
        .unwrap();
    let c = s.connection();
    let id = middle["version"].as_i64().unwrap();
    assert!(matches!(
        retention::purge(c, "a", id, false, true),
        Err(Error::Conflict(_))
    ));
    accept(&s);
    assert!(matches!(
        retention::purge(c, "b", id, false, false),
        Err(Error::NotFound(_))
    ));
    let before = c.total_changes();
    let plan = retention::purge(c, "a", id, false, false).unwrap();
    assert_eq!(before, c.total_changes());
    assert_eq!(plan["chain_repairs"][0]["memory_id"], first["version"]);
    assert_eq!(plan["chain_repairs"][0]["to"], last["version"]);
    retention::purge(c, "a", id, false, true).unwrap();
    assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
    for group in plan["pruned_changesets"].as_array().unwrap() {
        assert!(matches!(
            changes::rollback(c, "a", group["id"].as_str().unwrap(), false),
            Err(Error::NotFound(_))
        ));
    }
    let copies:i64=c.query_row("SELECT count(*) FROM change_rows WHERE before_row LIKE '%secret middle%' OR after_row LIKE '%secret middle%'",[],|r|r.get(0)).unwrap();
    assert_eq!(copies, 0);
    assert!(matches!(
        retention::purge(
            c,
            "a",
            last["version"].as_i64().unwrap(),
            false,
            true
        ),
        Err(Error::Invalid(_))
    ));
}

// Prevent purge from deleting shared history while foreign pending changes still depend on it.
#[test]
fn shared_and_cross_namespace_pending() {
    let s = Store::memory().unwrap();
    let first = create(&s);
    s.write("a", None, |c| {
        graph::update(c, "a", "core://one", "next", None)
    })
    .unwrap();
    accept(&s);
    let c = s.connection();
    let id = first["version"].as_i64().unwrap();
    s.write("b",None,|c| {
        c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) SELECT 'b',domain,path,edge_id,node_uuid FROM paths WHERE namespace='a'",[])?;
        Ok(serde_json::json!({}))
    }).unwrap();
    assert!(matches!(
        retention::purge(c, "a", id, true, false),
        Err(Error::Conflict(_))
    ));
    accept(&s);
    assert!(matches!(
        retention::purge(c, "a", id, false, false),
        Err(Error::Conflict(_))
    ));
    s.write("unrelated", None, |c| {
        graph::create(c, "unrelated", "core://", "other", "unrelated", 0, None)
    })
    .unwrap();
    retention::purge(c, "a", id, true, true).unwrap();
    assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
}

fn snapshot(s: &Store) -> Vec<String> {
    let mut values = Vec::new();
    for table in [
        "nodes",
        "memories",
        "edges",
        "paths",
        "glossary_keywords",
        "memory_access_logs",
        "changesets",
        "change_rows",
    ] {
        let mut q = s
            .connection()
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = q.column_count();
        let mut rows = q.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            values.push(format!(
                "{table}:{:?}",
                (0..columns)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
            ));
        }
    }
    values
}

// Prevent deleting the newest orphan version from erasing ownership of surviving history.
#[test]
fn orphan_owner_preserved_until_final_version_gc() {
    let s = Store::memory().unwrap();
    let first = create(&s);
    let latest = s
        .write("a", None, |c| {
            graph::update(c, "a", "core://one", "last orphan", None)
        })
        .unwrap();
    s.write("a", None, |c| graph::unlink(c, "a", "core://one"))
        .unwrap();
    accept(&s);
    let c = s.connection();
    let older_id = first["version"].as_i64().unwrap();
    let latest_id = latest["version"].as_i64().unwrap();
    let before = snapshot(&s);
    for all in [false, true] {
        for apply in [false, true] {
            let err =
                retention::purge(c, "a", latest_id, all, apply).unwrap_err();
            assert!(matches!(err, Error::Conflict(_)));
            assert!(err.to_string().contains("older versions first"));
            assert_eq!(before, snapshot(&s));
        }
    }
    let older_plan = retention::purge(c, "a", older_id, false, true).unwrap();
    assert_eq!(older_plan["node_gc"], false);
    let visible = retention::deprecated(c, "a", false).unwrap();
    assert_eq!(visible.as_array().unwrap().len(), 1);
    assert_eq!(visible[0]["memory_id"], latest_id);
    assert!(
        retention::deprecated(c, "b", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        retention::purge(c, "a", latest_id, false, true).unwrap()["node_gc"],
        true
    );
    assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
}

// Prevent whole-audit pruning from erasing ownership evidence for a different surviving orphan.
#[test]
fn joint_unlink_audit_preserves_other_orphan_ownership() {
    let s = Store::memory().unwrap();
    let first = create(&s);
    let other = s
        .write("a", None, |c| {
            graph::create(
                c,
                "a",
                "core://",
                "other",
                "remaining orphan",
                0,
                None,
            )
        })
        .unwrap();
    s.write("a", None, |c| {
        graph::unlink(c, "a", "core://one")?;
        graph::unlink(c, "a", "core://other")
    })
    .unwrap();
    accept(&s);
    let c = s.connection();
    let before = snapshot(&s);
    for all in [false, true] {
        for apply in [false, true] {
            let err = retention::purge(
                c,
                "a",
                first["version"].as_i64().unwrap(),
                all,
                apply,
            )
            .unwrap_err();
            assert!(matches!(err, Error::Conflict(_)));
            assert!(
                err.to_string()
                    .contains(other["node_uuid"].as_str().unwrap())
            );
            assert_eq!(before, snapshot(&s));
        }
    }
    assert_eq!(
        retention::deprecated(c, "a", false)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        retention::deprecated(c, "b", false)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
}
