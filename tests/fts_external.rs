use nocturne::{maintenance, search, store::Store};
use rusqlite::Connection;

fn seed(c: &Connection) {
    c.execute_batch("INSERT INTO nodes(uuid) VALUES('a');
        INSERT INTO contents(content) VALUES('oldword'); INSERT INTO memories(node_uuid,content_id) VALUES('a',last_insert_rowid());
        INSERT INTO edges(parent_uuid,child_uuid,name) VALUES('00000000-0000-0000-0000-000000000000','a','item');
        INSERT INTO paths(namespace,domain,path,node_uuid,edge_id) SELECT 'one','core','item','a',id FROM edges WHERE child_uuid='a';
        INSERT INTO paths(namespace,domain,path,node_uuid,edge_id) SELECT 'two','core','alias','a',id FROM edges WHERE child_uuid='a';").unwrap();
    search::rebuild(c, None).unwrap();
}
fn matches(c: &Connection, word: &str) -> i64 {
    c.query_row(
        "SELECT count(*) FROM search_documents_fts WHERE search_documents_fts MATCH ?1",
        [word],
        |r| r.get(0),
    )
    .unwrap()
}
// External-content FTS triggers must replace and cascade-delete postings across shared routes.
#[test]
fn triggers_refresh_cascade_and_null_disclosure() {
    let s = Store::memory().unwrap();
    let c = s.connection();
    seed(c);
    assert_eq!(matches(c, "oldword"), 2);
    c.execute_batch(
        "UPDATE search_documents SET disclosure=''; UPDATE search_documents SET disclosure=NULL;",
    )
    .unwrap();
    assert_eq!(maintenance::doctor(c).unwrap()["ok"], true);
    search::rebuild(c, Some("one")).unwrap();
    assert_eq!(matches(c, "oldword"), 2);
    c.execute_batch(
        "INSERT INTO contents(content) VALUES('newword'); UPDATE memories SET content_id=last_insert_rowid() WHERE node_uuid='a'",
    )
    .unwrap();
    search::refresh(c, &["a".into()]).unwrap();
    assert_eq!(matches(c, "oldword"), 0);
    assert_eq!(matches(c, "newword"), 2);
    c.execute_batch("UPDATE search_documents SET search_terms='directword' ")
        .unwrap();
    assert_eq!(matches(c, "newword"), 2);
    assert_eq!(matches(c, "directword"), 2);
    c.execute_batch("DELETE FROM memories WHERE node_uuid='a'")
        .unwrap();
    assert_eq!(matches(c, "directword"), 0);
}
