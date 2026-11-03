use nocturne::{
    error::Error,
    search::{self, MatchMode},
    store::Store,
};
use rusqlite::params;
use serde_json::{Value, json};

fn page(
    c: &rusqlite::Connection, query: Option<&str>, page: (usize, usize),
    mode: MatchMode, filters: &[(String, String)],
) -> Value {
    search::search_page(c, "", query, Some("core"), page, mode, filters)
        .unwrap()
}

// Filtering and alias deduplication must precede stable offsets in every search mode without ending caller transactions.
#[test]
fn deduplication_precedes_pagination_and_filters() {
    let store = Store::memory().unwrap();
    let c = store.connection();
    for (id, ns, domain) in [
        ("a", "", "core"),
        ("b", "", "core"),
        ("c", "", "core"),
        ("foreign", "other", "core"),
        ("domain", "", "other"),
    ] {
        c.execute("INSERT INTO nodes(uuid) VALUES(?1)", [id])
            .unwrap();
        c.execute(
            "INSERT INTO memories(node_uuid,content,deprecated) VALUES(?1,'needle haystack',0)",
            [id],
        )
        .unwrap();
        c.execute("INSERT INTO edges(parent_uuid,child_uuid,name,priority) VALUES('00000000-0000-0000-0000-000000000000',?1,?1,0)", [id]).unwrap();
        let edge = c.last_insert_rowid();
        c.execute(
            "INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?3)",
            params![ns, domain, id, edge],
        )
        .unwrap();
        if id == "a" {
            c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES('','core','aa',?1,'a')", [edge]).unwrap();
        }
    }
    search::rebuild(c, None).unwrap();
    for mode in [MatchMode::All, MatchMode::Any, MatchMode::Contains] {
        let all = page(c, Some("needle"), (10, 0), mode, &[]);
        assert_eq!(all.as_array().unwrap().len(), 3);
        for offset in 0..3 {
            assert_eq!(
                page(c, Some("needle"), (1, offset), mode, &[])[0],
                all[offset]
            );
        }
        assert_eq!(page(c, Some("needle"), (0, 0), mode, &[]), json!([]));
        assert_eq!(page(c, Some("needle"), (1, 100), mode, &[]), json!([]));
    }
    assert!(matches!(
        search::search_page(
            c,
            "",
            Some("needle"),
            None,
            (1, usize::MAX),
            MatchMode::All,
            &[]
        ),
        Err(Error::Invalid(_))
    ));
    c.execute(
        "INSERT INTO node_attrs(node_uuid,data_json) VALUES('b','{\"kind\":\"keep\"}')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO node_attrs(node_uuid,data_json) VALUES('c','{\"kind\":\"keep\"}')",
        [],
    )
    .unwrap();
    let filters = vec![("kind".into(), "keep".into())];
    let filtered = page(c, None, (10, 0), MatchMode::All, &filters);
    assert_eq!(filtered.as_array().unwrap().len(), 2);
    assert_eq!(
        page(c, None, (1, 1), MatchMode::All, &filters)[0],
        filtered[1]
    );
    assert_eq!(
        page(c, Some("needle"), (1, 1), MatchMode::Contains, &filters)[0],
        filtered[1]
    );
    assert_eq!(
        page(c, Some("needle"), (1, 1), MatchMode::All, &filters)[0],
        filtered[1]
    );
    // Search must not commit or roll back the enclosing business transaction.
    c.execute_batch("BEGIN; INSERT INTO nodes(uuid) VALUES('pending')")
        .unwrap();
    page(c, Some("needle"), (1, 0), MatchMode::All, &[]);
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
}

// Streaming snippets must preserve Unicode fold expansion, embedded NUL and CRLF in large bodies.
#[test]
fn long_unicode_snippet_preserves_expansion_nul_and_crlf() {
    let store = Store::memory().unwrap();
    let c = store.connection();
    let content = format!(
        "{}\0\r\nİstanbul needle\r\n{}",
        "界".repeat(4096),
        "終".repeat(4096)
    );
    c.execute("INSERT INTO nodes(uuid) VALUES('long')", [])
        .unwrap();
    c.execute(
        "INSERT INTO memories(node_uuid,content,deprecated) VALUES('long',?1,0)",
        [&content],
    )
    .unwrap();
    c.execute("INSERT INTO edges(parent_uuid,child_uuid,name,priority) VALUES('00000000-0000-0000-0000-000000000000','long','long',0)", []).unwrap();
    c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES('','core','long',?1,'long')", [c.last_insert_rowid()]).unwrap();
    let hits = page(c, Some("needle"), (1, 0), MatchMode::Contains, &[]);
    let snippet = hits[0]["snippet"].as_str().unwrap();
    assert!(snippet.contains("\0\r\nİstanbul needle\r\n"));
    assert!(snippet.chars().count() < 100);
    let hits =
        page(c, Some("i\u{307}stanbul"), (1, 0), MatchMode::Contains, &[]);
    // SQLite lower is ASCII-only; test expansion via an indexed query instead.
    assert_eq!(hits, json!([]));
    search::rebuild(c, None).unwrap();
    let hits = page(c, Some("İstanbul"), (1, 0), MatchMode::All, &[]);
    // Tokenization also matches the ASCII suffix; snippet still searches the full query first.
    assert!(hits[0]["snippet"].as_str().unwrap().contains("İstanbul"));

    // The folded query is shorter than its original-body match. Preserve the
    // actual match end rather than inferring it from the query character count.
    let query = format!("{} x", "İ".repeat(40));
    let content = format!("{} x", "i\u{307}".repeat(40));
    c.execute(
        "UPDATE memories SET content=?1 WHERE node_uuid=\'long\'",
        [&content],
    )
    .unwrap();
    search::refresh(c, &["long".into()]).unwrap();
    let hits = page(c, Some(&query), (1, 0), MatchMode::All, &[]);
    assert_eq!(hits[0]["snippet"], content);
}
