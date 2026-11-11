use nocturne::{
    attrs, glossary, graph,
    search::{self, MatchMode},
    store::Store,
};
use serde_json::json;

// Shared-node refresh must keep namespace-local keywords private and deny attrs after the local route is removed.
#[test]
fn shared_node_keywords_and_attrs_do_not_leak() {
    let s = Store::memory().unwrap();
    let created = s
        .write("one", None, |c| {
            graph::create(c, "one", "core://", "shared", "common", 0, None)
        })
        .unwrap();
    let c = s.connection();
    let id = created["node_uuid"].as_str().unwrap();
    c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) SELECT 'two',domain,path,edge_id,node_uuid FROM paths WHERE namespace='one'", []).unwrap();
    s.write("one", None, |c| {
        glossary::add(c, "one", id, "private_marker")
    })
    .unwrap();
    s.write("one", None, |c| {
        attrs::set(c, "one", "core://shared", &json!({"secret":"yes"}))
    })
    .unwrap();
    s.write("two", None, |c| {
        graph::update(c, "two", "core://shared", "common updated", None)
    })
    .unwrap();
    for mode in [MatchMode::All, MatchMode::Any, MatchMode::Contains] {
        for (ns, count) in [("one", 1), ("two", 0)] {
            let hits = search::search_with_mode(
                c,
                ns,
                Some("private_marker"),
                None,
                10,
                mode,
                &[],
            )
            .unwrap();
            assert_eq!(hits.as_array().unwrap().len(), count, "{ns} {mode:?}");
        }
    }
    s.write("one", None, |c| graph::unlink(c, "one", "core://shared"))
        .unwrap();
    assert!(attrs::get(c, "one", "core://shared").is_err());
    let filters = [("secret".into(), "yes".into())];
    assert_eq!(
        search::search_filtered(c, "one", None, None, 10, false, &filters)
            .unwrap(),
        json!([])
    );
    assert_eq!(
        search::search_filtered(c, "two", None, None, 10, false, &filters)
            .unwrap()[0]["node_uuid"],
        id
    );
}

// Removing a dictionary keyword must retokenize other bodies through Store::write, not merely delete its binding.
#[test]
fn keyword_removal_retokenizes_remaining_body() {
    let s = Store::memory().unwrap();
    let target = s
        .write("one", None, |c| {
            graph::create(
                c,
                "one",
                "core://",
                "target",
                "dictionary owner",
                0,
                None,
            )
        })
        .unwrap();
    let body = s
        .write("one", None, |c| {
            graph::create(
                c,
                "one",
                "core://",
                "document",
                "超导量子计算平台",
                0,
                None,
            )
        })
        .unwrap();
    let c = s.connection();
    let id = target["node_uuid"].as_str().unwrap();
    s.write("one", None, |c| {
        glossary::add(c, "one", id, "超导量子计算平台")
    })
    .unwrap();
    let terms = || {
        c.query_row("SELECT search_terms FROM search_documents WHERE namespace='one' AND node_uuid=?1", [body["node_uuid"].as_str().unwrap()], |r| r.get::<_, String>(0)).unwrap()
    };
    assert!(terms().split_whitespace().any(|t| t == "超导量子计算平台"));
    s.write("one", None, |c| {
        glossary::remove(c, "one", id, "超导量子计算平台")
    })
    .unwrap();
    assert!(!terms().split_whitespace().any(|t| t == "超导量子计算平台"));
    let hits = search::search(c, "one", "超导量子计算平台", None, 10).unwrap();
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["node_uuid"], body["node_uuid"]);
}

// Non-CJK Unicode scripts must be tokenized into FTS rather than dropped as punctuation.
#[test]
fn non_cjk_unicode_scripts_tokenize_and_match() {
    let s = Store::memory().unwrap();
    let doc = s
        .write("", None, |c| {
            graph::create(
                c,
                "",
                "core://",
                "clinker",
                "Phase β-C2S by Müller with кварц and cafe\u{301}",
                0,
                None,
            )
        })
        .unwrap();
    let c = s.connection();
    for query in ["β", "β C2S", "Müller", "muller", "кварц", "café"] {
        let hits = search::search(c, "", query, None, 10).unwrap();
        assert_eq!(hits.as_array().unwrap().len(), 1, "query={query}");
        assert_eq!(hits[0]["node_uuid"], doc["node_uuid"], "query={query}");
    }
}
