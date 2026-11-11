use nocturne::{changes, error::Error, graph, store::Store};

// Prevent alias expansion from exposing another namespace's private descendants.
#[test]
fn shared_parent_alias_does_not_export_private_children() {
    let s = Store::memory().unwrap();
    let parent = s
        .write("a", None, |c| {
            graph::create(c, "a", "core://", "parent", "p", 0, None)
        })
        .unwrap();
    s.write("b",None,|c|{let node=parent["node_uuid"].as_str().unwrap();let edge:i64=c.query_row("SELECT id FROM edges WHERE child_uuid=?1",[node],|r|r.get(0))?;c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES('b','core','parent',?1,?2)",rusqlite::params![edge,node])?;Ok(serde_json::json!({}))}).unwrap();
    s.write("b", None, |c| {
        graph::create(c, "b", "core://parent", "private", "secret", 0, None)
    })
    .unwrap();
    s.write("a", None, |c| {
        graph::alias(c, "a", "core://parent", "notes://parent")
    })
    .unwrap();
    assert!(
        graph::read(s.connection(), "a", "notes://parent/private", false)
            .is_err()
    );
    assert!(
        graph::read(s.connection(), "b", "core://parent/private", false)
            .is_ok()
    );
}

// Top-level and sibling aliases must own independent edges so metadata edits never bleed across routes.
#[test]
fn aliases_isolate_edge_metadata_across_domains_and_siblings() {
    let s = Store::memory().unwrap();
    s.write("a", None, |c| {
        graph::create(
            c,
            "a",
            "core://",
            "session-policy",
            "idle expiry is 30m",
            1,
            Some("When changing session expiry"),
        )
    })
    .unwrap();
    s.write("a", None, |c| {
        graph::alias_with_meta(
            c,
            "a",
            "core://session-policy",
            "notes://session-policy",
            Some(4),
            Some("When investigating login renewal"),
        )?;
        graph::alias_with_meta(
            c,
            "a",
            "core://session-policy",
            "core://login-renewal",
            Some(7),
            Some("When debugging token refresh"),
        )
    })
    .unwrap();
    s.write("a", None, |c| {
        graph::edit(
            c,
            "a",
            "notes://session-policy",
            None,
            Some(9),
            Some("Updated notes disclosure"),
            None,
        )
    })
    .unwrap();
    let c = s.connection();
    let primary = graph::read(c, "a", "core://session-policy", false).unwrap();
    let cross_domain =
        graph::read(c, "a", "notes://session-policy", false).unwrap();
    let sibling = graph::read(c, "a", "core://login-renewal", false).unwrap();
    assert_eq!(primary["node_uuid"], cross_domain["node_uuid"]);
    assert_eq!(primary["node_uuid"], sibling["node_uuid"]);
    assert_eq!(primary["priority"], 1);
    assert_eq!(primary["disclosure"], "When changing session expiry");
    assert_eq!(cross_domain["priority"], 9);
    assert_eq!(cross_domain["disclosure"], "Updated notes disclosure");
    assert_eq!(sibling["priority"], 7);
    assert_eq!(sibling["disclosure"], "When debugging token refresh");
    assert_eq!(nocturne::maintenance::doctor(c).unwrap()["ok"], true);
}

// Prevent same-named batches from leaking foreign review data or accepting and undoing another namespace's writes.
#[test]
fn same_batch_name_is_independent_in_each_namespace() {
    let s = Store::memory().unwrap();
    let a = s
        .write("a", Some("shared-batch"), |c| {
            graph::create(c, "a", "core://", "alpha", "private a", 0, None)
        })
        .unwrap();
    let b = s
        .write("b", Some("shared-batch"), |c| {
            graph::create(c, "b", "core://", "beta", "private b", 0, None)
        })
        .unwrap();
    for (ns, own, other) in [("a", &a, &b), ("b", &b, &a)] {
        let list = changes::list(s.connection(), ns).unwrap();
        assert_eq!(list[0]["id"], own["change_id"]);
        assert!(
            !list
                .to_string()
                .contains(other["change_id"].as_str().unwrap())
        );
        let diff = changes::diff(s.connection(), ns, "shared-batch", true)
            .unwrap()
            .to_string();
        assert!(diff.contains(own["node_uuid"].as_str().unwrap()));
        assert!(!diff.contains(other["node_uuid"].as_str().unwrap()));
        assert!(matches!(
            changes::accept(
                s.connection(),
                ns,
                other["change_id"].as_str().unwrap(),
                false
            ),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            changes::rollback(
                s.connection(),
                ns,
                other["change_id"].as_str().unwrap(),
                false
            ),
            Err(Error::NotFound(_))
        ));
    }
    changes::accept(s.connection(), "a", "shared-batch", true).unwrap();
    assert_eq!(
        changes::list(s.connection(), "a").unwrap()[0]["status"],
        "accepted"
    );
    assert_eq!(
        changes::list(s.connection(), "b").unwrap()[0]["status"],
        "pending"
    );
    changes::rollback(s.connection(), "b", "shared-batch", true).unwrap();
    assert!(graph::read(s.connection(), "a", "core://alpha", false).is_ok());
    assert!(graph::read(s.connection(), "b", "core://beta", false).is_err());
    assert_eq!(
        changes::list(s.connection(), "a").unwrap()[0]["status"],
        "accepted"
    );
    changes::rollback(s.connection(), "a", "shared-batch", true).unwrap();
    assert!(graph::read(s.connection(), "a", "core://alpha", false).is_err());
}
