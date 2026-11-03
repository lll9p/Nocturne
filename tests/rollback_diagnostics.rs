use nocturne::{
    attrs, changes, error::Error, graph, maintenance, store::Store,
};
use serde_json::{Value, json};

fn create(s: &Store, name: &str, batch: Option<&str>) -> Value {
    s.write("a", batch, |c| {
        graph::create(c, "a", "core://", name, "body", 0, None)
    })
    .unwrap()
}
fn snapshot(s: &Store) -> Value {
    let c = s.connection();
    let mut result = Vec::new();
    for table in [
        "nodes",
        "node_attrs",
        "memories",
        "edges",
        "paths",
        "changesets",
        "change_rows",
    ] {
        let mut q = c
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = q.column_count();
        let rows = q
            .query_map([], |r| {
                Ok((0..count)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>())
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        result.push(json!([table, rows]));
    }
    json!(result)
}

// Prevent rollback conflict evidence from leaking foreign URIs, batch names, or changeset identifiers.
#[test]
fn foreign_references_remain_private() {
    let s = Store::memory().unwrap();
    let first = create(&s, "one", None);
    let foreign = s.write("b", Some("private-batch"), |c| {
        c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid,created_at) SELECT 'b',domain,'secret',edge_id,node_uuid,created_at FROM paths WHERE namespace='a' AND path='one'", [])?;
        attrs::set(c, "b", "core://secret", &json!({"private":1}))
    }).unwrap();
    let before = snapshot(&s);
    let (message, details) = match changes::rollback(
        s.connection(),
        "a",
        first["change_id"].as_str().unwrap(),
        false,
    ) {
        Err(Error::RollbackConflict {
            message,
            blocking_uris,
            related_changesets,
            foreign_references,
        }) => (
            message,
            json!({"blocking_uris": blocking_uris,
                "related_changesets": related_changesets.iter()
                    .map(|change| json!({"id":change.id,"batch":change.batch})).collect::<Vec<_>>(),
                "foreign_references": foreign_references}),
        ),
        other => panic!("expected conflict, got {other:?}"),
    };
    assert_eq!(snapshot(&s), before);
    assert_eq!(maintenance::doctor(s.connection()).unwrap()["ok"], true);
    assert!(
        message.contains("undo subsequent changes first"),
        "{message}"
    );
    assert!(message.contains("another namespace"));
    assert_eq!(
        details,
        json!({"blocking_uris": ["core://one"],
        "related_changesets": [], "foreign_references": true})
    );
    for private in [
        "core://secret",
        "private-batch",
        foreign["change_id"].as_str().unwrap(),
    ] {
        assert!(!message.contains(private), "{message}");
        assert!(!details.to_string().contains(private), "{details}");
    }
}
