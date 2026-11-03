use crate::error::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
    time::{Duration, Instant},
};
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);
pub const APPLICATION_ID: i64 = 0x4E4D3031; // NM01: distinct from the retired Rust format.
include!(concat!(env!("OUT_DIR"), "/schema_version.rs"));
pub const ROOT: &str = "00000000-0000-0000-0000-000000000000";
pub(crate) struct Rollback<'a>(pub(crate) &'a Connection);
impl Drop for Rollback<'_> {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
    }
}
pub struct Store {
    pub(crate) conn: Connection,
}
impl Store {
    fn configure(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            conn.busy_timeout(
                deadline.saturating_duration_since(Instant::now()),
            )?;
            match conn.pragma_update(None, "journal_mode", "WAL") {
                Ok(()) => break,
                // Concurrent journal-mode transitions can return BUSY without invoking
                // SQLite's busy handler. Restart only this statement, within the same budget.
                Err(rusqlite::Error::SqliteFailure(code, _))
                    if code.code == rusqlite::ErrorCode::DatabaseBusy
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10).min(
                        deadline.saturating_duration_since(Instant::now()),
                    ));
                }
                Err(error) => {
                    return Err(Error::Storage(format!("enable WAL: {error}")));
                }
            }
        }
        conn.busy_timeout(LOCK_TIMEOUT)?;
        Ok(Self { conn })
    }
    fn initialize(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(LOCK_TIMEOUT)?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let guard = Rollback(&conn);
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.pragma_update(None, "application_id", APPLICATION_ID)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        conn.execute_batch("COMMIT")?;
        drop(guard);
        // Persist the identity in the main header before enabling WAL.
        Self::configure(conn)
    }
    pub fn create(path: &Path) -> Result<Self> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::Conflict("database already exists".into())
                } else {
                    e.into()
                }
            })?;
        let result = Connection::open(path)
            .map_err(Error::from)
            .and_then(Self::initialize);
        if result.is_err() {
            let _ = std::fs::remove_file(path);
            for suffix in ["-wal", "-shm"] {
                let _ =
                    std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
        result
    }
    pub fn open(path: &Path) -> Result<Self> {
        // Even a PRAGMA read can recover journals on a writable SQLite connection.
        let mut header = [0u8; 100];
        let mut file = File::open(path).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                Error::NotFound("database does not exist".into())
            } else {
                error.into()
            }
        })?;
        if let Err(error) = file.read_exact(&mut header) {
            return Err(if error.kind() == ErrorKind::UnexpectedEof {
                Self::unsupported_format()
            } else {
                error.into()
            });
        }
        if &header[..16] != b"SQLite format 3\0"
            || u32::from_be_bytes(header[68..72].try_into().unwrap()) as i64
                != APPLICATION_ID
            || u32::from_be_bytes(header[60..64].try_into().unwrap()) as i64
                != SCHEMA_VERSION
        {
            return Err(Self::unsupported_format());
        }
        let c = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        c.busy_timeout(LOCK_TIMEOUT)?;
        let app: i64 =
            c.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 =
            c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app != APPLICATION_ID || version != SCHEMA_VERSION {
            return Err(Self::unsupported_format());
        }
        let s = Self::configure(c)?;
        Ok(s)
    }
    fn unsupported_format() -> Error {
        Error::Invalid(format!(
            "unsupported database format; create a new database with nocturne {} (existing data is not upgraded or cleared)",
            env!("CARGO_PKG_VERSION")
        ))
    }
    pub fn memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn write<F>(
        &self, namespace: &str, batch: Option<&str>, f: F,
    ) -> Result<serde_json::Value>
    where
        F: FnOnce(&Connection) -> Result<serde_json::Value>,
    {
        if namespace.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid namespace".into()));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let _guard = Rollback(&self.conn);
        let result = (|| {
            let id = uuid::Uuid::new_v4().to_string();
            self.conn.execute(
                "INSERT INTO changesets(id,batch,namespace) VALUES(?1,?2,?3)",
                rusqlite::params![id, batch, namespace],
            )?;
            self.conn
                .execute("INSERT INTO audit_context VALUES(?1)", [&id])?;
            let mut value = f(&self.conn)?;
            if crate::changes::diff(&self.conn, namespace, &id, false)?
                .as_array()
                .is_some_and(|rows| rows.is_empty())
            {
                self.conn.execute_batch("ROLLBACK")?;
                if !value.is_object() {
                    value = serde_json::json!({"data":value});
                }
                value["changed"] = serde_json::json!(false);
                return Ok(value);
            }
            let mut q=self.conn.prepare("SELECT DISTINCT coalesce(json_extract(after_row,'$.node_uuid'),json_extract(before_row,'$.node_uuid'),json_extract(after_row,'$.uuid'),json_extract(before_row,'$.uuid'),json_extract(after_row,'$.child_uuid'),json_extract(before_row,'$.child_uuid')) FROM change_rows WHERE changeset_id=?1 AND table_name<>'node_attrs'")?;
            let nodes = q
                .query_map([&id], |r| r.get::<_, Option<String>>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if !nodes.is_empty() {
                crate::search::refresh(&self.conn, &nodes)?;
            }
            if self.conn.query_row("SELECT count(*) FROM change_rows WHERE changeset_id=?1 AND table_name='glossary_keywords'",[&id],|r|r.get::<_,i64>(0))?>0{crate::search::rebuild(&self.conn,Some(namespace))?;}
            if let Some(obj) = value.as_object_mut() {
                obj.insert("change_id".into(), serde_json::json!(id));
            } else {
                value = serde_json::json!({"change_id":id,"data":value});
            }
            value["changed"] = serde_json::json!(true);
            self.conn.execute("DELETE FROM audit_context", [])?;
            Ok(value)
        })();
        match result {
            Ok(v) => {
                if !self.conn.is_autocommit() {
                    self.conn.execute_batch("COMMIT")?;
                }
                Ok(v)
            }
            Err(e) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(e)
            }
        }
    }
}

pub struct TransactionGuard<'a>(&'a Connection);
impl<'a> TransactionGuard<'a> {
    pub fn new(c: &'a Connection) -> Self {
        Self(c)
    }
}
impl Drop for TransactionGuard<'_> {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("ROLLBACK");
        }
    }
}
