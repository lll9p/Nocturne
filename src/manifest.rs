//! Versioned, explicit metadata for directory round trips.
use crate::{
    error::{Error, Result},
    graph,
    uri::{Uri, validate_name},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    path::{Component, Path},
};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone)]
pub struct Manifest {
    pub version: u32,
    pub namespace: String,
    pub paths: Vec<Entry>,
}
#[derive(Clone)]
pub struct Entry {
    pub uri: String,
    pub node: String,
    pub edge: i64,
    pub parent: String,
    pub name: String,
    pub priority: i64,
    pub disclosure: Option<String>,
    pub attrs: Value,
    pub file: Option<String>,
    pub directory: Option<String>,
}
pub(crate) fn export_metadata(
    c: &Connection, ns: &str, rows: &[(String, String, i64, i64)],
    parents: &BTreeSet<(&str, &str)>,
) -> Result<Manifest> {
    let mut paths = Vec::new();
    for (domain, path, _, bytes) in rows {
        let (node,edge,parent,name,priority,disclosure) = c.query_row("SELECT p.node_uuid,e.id,e.parent_uuid,e.name,e.priority,e.disclosure FROM paths p JOIN edges e ON e.id=p.edge_id WHERE p.namespace=?1 AND p.domain=?2 AND p.path=?3",params![ns,domain,path],|r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<String>>(5)?)))?;
        let directory = parents.contains(&(domain.as_str(), path.as_str()));
        paths.push(Entry {
            uri: format!("{domain}://{path}"),
            attrs: crate::attrs::for_node(c, &node)?,
            node,
            edge,
            parent,
            name,
            priority,
            disclosure,
            file: if *bytes != 0 || !directory {
                Some(format!("{domain}/{path}.md"))
            } else {
                None
            },
            directory: if directory {
                Some(format!("{domain}/{path}"))
            } else {
                None
            },
        });
    }
    Ok(Manifest {
        version: 1,
        namespace: ns.into(),
        paths,
    })
}

pub struct Plan {
    manifest: Manifest,
    bodies: BTreeMap<String, String>,
    parents: BTreeMap<String, String>,
    issues: Vec<Value>,
    directory: std::path::PathBuf,
    manifest_path: std::path::PathBuf,
    namespace: String,
    sources: BTreeMap<
        std::path::PathBuf,
        (
            u64,
            Option<std::time::SystemTime>,
            Option<std::time::SystemTime>,
        ),
    >,
}
impl Plan {
    pub fn preview(&self) -> Value {
        json!({"valid":self.issues.is_empty(),"issues":self.issues,"mappings":self.manifest.paths.iter().map(|e|json!({"uri":e.uri,"file":e.file,"directory":e.directory})).collect::<Vec<_>>()})
    }
    pub fn validate(&self) -> Result<()> {
        if self.issues.is_empty() {
            Ok(())
        } else {
            let sources: BTreeSet<_> = self
                .issues
                .iter()
                .filter_map(|i| i["source"].as_str())
                .collect();
            Err(crate::import_dir::validation_error(
                &self.issues,
                sources.len(),
            ))
        }
    }
}
fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
// Check every component rather than canonicalizing: even an internal symlink is rejected.
fn inside(
    base: &Path, relative: &Path, directory: bool,
) -> Result<std::path::PathBuf> {
    if relative.as_os_str().is_empty() {
        return Err(invalid("empty manifest reference"));
    }
    let mut p = base.to_path_buf();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(invalid(
                "manifest references must be relative without traversal",
            ));
        };
        p.push(name);
        if fs::symlink_metadata(&p)?.file_type().is_symlink() {
            return Err(invalid("symlink in manifest reference"));
        }
    }
    let meta = fs::metadata(&p)?;
    if if directory {
        !meta.is_dir()
    } else {
        !meta.is_file()
    } {
        return Err(invalid("manifest reference has wrong file type"));
    }
    Ok(p)
}
pub fn plan(
    c: &Connection, ns: &str, directory: &Path, manifest_path: &Path,
) -> Result<Plan> {
    if fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Err(invalid("symlink export directory"));
    }
    let base = fs::canonicalize(directory)?;
    let absolute_directory = if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        std::env::current_dir()?.join(directory)
    };
    let relative = if manifest_path.is_absolute() {
        manifest_path
            .strip_prefix(&base)
            .or_else(|_| manifest_path.strip_prefix(&absolute_directory))
            .map_err(|_| invalid("manifest must be inside export directory"))?
    } else {
        manifest_path
            .strip_prefix(directory)
            .unwrap_or(manifest_path)
    };
    let manifest = decode(serde_json::from_slice(&fs::read(inside(
        &base, relative, false,
    )?)?)?)?;
    if manifest.version != 1 {
        return Err(invalid("unknown manifest version"));
    }
    let mut plan = Plan {
        manifest,
        bodies: BTreeMap::new(),
        parents: BTreeMap::new(),
        issues: Vec::new(),
        directory: base.clone(),
        manifest_path: relative.to_path_buf(),
        namespace: ns.into(),
        sources: BTreeMap::new(),
    };
    let manifest_file = inside(&base, relative, false)?;
    plan.sources
        .insert(manifest_file.clone(), stamp(&manifest_file)?);
    let mut paths = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut references = BTreeSet::new();
    let mut portable = BTreeMap::new();
    for e in &plan.manifest.paths {
        let u = Uri::parse(&e.uri)?;
        if u.domain == "system" || u.path.is_empty() {
            return Err(invalid("manifest requires writable non-root URIs"));
        }
        for name in u.path.split('/') {
            validate_name(name, "manifest path")?;
        }
        validate_name(&e.name, "edge name")?;
        if !(0..=999).contains(&e.priority)
            || !e.attrs.is_object()
            || e.node.is_empty()
            || e.parent.is_empty()
        {
            return Err(invalid("invalid manifest metadata"));
        }
        if paths.insert(e.uri.clone(), e.node.clone()).is_some() {
            return Err(invalid("duplicate manifest URI"));
        }
        let metadata = (
            e.parent.clone(),
            e.node.clone(),
            e.name.clone(),
            e.priority,
            e.disclosure.clone(),
        );
        if edges
            .insert(e.edge, metadata.clone())
            .is_some_and(|old| old != metadata)
        {
            return Err(invalid("inconsistent edge metadata"));
        }
        for (reference, is_dir) in e
            .file
            .iter()
            .map(|p| (p, false))
            .chain(e.directory.iter().map(|p| (p, true)))
        {
            let parts: Vec<_> = Path::new(reference).components().collect();
            let mut path = std::path::PathBuf::new();
            for (i, part) in parts.iter().enumerate() {
                let Component::Normal(name) = part else {
                    return Err(invalid("invalid manifest reference"));
                };
                crate::export_dir::component(name.to_str().ok_or_else(
                    || invalid("non-Unicode manifest reference"),
                )?)?;
                path.push(name);
                let directory = i + 1 < parts.len() || is_dir;
                let key: String = path
                    .to_string_lossy()
                    .nfc()
                    .flat_map(char::to_uppercase)
                    .collect();
                if let Some((previous, previous_dir)) =
                    portable.insert(key, (path.clone(), directory))
                    && (previous != path || !directory || !previous_dir)
                {
                    return Err(invalid(
                        "manifest file/directory or case collision",
                    ));
                }
            }
        }
        let body = if let Some(file) = &e.file {
            if !references.insert(file.clone()) {
                return Err(invalid("duplicate file reference"));
            }
            let source = inside(&base, Path::new(file), false)?;
            plan.sources.insert(source.clone(), stamp(&source)?);
            fs::read_to_string(source)?
        } else {
            if e.directory.is_none() {
                return Err(invalid("missing file or empty directory"));
            }
            String::new()
        };
        if let Some(dir) = &e.directory {
            let source = inside(&base, Path::new(dir), true)?;
            plan.sources.insert(source.clone(), stamp(&source)?);
        }
        if plan
            .bodies
            .insert(e.node.clone(), body.clone())
            .is_some_and(|old| old != body)
        {
            return Err(invalid("shared node has inconsistent content"));
        }
        if plan
            .manifest
            .paths
            .iter()
            .any(|other| other.node == e.node && other.attrs != e.attrs)
        {
            return Err(invalid("shared node has inconsistent attrs"));
        }
    }
    let mut external_identities = BTreeMap::new();
    for e in &plan.manifest.paths {
        let u = Uri::parse(&e.uri)?;
        let parent_uri = format!(
            "{}://{}",
            u.domain,
            u.path.rsplit_once('/').map_or("", |p| p.0)
        );
        if let Some(parent) = paths.get(&parent_uri) {
            if parent != &e.parent {
                return Err(invalid("parent identity mismatch"));
            }
        } else {
            match graph::resolve(c,ns,&parent_uri) {
                Ok(id) => {
                    if external_identities.insert(e.parent.clone(),id.clone()).is_some_and(|old|old != id) {plan.issues.push(json!({"source":e.uri,"uri":e.uri,"kind":"invalid","message":"shared external parent resolves to different target identities"}));}
                    plan.parents.insert(parent_uri,id);
                }
                Err(Error::NotFound(_)) => plan.issues.push(json!({"source":e.uri,"uri":e.uri,"kind":"invalid","message":format!("required parent {parent_uri} must already exist in namespace {ns:?}")})),
                Err(error) => return Err(error),
            }
        }
        if graph::resolve(c, ns, &e.uri).is_ok() {
            plan.issues.push(json!({"source":e.uri,"uri":e.uri,"kind":"conflict","message":format!("target URI already exists: {}",e.uri)}));
        }
    }
    let selected_nodes: BTreeSet<_> = paths.values().collect();
    for e in &plan.manifest.paths {
        if external_identities.contains_key(&e.parent)
            && selected_nodes.contains(&e.parent)
        {
            plan.issues.push(json!({"source":e.uri,"uri":e.uri,"kind":"invalid","message":"source parent is both selected and external; target cannot preserve its identity"}));
        }
    }
    // Kahn's algorithm keeps only nodes and direct edges, even for deep chains.
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut unique_links = BTreeSet::new();
    for (parent, child, ..) in edges.values() {
        if unique_links.insert((parent.as_str(), child.as_str())) {
            indegree.entry(parent).or_default();
            *indegree.entry(child).or_default() += 1;
            children.entry(parent).or_default().push(child);
        }
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(&node, &degree)| (degree == 0).then_some(node))
        .collect();
    let mut visited = 0;
    while let Some(node) = ready.pop_front() {
        visited += 1;
        if let Some(children) = children.get(node) {
            for &child in children {
                let degree = indegree.entry(child).or_default();
                *degree -= 1;
                if *degree == 0 {
                    ready.push_back(child);
                }
            }
        }
    }
    if visited != indegree.len() {
        return Err(invalid("manifest graph contains a cycle"));
    }
    Ok(plan)
}

pub fn apply(c: &Connection, ns: &str, plan: &Plan) -> Result<Value> {
    if c.is_autocommit() {
        return Err(invalid(
            "manifest import requires Store::write transaction",
        ));
    }
    plan.validate()?;
    if ns != plan.namespace {
        return Err(invalid("manifest plan namespace changed"));
    }
    let fresh = self::plan(c, ns, &plan.directory, &plan.manifest_path)?;
    fresh.validate()?;
    if fresh.manifest.value() != plan.manifest.value()
        || fresh.bodies != plan.bodies
        || fresh.parents != plan.parents
        || fresh.sources != plan.sources
    {
        return Err(invalid(
            "manifest source or target parent changed after planning",
        ));
    }
    c.execute_batch("SAVEPOINT nocturne_manifest")?;
    let result = (|| {
        for (uri, id) in &plan.parents {
            if graph::resolve(c, ns, uri)? != *id {
                return Err(invalid(format!("parent identity changed: {uri}")));
            }
        }
        for e in &plan.manifest.paths {
            if graph::resolve(c, ns, &e.uri).is_ok() {
                return Err(Error::Conflict(format!(
                    "target URI already exists: {}",
                    e.uri
                )));
            }
        }
        let mut entries: Vec<_> = plan.manifest.paths.iter().collect();
        entries.sort_by_key(|e| e.uri.matches('/').count());
        let mut nodes = BTreeMap::<String, String>::new();
        let mut edge_ids = BTreeMap::<i64, i64>::new();
        for e in entries {
            let u = Uri::parse(&e.uri)?;
            let (parent_path, title) =
                u.path.rsplit_once('/').unwrap_or(("", &u.path));
            let parent_uri = format!("{}://{parent_path}", u.domain);
            let parent = graph::resolve(c, ns, &parent_uri)?;
            if let Some(node) = nodes.get(&e.node) {
                // Restore the explicit snapshot view, never alias()'s derived descendants.
                let edge = if let Some(&edge) = edge_ids.get(&e.edge) {
                    let matches: bool = c.query_row(
                        "SELECT parent_uuid=?2 AND child_uuid=?3 FROM edges WHERE id=?1",
                        params![edge,parent,node], |r|r.get(0))?;
                    if !matches {
                        return Err(invalid("import edge identity mismatch"));
                    }
                    edge
                } else {
                    c.execute("INSERT INTO edges(parent_uuid,child_uuid,name,priority,disclosure) VALUES(?1,?2,?3,?4,?5)",params![parent,node,e.name,e.priority,e.disclosure])?;
                    let edge = c.last_insert_rowid();
                    edge_ids.insert(e.edge, edge);
                    edge
                };
                c.execute("INSERT INTO paths(namespace,domain,path,edge_id,node_uuid) VALUES(?1,?2,?3,?4,?5)",params![ns,u.domain,u.path,edge,node])?;
            } else {
                let record = graph::create_record(
                    c,
                    ns,
                    &parent_uri,
                    title,
                    &plan.bodies[&e.node],
                    e.priority,
                    e.disclosure.as_deref(),
                )?;
                let node = record["node_uuid"]
                    .as_str()
                    .ok_or_else(|| invalid("create returned no node identity"))?
                    .to_owned();
                let edge: i64 = c.query_row("SELECT edge_id FROM paths WHERE namespace=?1 AND domain=?2 AND path=?3",params![ns,u.domain,u.path],|r|r.get(0))?;
                // A shared edge's canonical name need not equal this path's last segment.
                c.execute(
                    "UPDATE edges SET name=?2 WHERE id=?1",
                    params![edge, e.name],
                )?;
                crate::attrs::set(c, ns, &e.uri, &e.attrs)?;
                nodes.insert(e.node.clone(), node);
                edge_ids.insert(e.edge, edge);
            }
        }
        Ok(json!({"imported":plan.manifest.paths.len(),"namespace":ns}))
    })();
    match result {
        Ok(value) => {
            c.execute_batch("RELEASE nocturne_manifest")?;
            Ok(value)
        }
        Err(error) => {
            c.execute_batch(
                "ROLLBACK TO nocturne_manifest; RELEASE nocturne_manifest",
            )?;
            Err(error)
        }
    }
}

impl Manifest {
    pub(crate) fn value(&self) -> Value {
        json!({"version":self.version,"namespace":self.namespace,"paths":self.paths.iter().map(|e|json!({"uri":e.uri,"node":e.node,"edge":e.edge,"parent":e.parent,"name":e.name,"priority":e.priority,"disclosure":e.disclosure,"attrs":e.attrs,"file":e.file,"directory":e.directory})).collect::<Vec<_>>()})
    }
}
fn fields(v: &Value, names: &[&str]) -> Result<()> {
    let object = v
        .as_object()
        .ok_or_else(|| invalid("manifest record must be an object"))?;
    if object.len() != names.len()
        || object.keys().any(|key| !names.contains(&key.as_str()))
    {
        return Err(invalid("unknown or missing manifest fields"));
    }
    Ok(())
}
fn string(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("manifest {key} must be a string")))
}
fn nullable(v: &Value, key: &str) -> Result<Option<String>> {
    if v[key].is_null() {
        Ok(None)
    } else {
        string(v, key).map(Some)
    }
}
fn decode(v: Value) -> Result<Manifest> {
    fields(&v, &["version", "namespace", "paths"])?;
    let version = v["version"]
        .as_u64()
        .filter(|n| *n == 1)
        .ok_or_else(|| invalid("unknown manifest version"))?
        as u32;
    let mut paths = Vec::new();
    for e in v["paths"]
        .as_array()
        .ok_or_else(|| invalid("manifest paths must be an array"))?
    {
        fields(
            e,
            &[
                "uri",
                "node",
                "edge",
                "parent",
                "name",
                "priority",
                "disclosure",
                "attrs",
                "file",
                "directory",
            ],
        )?;
        paths.push(Entry {
            uri: string(e, "uri")?,
            node: string(e, "node")?,
            edge: e["edge"]
                .as_i64()
                .ok_or_else(|| invalid("invalid edge identity"))?,
            parent: string(e, "parent")?,
            name: string(e, "name")?,
            priority: e["priority"]
                .as_i64()
                .ok_or_else(|| invalid("invalid priority"))?,
            disclosure: nullable(e, "disclosure")?,
            attrs: e["attrs"].clone(),
            file: nullable(e, "file")?,
            directory: nullable(e, "directory")?,
        });
    }
    Ok(Manifest {
        version,
        namespace: string(&v, "namespace")?,
        paths,
    })
}

fn stamp(
    path: &Path,
) -> Result<(
    u64,
    Option<std::time::SystemTime>,
    Option<std::time::SystemTime>,
)> {
    let metadata = fs::symlink_metadata(path)?;
    Ok((
        metadata.len(),
        metadata.modified().ok(),
        metadata.created().ok(),
    ))
}
