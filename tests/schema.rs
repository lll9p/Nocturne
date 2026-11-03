use nocturne::{Store, error::Error, graph};
use rusqlite::Connection;
use std::{
    process::Command,
    sync::{Arc, Barrier},
};

// New databases must be readable while their creator is alive, including racing WAL transitions.
#[test]
fn initialized_database_is_immediately_shareable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    let store = Store::create(&path).unwrap();
    store
        .write("", None, |c| {
            graph::create(c, "", "core://", "note", "committed", 0, None)
        })
        .unwrap();
    assert!(matches!(Store::create(&path), Err(Error::Conflict(_))));
    let other = Store::open(&path).unwrap();
    assert_eq!(
        graph::read(other.connection(), "", "core://note", false).unwrap()["content"],
        "committed"
    );
    drop(other);
    store
        .connection()
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let reader = Store::open(&path).unwrap();
                assert_eq!(
                    graph::read(reader.connection(), "", "core://note", false)
                        .unwrap()["content"],
                    "committed"
                );
            })
        })
        .collect();
    for reader in readers {
        reader.join().unwrap();
    }
}

// A failed open must not let SQLite recover and rewrite somebody else's database.
#[test]
fn unrecognized_input_is_rejected_without_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.db");
    assert!(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "hot_journal_fixture"])
            .env("NOCTURNE_TEST_FOREIGN_DB", &path)
            .status()
            .unwrap()
            .success()
    );
    let files: Vec<_> = ["", "-journal", "-wal", "-shm"]
        .iter()
        .map(|suffix| dir.path().join(format!("foreign.db{suffix}")))
        .collect();
    let snapshot = || {
        files
            .iter()
            .map(|file| match std::fs::read(file) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    None
                }
                Err(error) => panic!("{error}"),
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    assert!(before[1].as_ref().unwrap().len() > 512);
    assert!(matches!(Store::open(&path), Err(Error::Invalid(_))));
    assert_eq!(snapshot(), before);

    // Truncated and non-SQLite input exercise the other header rejection branches.
    for bytes in [vec![], vec![0; 100]] {
        let path = dir.path().join("invalid.db");
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Invalid(_))));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

// Fixture process exits without destructors so SQLite leaves a real hot rollback journal.
#[test]
#[ignore]
fn hot_journal_fixture() {
    let path = std::env::var_os("NOCTURNE_TEST_FOREIGN_DB").unwrap();
    let c = Connection::open(path).unwrap();
    c.pragma_update(None, "application_id", 0x42414431_i64)
        .unwrap();
    c.execute_batch("CREATE TABLE sentinel(value BLOB); INSERT INTO sentinel VALUES(zeroblob(4096));
        PRAGMA cache_size=5; BEGIN IMMEDIATE; UPDATE sentinel SET value=zeroblob(8192);
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000)
        INSERT INTO sentinel SELECT zeroblob(4096) FROM n;").unwrap();
    std::process::exit(0);
}
