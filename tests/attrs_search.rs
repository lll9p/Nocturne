use nocturne::{
    attrs, graph,
    search::{self, MatchMode},
    store::Store,
};
use serde_json::json;

// Attr filters must compare literal top-level string keys, never JSON paths, array members or coerced scalars.
#[test]
fn exact_filters_do_not_interpret_json_paths() {
    let s = Store::memory().unwrap();
    for (name, value) in [
        (
            "literal",
            json!({"a.b":"123", "x[0]":"yes", "quote\"":"safe"}),
        ),
        ("nested", json!({"a":{"b":"123"}, "x":["yes"]})),
        ("number", json!({"a.b":123})),
        ("array", json!({"a.b":["123"]})),
        ("boolean", json!({"a.b":true})),
        ("null", json!({"a.b":null})),
    ] {
        s.write("a", None, |c| {
            graph::create(c, "a", "core://", name, "needle", 0, None)?;
            attrs::set(c, "a", &format!("core://{name}"), &value)
        })
        .unwrap();
    }
    for (query, mode) in [
        (None, MatchMode::All),
        (Some("needle"), MatchMode::All),
        (Some("needle"), MatchMode::Contains),
    ] {
        for (key, value) in
            [("a.b", "123"), ("x[0]", "yes"), ("quote\"", "safe")]
        {
            let filters = [(key.into(), value.into())];
            let hits = search::search_with_mode(
                s.connection(),
                "a",
                query,
                None,
                10,
                mode,
                &filters,
            )
            .unwrap();
            assert_eq!(hits.as_array().unwrap().len(), 1);
            assert_eq!(hits[0]["path"], "literal");
        }
        let filters = [("a.b' OR 1=1 --".into(), "123".into())];
        assert_eq!(
            search::search_with_mode(
                s.connection(),
                "a",
                query,
                None,
                10,
                mode,
                &filters
            )
            .unwrap(),
            json!([])
        );
    }
}
