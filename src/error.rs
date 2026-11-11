#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelatedChangeset {
    pub id: String,
    pub batch: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    ImportValidation {
        /// Exit code: invalid = 2, conflict = 4, storage = 5.
        code: i32,
        message: String,
        issue_count: usize,
        problem_source_count: usize,
        samples: Vec<serde_json::Value>,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{message}")]
    RollbackConflict {
        message: String,
        blocking_uris: Vec<String>,
        related_changesets: Vec<RelatedChangeset>,
        foreign_references: bool,
    },
    #[error(
        "version conflict for {uri} in namespace {namespace:?}: expected {expected}, current {current}; re-read current content and reconcile your edit before retrying"
    )]
    VersionConflict {
        uri: String,
        namespace: String,
        expected: i64,
        current: i64,
    },
    #[error("patch found no match")]
    PatchNoMatch,
    #[error(
        "patch is ambiguous: found at least {matches} matches; use a more specific match"
    )]
    PatchAmbiguous { matches: usize },
    #[error("{0}")]
    Storage(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Invalid(e.to_string())
    }
}
