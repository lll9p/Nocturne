use nocturne::{stats::stats, store::Store};
use rusqlite::Connection;

// Stats savepoints must preserve caller writes and transactions on both success and SQL failure.
#[test]
fn nested_store_write_and_error_preserve_transaction() {
    let s = Store::memory().unwrap();
    s.write("other", None, |c| {
        let before = c.total_changes();
        let v = stats(c)?;
        assert!(!c.is_autocommit());
        assert_eq!(c.total_changes(), before);
        assert_eq!(v["review"]["pending"]["changesets"], 1);
        Ok(v)
    })
    .unwrap();
    assert!(s.connection().is_autocommit());
    let c = s.connection();
    c.execute_batch("BEGIN; DROP TABLE search_documents")
        .unwrap();
    assert!(stats(c).is_err());
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    assert!(stats(c).is_ok());
    let empty = Connection::open_in_memory().unwrap();
    assert!(stats(&empty).is_err());
    assert!(empty.is_autocommit());
}
