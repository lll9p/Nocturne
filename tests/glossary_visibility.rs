use nocturne::{Store, glossary, graph};

// An unlinked namespace must not see future private content of a still-shared node through glossary joins.
#[test]
fn unlinked_namespace_cannot_read_shared_nodes_future_content_via_glossary() {
    let store = Store::memory().unwrap();
    let shared = store
        .write("one", None, |c| {
            graph::create(
                c,
                "one",
                "core://",
                "shared",
                "Previously shared",
                0,
                None,
            )
        })
        .unwrap();
    let c = store.connection();
    let node = shared["node_uuid"].as_str().unwrap();
    c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) SELECT 'two',domain,path,edge_id,node_uuid FROM paths WHERE namespace='one'", []).unwrap();
    for ns in ["one", "two"] {
        store
            .write(ns, None, |c| glossary::add(c, ns, node, "shared-key"))
            .unwrap();
    }
    store
        .write("one", None, |c| graph::unlink(c, "one", "core://shared"))
        .unwrap();
    store
        .write("two", None, |c| {
            graph::update(
                c,
                "two",
                "core://shared",
                "Private future content",
                None,
            )
        })
        .unwrap();
    let inaccessible = glossary::list(c, "one", None).unwrap();
    assert_eq!(inaccessible[0]["nodes"][0]["content_snippet"], "");
    assert!(!inaccessible.to_string().contains("Private future content"));
    assert!(
        glossary::scan(c, "one", "shared-key")
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty()
    );
    let accessible = glossary::list(c, "two", None).unwrap();
    assert!(accessible.to_string().contains("Private future content"));
    assert!(
        glossary::scan(c, "two", "shared-key")
            .unwrap()
            .get("shared-key")
            .is_some()
    );
}
