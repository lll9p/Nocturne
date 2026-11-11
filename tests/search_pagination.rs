use nocturne::{
    error::Error,
    search::{self, MatchMode, PageRequest},
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
            "INSERT INTO contents(content) VALUES('needle haystack')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO memories(node_uuid,content_id,deprecated) VALUES(?1,last_insert_rowid(),0)",
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
        for with_total in [false, true] {
            for (limit, offset) in
                [(1, 0), (1, 2), (1, 3), (1, 100), (0, 0), (0, 3), (3, 0)]
            {
                let info = search::search_page_with_info(
                    c,
                    "",
                    Some("needle"),
                    Some("core"),
                    PageRequest {
                        limit,
                        offset,
                        with_total,
                    },
                    mode,
                    &[],
                )
                .unwrap();
                assert_eq!(
                    info["results"],
                    page(c, Some("needle"), (limit, offset), mode, &[]),
                    "metadata must preserve legacy ordering and deduplication"
                );
                assert_eq!(
                    info["offset"], offset,
                    "offset must be echoed without clamping"
                );
                assert_eq!(
                    info["limit"], limit,
                    "limit zero must remain a count/probe request"
                );
                let more = offset + limit < 3;
                assert_eq!(
                    info["has_more"], more,
                    "aliases and isolated namespaces/domains must not create extra pages"
                );
                assert_eq!(
                    info["total"],
                    if with_total || !more {
                        json!(3)
                    } else {
                        Value::Null
                    },
                    "exact counts include skipped and trailing nodes; exhausted probes reveal out-of-range offsets"
                );
            }
        }
        let empty = search::search_page_with_info(
            c,
            "",
            Some("missing"),
            Some("core"),
            PageRequest {
                limit: 1,
                offset: 100,
                with_total: false,
            },
            mode,
            &[],
        )
        .unwrap();
        assert_eq!(
            empty["total"], 0,
            "no matches must be distinguishable from an offset beyond existing matches"
        );
        assert_eq!(page(c, Some("needle"), (0, 0), mode, &[]), json!([]));
        assert_eq!(
            page(c, Some("needle"), (0, i64::MAX as usize), mode, &[]),
            json!([]),
            "legacy zero-limit calls must remain empty even with a large valid offset"
        );
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
    assert!(
        matches!(
            search::search_page(
                c,
                "",
                Some("needle"),
                None,
                (0, usize::MAX),
                MatchMode::All,
                &[]
            ),
            Err(Error::Invalid(_))
        ),
        "legacy zero-limit optimization must not bypass offset range validation"
    );
    assert!(
        matches!(
            search::search_page(
                c,
                "",
                Some("\"needle\""),
                None,
                (0, 100),
                MatchMode::All,
                &[]
            ),
            Err(Error::Invalid(_))
        ),
        "legacy zero-limit optimization must still reject raw FTS expressions"
    );
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
    for query in [None, Some("needle")] {
        for mode in [MatchMode::All, MatchMode::Any, MatchMode::Contains] {
            if query.is_none() && mode != MatchMode::All {
                continue;
            }
            let info = search::search_page_with_info(
                c,
                "",
                query,
                Some("core"),
                PageRequest {
                    limit: 1,
                    offset: 0,
                    with_total: true,
                },
                mode,
                &filters,
            )
            .unwrap();
            assert_eq!(
                info["total"], 2,
                "attrs must filter candidates before both counting and paging"
            );
            assert_eq!(
                info["has_more"], true,
                "exact counting must continue after the selected page"
            );
            assert_eq!(
                info["results"][0], filtered[0],
                "combined filters must preserve attrs-only ordering for equal scores"
            );
            let count = search::search_page_with_info(
                c,
                "",
                query,
                Some("core"),
                PageRequest {
                    limit: 0,
                    offset: 1,
                    with_total: true,
                },
                mode,
                &filters,
            )
            .unwrap();
            assert_eq!(
                count["total"], 2,
                "count-only must include nodes before and after offset"
            );
            assert_eq!(
                count["results"],
                json!([]),
                "count-only must never return bodies"
            );
        }
    }
    // Search must not commit or roll back the enclosing business transaction.
    c.execute_batch("BEGIN; INSERT INTO nodes(uuid) VALUES('pending')")
        .unwrap();
    page(c, Some("needle"), (1, 0), MatchMode::All, &[]);
    search::search_page_with_info(
        c,
        "",
        Some("needle"),
        Some("core"),
        PageRequest {
            limit: 0,
            offset: 0,
            with_total: true,
        },
        MatchMode::All,
        &[],
    )
    .unwrap();
    assert!(
        !c.is_autocommit(),
        "metadata search must release only its own savepoint"
    );
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
    c.execute("INSERT INTO contents(content) VALUES(?1)", [&content])
        .unwrap();
    c.execute("INSERT INTO memories(node_uuid,content_id) VALUES('long',last_insert_rowid())", []).unwrap();
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
    c.execute("INSERT INTO contents(content) VALUES(?1)", [&content])
        .unwrap();
    c.execute("UPDATE memories SET content_id=last_insert_rowid() WHERE node_uuid='long'", []).unwrap();
    search::refresh(c, &["long".into()]).unwrap();
    let hits = page(c, Some(&query), (1, 0), MatchMode::All, &[]);
    assert_eq!(hits[0]["snippet"], content);
}

// Invalid UTF-8 bodies expose accidental hydration without a query hook or extra dependency.
#[test]
fn metadata_reads_bodies_only_for_selected_page_and_releases_on_error() {
    let store = Store::memory().unwrap();
    let c = store.connection();
    for id in ["a", "b", "c"] {
        c.execute("INSERT INTO nodes(uuid) VALUES(?1)", [id])
            .unwrap();
        c.execute("INSERT INTO contents(content) VALUES('needle')", [])
            .unwrap();
        c.execute("INSERT INTO memories(node_uuid,content_id,deprecated) VALUES(?1,last_insert_rowid(),0)", [id]).unwrap();
        c.execute("INSERT INTO edges(parent_uuid,child_uuid,name,priority) VALUES('00000000-0000-0000-0000-000000000000',?1,?1,0)", [id]).unwrap();
        c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES('','core',?1,?2,?1)", params![id, c.last_insert_rowid()]).unwrap();
    }
    search::rebuild(c, None).unwrap();
    // Corrupt storage deliberately without changing indexed postings.
    c.execute_batch("DROP TRIGGER contents_immutable; UPDATE contents SET content=x'ff' WHERE id IN (SELECT content_id FROM memories WHERE node_uuid != 'b');").unwrap();
    c.execute_batch("BEGIN; INSERT INTO nodes(uuid) VALUES('pending')")
        .unwrap();
    let info = search::search_page_with_info(
        c,
        "",
        Some("needle"),
        Some("core"),
        PageRequest {
            limit: 1,
            offset: 1,
            with_total: true,
        },
        MatchMode::All,
        &[],
    )
    .unwrap();
    assert_eq!(
        info["total"], 3,
        "exact counting must not hydrate skipped or trailing bodies"
    );
    assert_eq!(
        info["results"][0]["node_uuid"], "b",
        "only the selected valid body may be hydrated"
    );
    for with_total in [false, true] {
        let count = search::search_page_with_info(
            c,
            "",
            Some("needle"),
            Some("core"),
            PageRequest {
                limit: 0,
                offset: 0,
                with_total,
            },
            MatchMode::All,
            &[],
        )
        .unwrap();
        assert_eq!(
            count["results"],
            json!([]),
            "zero limit must not hydrate even the first matching body"
        );
    }
    assert!(
        search::search_page_with_info(
            c,
            "",
            Some("needle"),
            Some("core"),
            PageRequest {
                limit: 1,
                offset: 0,
                with_total: true
            },
            MatchMode::All,
            &[]
        )
        .is_err(),
        "selected invalid bodies must propagate conversion errors"
    );
    assert!(
        !c.is_autocommit(),
        "errors must not commit the caller transaction"
    );
    assert!(
        c.execute_batch("RELEASE search_page_snapshot").is_err(),
        "error paths must not leak the search savepoint before caller rollback"
    );
    c.execute_batch("ROLLBACK").unwrap();
    let pending: i64 = c
        .query_row("SELECT count(*) FROM nodes WHERE uuid='pending'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(pending, 0, "caller rollback must still undo its own writes");
}
