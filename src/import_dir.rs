use crate::{
    error::{Error, Result},
    graph,
    uri::Uri,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Entry {
    source: PathBuf,
    uri: String,
    parent: String,
    title: String,
    content: String,
    directory: bool,
}

#[derive(Debug)]
pub struct Plan {
    source: PathBuf,
    namespace: String,
    into: String,
    parent_uuid: String,
    entries: Vec<Entry>,
    skipped: Vec<Value>,
    issues: Vec<Value>,
    problem_sources: BTreeSet<PathBuf>,
}

impl Plan {
    pub fn preview(&self) -> Value {
        json!({"source": self.source.to_string_lossy(), "into": self.into,
            "valid": self.is_valid(), "issues": self.issues, "issue_count": self.issues.len(),
            "problem_source_count": self.problem_sources.len(),
            "mappings": self.entries.iter().map(|e| json!({
                "source": e.source.to_string_lossy(), "uri": e.uri,
                "kind": if e.directory { "directory" } else { "file" }
            })).collect::<Vec<_>>(), "skipped": self.skipped})
    }
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        if self.is_valid() {
            return Ok(());
        }
        Err(validation_error(&self.issues, self.problem_sources.len()))
    }

    fn io_issue(
        &mut self, source: &Path, uri: Option<&str>, error: std::io::Error,
    ) {
        self.issue(source, uri, &error, false);
        if error.kind() != std::io::ErrorKind::InvalidData {
            self.issues.last_mut().unwrap()["kind"] = json!("storage");
        }
    }

    fn issue(
        &mut self, source: &Path, uri: Option<&str>, message: impl ToString,
        conflict: bool,
    ) {
        self.problem_sources.insert(source.to_path_buf());
        self.issues.push(
            json!({"source": source.to_string_lossy(), "uri": uri, "message": message.to_string(),
            "kind": if conflict { "conflict" } else { "invalid" }}),
        );
    }
}

fn validation_error(issues: &[Value], problem_source_count: usize) -> Error {
    let code = if issues.iter().any(|i| i["kind"] == "storage") {
        5
    } else if issues.iter().any(|i| i["kind"] == "conflict") {
        4
    } else {
        2
    };
    let samples: Vec<Value> = issues.iter().take(3).cloned().collect();
    let message = format!(
        "import validation failed: issue_count={}, problem_source_count={}; samples={}",
        issues.len(),
        problem_source_count,
        serde_json::json!(samples)
    );
    Error::ImportValidation {
        code,
        message,
        issue_count: issues.len(),
        problem_source_count,
        samples,
    }
}

fn sanitize_name(raw: &str) -> String {
    let mut title = String::new();
    let mut pending_separator = false;
    for c in raw.chars() {
        if crate::uri::is_name_char(c) {
            if pending_separator && !title.is_empty() {
                title.push('-');
            }
            pending_separator = false;
            title.push(c);
        } else {
            pending_separator = true;
        }
    }
    title
}

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::Storage(format!("{}: {error}", path.display()))
}

fn unused(c: &Connection, ns: &str, uri: &str) -> Result<()> {
    let u = Uri::parse(uri)?;
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3)",
        params![ns, u.domain, u.path],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict(format!("path already exists: {uri}")));
    }
    Ok(())
}

pub fn plan(
    c: &Connection, ns: &str, directory: &Path, into: &str,
) -> Result<Plan> {
    let result = plan_with_sanitize(c, ns, directory, into, false)?;
    result.validate()?;
    Ok(result)
}

pub fn plan_with_sanitize(
    c: &Connection, ns: &str, directory: &Path, into: &str, sanitize: bool,
) -> Result<Plan> {
    let target = Uri::parse(into)?;
    if target.domain == "system" {
        return Err(Error::Invalid("system domain is reserved".into()));
    }
    let metadata =
        fs::symlink_metadata(directory).map_err(|e| io_error(directory, e))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Invalid(format!(
            "source must be a real directory: {}",
            directory.display()
        )));
    }
    let mut result = Plan {
        source: directory.to_path_buf(),
        namespace: ns.into(),
        into: target.display(),
        parent_uuid: graph::resolve(c, ns, into)?,
        entries: Vec::new(),
        skipped: Vec::new(),
        issues: Vec::new(),
        problem_sources: BTreeSet::new(),
    };
    walk(directory, Some(&target.display()), &mut result, sanitize);
    let mut sources = BTreeMap::<String, Vec<PathBuf>>::new();
    for entry in &result.entries {
        sources
            .entry(entry.uri.clone())
            .or_default()
            .push(entry.source.clone());
    }
    for (uri, paths) in sources {
        if paths.len() > 1 {
            for source in &paths {
                result.issue(
                    source,
                    Some(&uri),
                    format!(
                        "multiple sources map to {uri}: source_count={}; samples={}",
                        paths.len(),
                        paths
                            .iter()
                            .take(3)
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    true,
                );
            }
        }
        match unused(c, ns, &uri) {
            Ok(()) => {}
            Err(error @ Error::Conflict(_)) => {
                for source in &paths {
                    result.issue(source, Some(&uri), &error, true);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

fn walk(
    directory: &Path, parent: Option<&str>, plan: &mut Plan, sanitize: bool,
) {
    let children = match fs::read_dir(directory) {
        Ok(children) => children,
        Err(error) => {
            plan.io_issue(directory, parent, error);
            return;
        }
    };
    let mut paths = Vec::new();
    for child in children {
        match child {
            Ok(child) => paths.push(child),
            Err(error) => plan.io_issue(directory, parent, error),
        }
    }
    paths.sort_by_key(|entry| entry.file_name());
    for child in paths {
        let source = child.path();
        let name = child.file_name();
        let kind = match child.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                plan.io_issue(&source, None, error);
                continue;
            }
        };
        let reason = if name.as_encoded_bytes().first() == Some(&b'.') {
            Some("hidden")
        } else if kind.is_symlink() {
            Some("symlink")
        } else if !kind.is_dir() && !kind.is_file() {
            Some("unsupported file type")
        } else if kind.is_file()
            && !source
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| {
                    s.eq_ignore_ascii_case("md")
                        || s.eq_ignore_ascii_case("txt")
                })
        {
            Some("unsupported extension")
        } else {
            None
        };
        if let Some(reason) = reason {
            plan.skipped.push(
                json!({"path": source.to_string_lossy(), "reason": reason}),
            );
            continue;
        }
        let raw = if kind.is_dir() {
            name.to_str()
        } else {
            source.file_stem().and_then(|s| s.to_str())
        };
        let title = raw.map(|s| {
            if sanitize {
                sanitize_name(s)
            } else {
                s.to_owned()
            }
        });
        let mut uri = None;
        if let Some(title) = &title {
            if let Err(error) = crate::uri::validate_name(title, "title") {
                plan.issue(&source, None, error, false);
            } else if let Some(parent) = parent {
                let separator = if parent.ends_with("://") { "" } else { "/" };
                uri = Some(format!("{parent}{separator}{title}"));
            } else {
                plan.issue(
                    &source,
                    None,
                    "invalid ancestor prevents target mapping",
                    false,
                );
            }
        } else {
            plan.issue(&source, None, "non-UTF-8 name", false);
        }
        let content = if kind.is_dir() {
            Some(String::new())
        } else {
            match fs::read_to_string(&source) {
                Ok(content) => Some(content),
                Err(error) => {
                    plan.io_issue(&source, uri.as_deref(), error);
                    // Keep the mapping for collision detection; validate blocks all writes.
                    Some(String::new())
                }
            }
        };
        if let (Some(uri), Some(content)) = (&uri, content) {
            plan.entries.push(Entry {
                source: source.clone(),
                uri: uri.clone(),
                parent: parent.unwrap().into(),
                title: title.unwrap(),
                content,
                directory: kind.is_dir(),
            });
        }
        if kind.is_dir() {
            walk(&source, uri.as_deref(), plan, sanitize);
        }
    }
}

pub fn apply(
    c: &Connection, ns: &str, plan: &Plan, attrs: Option<&Value>,
) -> Result<Value> {
    apply_with_metadata(c, ns, plan, attrs, 0, None)
}

pub fn apply_with_metadata(
    c: &Connection, ns: &str, plan: &Plan, attrs: Option<&Value>,
    priority: i64, disclosure: Option<&str>,
) -> Result<Value> {
    plan.validate()?;
    graph::validate_priority(priority)?;
    if c.is_autocommit() {
        return Err(Error::Invalid(
            "import-dir apply requires a transaction".into(),
        ));
    }
    if ns != plan.namespace {
        return Err(Error::Invalid("import plan namespace changed".into()));
    }
    if attrs.is_some_and(|v| !v.is_object()) {
        return Err(Error::Invalid("attrs must be an object".into()));
    }
    let parent = graph::resolve(c, ns, &plan.into)?;
    if parent != plan.parent_uuid {
        return Err(Error::Conflict(format!(
            "import parent changed: {}",
            plan.into
        )));
    }
    let mut conflicts = Vec::new();
    for entry in &plan.entries {
        match unused(c, ns, &entry.uri) {
            Ok(()) => {}
            Err(error @ Error::Conflict(_)) => conflicts.push(json!({
                "source": entry.source.to_string_lossy(), "uri": entry.uri,
                "message": error.to_string(), "kind": "conflict"
            })),
            Err(error) => return Err(error),
        }
    }
    if !conflicts.is_empty() {
        return Err(validation_error(&conflicts, conflicts.len()));
    }
    for entry in &plan.entries {
        graph::create_record(
            c,
            ns,
            &entry.parent,
            &entry.title,
            &entry.content,
            priority,
            disclosure,
        )?;
        if let Some(value) = attrs {
            crate::attrs::set(c, ns, &entry.uri, value)?;
        }
    }
    let directories = plan.entries.iter().filter(|e| e.directory).count();
    let mut result = json!({"imported": plan.entries.len(), "files": plan.entries.len() - directories,
        "directories": directories, "skipped": plan.skipped});
    if let Some(attrs) = attrs {
        let warnings = crate::attrs::warnings(attrs);
        if !warnings.is_empty() {
            result["warnings"] = json!(warnings);
        }
    }
    Ok(result)
}
