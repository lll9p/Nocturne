use clap::{Args, Parser, Subcommand};
use nocturne::{
    changes,
    error::{Error, Result},
    graph,
    store::Store,
    views,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version = env!("CARGO_PKG_VERSION"),
    about = "Local, versioned memory with SQLite search and reversible writes"
)]
struct Cli {
    /// Database file; required unless NOCTURNE_DB is set (except standalone diff).
    #[arg(long, env = "NOCTURNE_DB", global = true)]
    db: Option<PathBuf>,
    /// Select the URI namespace; defaults to the empty namespace "".
    #[arg(long, env = "NOCTURNE_NAMESPACE", default_value = "", global = true)]
    namespace: String,
    /// Print successful results as JSON; errors on stderr are always JSON.
    #[arg(long, global = true)]
    json: bool,
    /// Tag writes with a namespace-local review batch label (not a multi-command transaction).
    #[arg(long, global = true, value_name = "BATCH_ID")]
    batch: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new database; refuse existing files.
    Init,
    /// Create a child memory from a UTF-8 file (use --file - for stdin).
    Create(CreateArgs),
    /// Read content, metadata, children, and keyword associations.
    Read {
        uri: String,
        #[arg(long)]
        no_track: bool,
        /// Omit child nodes and skip their query.
        #[arg(long)]
        no_children: bool,
    },
    /// Update content and/or metadata in one transaction; retain content history.
    Update {
        uri: String,
        /// UTF-8 content file; use - for stdin. Omit for metadata-only updates.
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Replace a unique literal UTF-8 block without fuzzy matching or escape translation.
    Patch(PatchArgs),
    /// Append UTF-8 text from a file (use --file - for stdin).
    Append {
        uri: String,
        /// UTF-8 content file; use - for stdin.
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        expect_version: Option<i64>,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
    },
    /// Update edge priority or disclosure and return the updated memory record.
    Meta {
        uri: String,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Read, replace, or edit shared node attributes.
    Attrs(AttrsArgs),
    /// Search text and/or exact top-level string attributes in the selected namespace.
    Search(SearchArgs),
    /// List existing URIs, metadata, and short content previews.
    Index(IndexArgs),
    /// Browse the memory graph without loading full content.
    Tree {
        uri: String,
        /// Maximum descendant depth; 0 returns no descendants.
        #[arg(long)]
        depth: Option<usize>,
        /// Maximum descendant nodes in the entire output, not per level.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// List child nodes through existing namespace routes.
    Children { uri: String },
    /// List recently created content versions, deduplicated by node.
    Recent {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Recall a memory weighted by priority and time since its last visit.
    Random {
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        no_track: bool,
    },
    /// List actual URI routes without loading memory content.
    Paths {
        #[arg(long)]
        all_namespaces: bool,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        node: Option<String>,
    },
    /// Discover namespaces, including those represented only by archived/configuration data.
    Namespaces,
    /// Rename the last path segment; descendants follow. Use move to change the parent.
    Rename {
        uri: String,
        /// New last segment: rename core://tree kid2 produces core://kid2, not core://tree/kid2.
        #[arg(value_name = "NEW_SEGMENT")]
        new_name: String,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Move one URI subtree to an exact destination URI.
    Move {
        source: String,
        destination: String,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Add another URI route with independent edge metadata: alias SOURCE DESTINATION.
    Alias {
        source: String,
        destination: String,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Remove a URI and protect dependent children from becoming unreachable.
    Unlink {
        uri: String,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Manage namespace trigger keywords associated with a node.
    Trigger {
        #[command(subcommand)]
        action: TriggerCommand,
    },
    /// Import UTF-8 .md/.txt files and directories in one reversible transaction.
    ImportDir(ImportDirArgs),
    /// Export the current namespace, a domain, or a URI subtree to Markdown.
    ExportDir {
        /// Optional domain or subtree root, including the root node itself.
        uri: Option<String>,
        /// Include a manifest for importing node attributes and edge metadata.
        #[arg(long)]
        with_metadata: bool,
        /// New output directory; existing files, directories, and symlinks are refused.
        #[arg(long)]
        output: PathBuf,
    },
    /// List every content version for a node.
    History { uri: String },
    /// Restore historical content; identical current content is a successful no-op.
    Restore {
        uri: String,
        version: i64,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Review effective changes: list, diff, accept, or rollback.
    Review {
        #[command(subcommand)]
        action: ReviewCommand,
    },
    /// Discover and restore nodes that have no remaining URI in any namespace.
    Orphans {
        /// Explicitly include other namespaces and imported nodes without audit provenance.
        #[arg(long, global = true)]
        all_namespaces: bool,
        #[command(subcommand)]
        action: OrphanCommand,
    },
    /// List deprecated content versions visible in the selected namespace.
    Deprecated {
        #[arg(long)]
        all_namespaces: bool,
    },
    /// Compare UTF-8 text without a database; --old-file - or --new-file - reads stdin.
    Diff {
        #[arg(long)]
        old: Option<String>,
        #[arg(long)]
        new: Option<String>,
        /// Old UTF-8 text file; use - for stdin (only one input may use stdin).
        #[arg(long)]
        old_file: Option<PathBuf>,
        /// New UTF-8 text file; use - for stdin.
        #[arg(long)]
        new_file: Option<PathBuf>,
    },
    /// Read startup memories for the selected namespace, or use set/list to configure them.
    Boot(BootArgs),
    /// Manage named startup presets (not memory access restrictions).
    Preset {
        #[command(subcommand)]
        action: PresetCommand,
    },
    /// Check database, graph, version, and search integrity (exit 1 on issues).
    Doctor,
    /// Report stale memories, crowded parents, broken parent routes, duplicate aliases, and large content.
    Diagnostic(DiagnosticArgs),
    /// Report database-wide record counts, logical bytes, and SQLite page usage.
    ///
    /// Text sizes count UTF-8 payload bytes; storage sizes use SQLite database pages.
    /// Memory-node counts exclude ROOT. Orphans have no path in any namespace;
    /// their sizes include all retained content versions.
    /// These figures describe stored data, not deletion candidates or expected space savings.
    Stats,
    /// Write a consistent backup to a new file.
    Backup {
        #[arg(long)]
        output: PathBuf,
    },
    /// Rebuild all derived search documents and FTS indexes.
    Reindex,
    /// Reclaim free SQLite pages without deleting history or audit; needs temporary disk space and a write lock.
    Vacuum,
    /// Preview permanent deletion of one deprecated version; --apply executes the plan.
    Purge {
        memory_id: i64,
        #[arg(long)]
        all_namespaces: bool,
        #[arg(long)]
        apply: bool,
    },
    /// Preview or apply access-log retention.
    Logs {
        #[command(subcommand)]
        action: LogCommand,
    },
}

// Separate argument builders keep Clap's debug stack usage bounded on Windows.
#[derive(Args)]
struct CreateArgs {
    /// Existing parent URI, for example core:// or core://project.
    #[arg(value_name = "PARENT_URI")]
    uri: String,
    /// New child path segment appended to PARENT_URI, not a display title.
    #[arg(long, value_name = "NAME")]
    title: String,
    /// UTF-8 content file; use - for stdin.
    #[arg(long)]
    file: PathBuf,
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..=999))]
    priority: i64,
    #[arg(long)]
    disclosure: Option<String>,
    /// JSON object of shared node attributes (use - for stdin).
    #[arg(long)]
    attrs_file: Option<PathBuf>,
}

#[derive(Args)]
struct PatchArgs {
    uri: String,
    #[arg(long)]
    old: Option<String>,
    #[arg(long)]
    new: Option<String>,
    /// Old UTF-8 text file; use - for stdin (only one input may use stdin).
    #[arg(long)]
    old_file: Option<PathBuf>,
    /// Replacement UTF-8 text file; use - for stdin.
    #[arg(long)]
    new_file: Option<PathBuf>,
    #[arg(long)]
    expect_version: Option<i64>,
    #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
    priority: Option<i64>,
    #[arg(long)]
    disclosure: Option<String>,
}

#[derive(Args)]
struct AttrsArgs {
    uri: String,
    /// Replace the entire JSON object; use - for stdin. Conflicts with --set/--unset.
    #[arg(long, conflicts_with_all = ["set", "unset"])]
    file: Option<PathBuf>,
    /// Set a top-level string field; repeat for multiple fields.
    #[arg(long, value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// Remove a top-level field; repeat for multiple fields.
    #[arg(long, value_name = "KEY")]
    unset: Vec<String>,
    /// Guard attribute writes against concurrent content updates on the same node.
    #[arg(long)]
    expect_version: Option<i64>,
}

#[derive(Args)]
struct IndexArgs {
    #[arg(long)]
    domain: Option<String>,
    /// Maximum number of routes; omit for all routes.
    #[arg(long)]
    limit: Option<usize>,
    /// Skip routes in stable domain/path order.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Count all matching routes; JSON becomes {total,offset,limit,has_more,results}.
    #[arg(long)]
    with_total: bool,
}

#[derive(Args)]
struct BootArgs {
    /// Read startup memories without updating access timestamps or writing access logs.
    #[arg(long)]
    no_track: bool,
    #[command(subcommand)]
    action: Option<BootCommand>,
}

#[derive(Subcommand)]
enum BootCommand {
    /// Configure startup memory URIs for the selected namespace.
    Set { uris: Vec<String> },
    /// List configured startup memory URIs for the selected namespace without reading bodies.
    List,
}

#[derive(Args)]
struct DiagnosticArgs {
    #[arg(long)]
    domain: Option<String>,
    #[arg(long, default_value_t = 30)]
    days_stale: i64,
    #[arg(long, default_value_t = 10)]
    max_children: usize,
    #[arg(long, default_value_t = 10)]
    longest_limit: usize,
    #[arg(long, default_value_t = 2048)]
    min_bytes: usize,
}

#[derive(Args)]
struct ImportDirArgs {
    directory: PathBuf,
    /// Existing parent URI receiving ordinary directory contents; required without --manifest.
    #[arg(
        long,
        required_unless_present = "manifest",
        conflicts_with = "manifest"
    )]
    into: Option<String>,
    /// Read an exported manifest, preserving its URIs in the current namespace.
    /// Parents outside the snapshot must already exist. The manifest must be inside DIRECTORY.
    #[arg(long, conflicts_with_all = ["into", "sanitize", "priority", "disclosure", "attrs_file", "source_path_attr"])]
    manifest: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    /// Replace invalid name characters with hyphens; never merge collisions.
    #[arg(long)]
    sanitize: bool,
    /// Priority for every imported directory/file edge.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..=999))]
    priority: i64,
    #[arg(long)]
    disclosure: Option<String>,
    /// Apply this JSON object to every new node; use - for stdin.
    #[arg(long)]
    attrs_file: Option<PathBuf>,
    /// Store each original input-relative path in this node attribute (includes file extension).
    #[arg(long)]
    source_path_attr: Option<String>,
}

#[derive(Args)]
struct SearchArgs {
    query: Option<String>,
    /// Exact top-level string attribute filter; repeat to require multiple fields.
    #[arg(long = "attr", value_name = "KEY=VALUE")]
    attrs: Vec<String>,
    #[arg(long)]
    domain: Option<String>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Skip matching nodes after alias deduplication.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Count all matching nodes; JSON becomes {total,offset,limit,has_more,results}.
    /// Use --limit 0 to count without loading result bodies. Counts are not recall coverage.
    #[arg(long)]
    with_total: bool,
    /// Match any tokenizer term instead of requiring all terms.
    #[arg(long = "or", requires = "query", conflicts_with = "contains")]
    or_mode: bool,
    /// Scan for the whole query as a literal substring, without tokenization or FTS ranking.
    /// With --with-total, scans all candidates in scope; never an automatic fallback.
    #[arg(long, requires = "query")]
    contains: bool,
}

#[derive(Subcommand)]
enum TriggerCommand {
    /// Add one or more trigger keywords idempotently.
    Add {
        uri: String,
        #[arg(required = true)]
        keywords: Vec<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Remove one or more trigger keywords idempotently.
    Remove {
        uri: String,
        #[arg(required = true)]
        keywords: Vec<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// Atomically add and/or remove trigger keywords in one write.
    Edit {
        uri: String,
        #[arg(long)]
        add: Vec<String>,
        #[arg(long)]
        remove: Vec<String>,
        #[arg(long)]
        expect_version: Option<i64>,
    },
    /// List trigger keywords for one URI, or all bindings in the selected namespace.
    List { uri: Option<String> },
}

#[derive(Subcommand)]
enum ReviewCommand {
    /// List changesets in the selected namespace, newest first.
    List {
        #[arg(long, value_parser = ["pending", "accepted", "rolled_back"])]
        status: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Inspect net changes for a changeset UUID or a batch name (--by-batch).
    Diff {
        id: String,
        /// Interpret ID as a batch name instead of a single changeset UUID.
        #[arg(long)]
        by_batch: bool,
    },
    /// Mark pending changes as reviewed (writes are already live before acceptance).
    Accept {
        /// Changeset UUID, or a batch name with --by-batch. Omit when using --all.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        /// Interpret ID as a batch name instead of a single changeset UUID.
        #[arg(long, conflicts_with = "all")]
        by_batch: bool,
        /// Accept every pending change in the selected namespace.
        #[arg(long)]
        all: bool,
        /// With --all, explicitly include every namespace in the database.
        #[arg(long, requires = "all")]
        all_namespaces: bool,
    },
    /// Atomically revert a changeset UUID or a batch name (--by-batch).
    Rollback {
        id: String,
        /// Interpret ID as a batch name instead of a single changeset UUID.
        #[arg(long)]
        by_batch: bool,
    },
}

#[derive(Subcommand)]
enum OrphanCommand {
    /// List pathless nodes last unlinked by the selected namespace.
    List,
    /// Read a pathless memory and its version history by memory ID.
    Show { memory_id: i64 },
    /// Restore one pathless node at a new URI, retaining all historical versions.
    Restore {
        memory_id: i64,
        destination: String,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: i64,
        #[arg(long)]
        disclosure: Option<String>,
    },
}

#[derive(Subcommand)]
enum PresetCommand {
    List,
    Show {
        name: String,
    },
    Create {
        name: String,
        /// JSON object mapping namespaces to URI arrays; use - for stdin. Defaults to an empty map.
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        activate: bool,
    },
    Update {
        name: String,
        #[arg(long)]
        new_name: Option<String>,
        /// Replace the complete namespace-to-URI map from JSON; use - for stdin.
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Duplicate {
        name: String,
        new_name: String,
    },
    Delete {
        name: String,
    },
    Activate {
        name: String,
    },
}

#[derive(Subcommand)]
enum LogCommand {
    Clear {
        /// Keep logs from the last N days; zero selects all logs.
        #[arg(long)]
        keep_days: u64,
        #[arg(long)]
        all_namespaces: bool,
        #[arg(long)]
        apply: bool,
    },
}

fn run(cli: Cli) -> Result<Value> {
    if let Command::Diff {
        old,
        new,
        old_file,
        new_file,
    } = cli.command
    {
        if old_file.as_deref() == Some(std::path::Path::new("-"))
            && new_file.as_deref() == Some(std::path::Path::new("-"))
        {
            return Err(Error::Invalid(
                "stdin cannot be consumed twice".into(),
            ));
        }
        let old_name = old_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "old".into());
        let new_name = new_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "new".into());
        let old = input(old, old_file)?;
        let new = input(new, new_file)?;
        return nocturne::text_diff::compare(&old, &new, &old_name, &new_name);
    }
    let path = cli.db.ok_or_else(|| {
        Error::Invalid(
            "missing database path; pass --db <PATH> or set NOCTURNE_DB".into(),
        )
    })?;
    let ns = &cli.namespace;
    if ns.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid namespace".into()));
    }
    if matches!(cli.command, Command::Init) {
        if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        Store::create(&path)?;
        return Ok(json!({"initialized":path}));
    }
    let store = Store::open(&path)?;
    let c = store.connection();
    match cli.command {
        Command::Recent { limit } => {
            let limit = i64::try_from(limit)
                .map_err(|_| Error::Invalid("limit is too large".into()))?;
            let mut q = c.prepare(
                "WITH ranked AS (
                    SELECT m.node_uuid,m.created_at,m.id,p.domain,p.path,e.priority,e.disclosure,
                    row_number() OVER (PARTITION BY m.node_uuid ORDER BY e.priority,length(p.path),p.domain,p.path) AS route
                    FROM memories m JOIN paths p ON p.node_uuid=m.node_uuid JOIN edges e ON e.id=p.edge_id
                    WHERE p.namespace=?1 AND m.deprecated=0)
                 SELECT node_uuid,created_at,domain,path,priority,disclosure FROM ranked
                 WHERE route=1 ORDER BY datetime(created_at) DESC,id DESC LIMIT ?2",
            )?;
            let rows = q
                .query_map(rusqlite::params![ns, limit], |r| {
                    Ok(json!({
                        "node_uuid": r.get::<_, String>(0)?,
                        "created_at": r.get::<_, String>(1)?,
                        "uri": format!("{}://{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?),
                        "priority": r.get::<_, i64>(4)?,
                        "disclosure": r.get::<_, Option<String>>(5)?
                    }))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(json!(rows))
        }
        Command::Trigger { action } => match action {
            TriggerCommand::List { uri } => {
                let node = uri
                    .as_deref()
                    .map(|u| graph::resolve(c, ns, u))
                    .transpose()?;
                nocturne::glossary::list(c, ns, node.as_deref())
            }
            TriggerCommand::Add {
                uri,
                keywords,
                expect_version,
            } => store.write(ns, cli.batch.as_deref(), |c| {
                graph::check_version(c, ns, &uri, expect_version)?;
                nocturne::glossary::edit(c, ns, &uri, &keywords, &[])
            }),
            TriggerCommand::Remove {
                uri,
                keywords,
                expect_version,
            } => store.write(ns, cli.batch.as_deref(), |c| {
                graph::check_version(c, ns, &uri, expect_version)?;
                nocturne::glossary::edit(c, ns, &uri, &[], &keywords)
            }),
            TriggerCommand::Edit {
                uri,
                add,
                remove,
                expect_version,
            } => {
                if add.is_empty() && remove.is_empty() {
                    return Err(Error::Invalid(
                        "trigger edit requires --add or --remove".into(),
                    ));
                }
                store.write(ns, cli.batch.as_deref(), |c| {
                    graph::check_version(c, ns, &uri, expect_version)?;
                    nocturne::glossary::edit(c, ns, &uri, &add, &remove)
                })
            }
        },
        Command::Alias {
            source,
            destination,
            priority,
            disclosure,
            expect_version,
        } => store.write(ns, cli.batch.as_deref(), |c| {
            if expect_version.is_some() {
                graph::check_version(c, ns, &source, expect_version)?;
            }
            graph::alias_with_meta(
                c,
                ns,
                &source,
                &destination,
                priority,
                disclosure.as_deref(),
            )
        }),
        Command::Rename {
            uri,
            new_name,
            expect_version,
        } => store.write(ns, cli.batch.as_deref(), |c| {
            if expect_version.is_some() {
                graph::check_version(c, ns, &uri, expect_version)?;
            }
            nocturne::paths::rename(c, ns, &uri, &new_name)
        }),
        Command::Move {
            source,
            destination,
            expect_version,
        } => store.write(ns, cli.batch.as_deref(), |c| {
            if expect_version.is_some() {
                graph::check_version(c, ns, &source, expect_version)?;
            }
            nocturne::paths::move_path(c, ns, &source, &destination)
        }),
        Command::Orphans {
            all_namespaces,
            action,
        } => match action {
            OrphanCommand::List => {
                nocturne::orphans::list(c, ns, all_namespaces)
            }
            OrphanCommand::Show { memory_id } => {
                nocturne::orphans::show(c, ns, memory_id, all_namespaces)
            }
            OrphanCommand::Restore {
                memory_id,
                destination,
                priority,
                disclosure,
            } => store.write(ns, cli.batch.as_deref(), |c| {
                nocturne::orphans::restore(
                    c,
                    ns,
                    memory_id,
                    &destination,
                    priority,
                    disclosure.as_deref(),
                    all_namespaces,
                )
            }),
        },
        Command::Stats => nocturne::stats::stats(c),
        Command::Unlink {
            uri,
            expect_version,
        } => store.write(ns, cli.batch.as_deref(), |c| {
            if expect_version.is_some() {
                graph::check_version(c, ns, &uri, expect_version)?;
            }
            graph::unlink(c, ns, &uri)
        }),
        Command::Restore {
            uri,
            version,
            expect_version,
        } => store.write(ns, cli.batch.as_deref(), |c| {
            graph::restore(c, ns, &uri, version, expect_version)
        }),
        Command::Meta {
            uri,
            priority,
            disclosure,
            expect_version,
        } => {
            if priority.is_none() && disclosure.is_none() {
                return Err(Error::Invalid(
                    "meta requires --priority or --disclosure".into(),
                ));
            }
            store.write(ns, cli.batch.as_deref(), |c| {
                graph::edit(
                    c,
                    ns,
                    &uri,
                    None,
                    priority,
                    disclosure.as_deref(),
                    expect_version,
                )
            })
        }
        Command::Patch(PatchArgs {
            uri,
            old,
            new,
            old_file,
            new_file,
            expect_version,
            priority,
            disclosure,
        }) => {
            if old_file.as_deref() == Some(std::path::Path::new("-"))
                && new_file.as_deref() == Some(std::path::Path::new("-"))
            {
                return Err(Error::Invalid(
                    "stdin cannot be consumed twice".into(),
                ));
            }
            let old = input(old, old_file)?;
            let new = input(new, new_file)?;
            store.write(ns, cli.batch.as_deref(), |c| {
                graph::check_version(c, ns, &uri, expect_version)?;
                let v = graph::read(c, ns, &uri, false)?;
                let content = v["content"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("missing content".into()))?;
                let text = nocturne::patch::apply(content, &old, &new)?;
                graph::edit(
                    c,
                    ns,
                    &uri,
                    Some(&text),
                    priority,
                    disclosure.as_deref(),
                    expect_version,
                )
            })
        }
        Command::Append {
            uri,
            file,
            expect_version,
            priority,
            disclosure,
        } => {
            let extra = input(None, Some(file))?;
            if extra.is_empty() {
                return Err(Error::Invalid(
                    "append requires non-empty text".into(),
                ));
            }
            store.write(ns, cli.batch.as_deref(), |c| {
                graph::check_version(c, ns, &uri, expect_version)?;
                let v = graph::read(c, ns, &uri, false)?;
                let content = v["content"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("missing content".into()))?;
                graph::edit(
                    c,
                    ns,
                    &uri,
                    Some(&format!("{content}{extra}")),
                    priority,
                    disclosure.as_deref(),
                    expect_version,
                )
            })
        }
        Command::Read {
            uri,
            no_track,
            no_children,
        } => graph::read_with_children(c, ns, &uri, !no_track, !no_children),
        Command::Children { uri } => views::children(c, ns, &uri),
        Command::Tree { uri, depth, limit } => {
            views::tree_limited(c, ns, &uri, depth, limit)
        }
        Command::Index(IndexArgs {
            domain,
            limit,
            offset,
            with_total,
        }) => {
            if !with_total {
                views::index_page(c, ns, domain.as_deref(), limit, offset)
            } else {
                views::index_page_with_info(
                    c,
                    ns,
                    domain.as_deref(),
                    limit,
                    offset,
                    true,
                )
            }
        }
        Command::History { uri } => graph::history(c, ns, &uri),
        Command::Boot(BootArgs { no_track, action }) => match action {
            None => views::boot_with_tracking(c, ns, !no_track),
            Some(_) if no_track => Err(Error::Invalid(
                "--no-track applies only when loading boot memories".into(),
            )),
            Some(BootCommand::List) => views::boot_uris(c, ns),
            Some(BootCommand::Set { uris }) => {
                store.write(ns, cli.batch.as_deref(), |c| {
                    views::set_boot(c, ns, &uris)
                })
            }
        },
        Command::Create(CreateArgs {
            uri,
            title,
            file,
            priority,
            disclosure,
            attrs_file,
        }) => {
            if file == std::path::Path::new("-")
                && attrs_file.as_deref() == Some(std::path::Path::new("-"))
            {
                return Err(Error::Invalid(
                    "stdin cannot be consumed twice".into(),
                ));
            }
            let attrs = attrs_file.map(attributes_input).transpose()?;
            let text = input(None, Some(file))?;
            store.write(ns, cli.batch.as_deref(), |c| {
                let mut result = graph::create(
                    c,
                    ns,
                    &uri,
                    &title,
                    &text,
                    priority,
                    disclosure.as_deref(),
                )?;
                if let Some(attrs) = &attrs {
                    let created_uri = result["uri"]
                        .as_str()
                        .ok_or_else(|| Error::Invalid("missing URI".into()))?
                        .to_owned();
                    let stored =
                        nocturne::attrs::set(c, ns, &created_uri, attrs)?;
                    result["attrs"] = attrs.clone();
                    if let Some(warnings) = stored.get("warnings") {
                        result["warnings"] = warnings.clone();
                    }
                }
                Ok(result)
            })
        }
        Command::Attrs(AttrsArgs {
            uri,
            file,
            set,
            unset,
            expect_version,
        }) => match file {
            Some(file) => {
                let attrs = attributes_input(file)?;
                store.write(ns, cli.batch.as_deref(), |c| {
                    if expect_version.is_some() {
                        graph::check_version(c, ns, &uri, expect_version)?;
                    }
                    nocturne::attrs::set(c, ns, &uri, &attrs)
                })
            }
            None if !set.is_empty() || !unset.is_empty() => {
                let set = set
                    .iter()
                    .map(|value| nocturne::attrs::parse_assignment(value))
                    .collect::<Result<Vec<_>>>()?;
                store.write(ns, cli.batch.as_deref(), |c| {
                    if expect_version.is_some() {
                        graph::check_version(c, ns, &uri, expect_version)?;
                    }
                    nocturne::attrs::edit(c, ns, &uri, &set, &unset)
                })
            }
            None => {
                if expect_version.is_some() {
                    return Err(Error::Invalid(
                        "--expect-version requires --file, --set, or --unset"
                            .into(),
                    ));
                }
                nocturne::attrs::get(c, ns, &uri)
            }
        },
        Command::ImportDir(ImportDirArgs {
            directory,
            into,
            manifest,
            dry_run,
            sanitize,
            priority,
            disclosure,
            attrs_file,
            source_path_attr,
        }) => {
            if let Some(manifest) = manifest {
                let plan =
                    nocturne::manifest::plan(c, ns, &directory, &manifest)?;
                if dry_run {
                    return Ok(plan.preview());
                }
                return store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::manifest::apply(c, ns, &plan)
                });
            }
            let into = into.ok_or_else(|| {
                Error::Invalid("pass --into or --manifest".into())
            })?;
            let attrs = attrs_file.map(attributes_input).transpose()?;
            let mut plan = nocturne::import_dir::plan_with_sanitize(
                c, ns, &directory, &into, sanitize,
            )?;
            if let Some(key) = source_path_attr {
                plan = plan.with_source_path_attr(&key, attrs.as_ref())?;
            }
            if dry_run {
                let mut preview = plan.preview();
                preview["priority"] = json!(priority);
                preview["disclosure"] = json!(disclosure);
                let warnings = plan.warnings(attrs.as_ref());
                if !warnings.is_empty() {
                    preview["warnings"] = json!(warnings);
                }
                return Ok(preview);
            }
            store.write(ns, cli.batch.as_deref(), |c| {
                nocturne::import_dir::apply_with_metadata(
                    c,
                    ns,
                    &plan,
                    attrs.as_ref(),
                    priority,
                    disclosure.as_deref(),
                )
            })
        }
        Command::Update {
            uri,
            file,
            priority,
            disclosure,
            expect_version,
        } => {
            let text = file.map(|f| input(None, Some(f))).transpose()?;
            store.write(ns, cli.batch.as_deref(), |c| {
                graph::edit(
                    c,
                    ns,
                    &uri,
                    text.as_deref(),
                    priority,
                    disclosure.as_deref(),
                    expect_version,
                )
            })
        }
        Command::Search(SearchArgs {
            query,
            attrs,
            domain,
            limit,
            offset,
            with_total,
            or_mode,
            contains,
        }) => {
            let filters = attrs
                .iter()
                .map(|value| nocturne::attrs::parse_assignment(value))
                .collect::<Result<Vec<_>>>()?;
            use nocturne::search::MatchMode;
            let mode = if contains {
                MatchMode::Contains
            } else if or_mode {
                MatchMode::Any
            } else {
                MatchMode::All
            };
            if cli.json && !with_total {
                nocturne::search::search_page(
                    c,
                    ns,
                    query.as_deref(),
                    domain.as_deref(),
                    (limit, offset),
                    mode,
                    &filters,
                )
            } else {
                nocturne::search::search_page_with_info(
                    c,
                    ns,
                    query.as_deref(),
                    domain.as_deref(),
                    nocturne::search::PageRequest {
                        limit,
                        offset,
                        with_total,
                    },
                    mode,
                    &filters,
                )
            }
        }
        Command::Review { action } => match action {
            ReviewCommand::List {
                status,
                limit,
                offset,
            } => {
                changes::list_filtered(c, ns, status.as_deref(), limit, offset)
            }
            ReviewCommand::Diff { id, by_batch } if !cli.json => {
                Ok(json!({"unified":changes::diff_text(c, ns, &id, by_batch)?}))
            }
            ReviewCommand::Diff { id, by_batch } => {
                changes::diff(c, ns, &id, by_batch)
            }
            ReviewCommand::Accept {
                id,
                by_batch,
                all,
                all_namespaces,
            } => {
                if all {
                    changes::accept_all(c, ns, all_namespaces)
                } else {
                    let id = id.ok_or_else(|| {
                        Error::Invalid("review accept requires ID".into())
                    })?;
                    changes::accept(c, ns, &id, by_batch)
                }
            }
            ReviewCommand::Rollback { id, by_batch } => {
                changes::rollback(c, ns, &id, by_batch)
            }
        },
        Command::Preset { action } => match action {
            PresetCommand::List => nocturne::presets::list(c),
            PresetCommand::Show { name } => nocturne::presets::show(c, &name),
            PresetCommand::Create {
                name,
                file,
                activate,
            } => {
                let uris = file
                    .map(|f| json_input(f, "preset"))
                    .transpose()?
                    .unwrap_or_else(|| json!({}));
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::presets::create(c, &name, &uris, activate)
                })
            }
            PresetCommand::Update {
                name,
                new_name,
                file,
            } => {
                let uris = file.map(|f| json_input(f, "preset")).transpose()?;
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::presets::update(
                        c,
                        &name,
                        new_name.as_deref(),
                        uris.as_ref(),
                    )
                })
            }
            PresetCommand::Duplicate { name, new_name } => {
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::presets::duplicate(c, &name, &new_name)
                })
            }
            PresetCommand::Delete { name } => {
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::presets::delete(c, &name)
                })
            }
            PresetCommand::Activate { name } => {
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::presets::activate(c, &name)
                })
            }
        },
        Command::Namespaces => nocturne::catalog::namespaces(c),
        Command::Paths {
            all_namespaces,
            domain,
            node,
        } => nocturne::catalog::paths(
            c,
            ns,
            all_namespaces,
            domain.as_deref(),
            node.as_deref(),
        ),
        Command::Diagnostic(DiagnosticArgs {
            domain,
            days_stale,
            max_children,
            longest_limit,
            min_bytes,
        }) => nocturne::reports::diagnostic(
            c,
            ns,
            domain.as_deref(),
            days_stale,
            max_children,
            longest_limit,
            min_bytes,
        ),
        Command::Random { domain, no_track } => {
            nocturne::reports::random(c, ns, domain.as_deref(), !no_track)
        }
        Command::Deprecated { all_namespaces } => {
            nocturne::retention::deprecated(c, ns, all_namespaces)
        }
        Command::Purge {
            memory_id,
            all_namespaces,
            apply,
        } => {
            nocturne::retention::purge(c, ns, memory_id, all_namespaces, apply)
        }
        Command::Logs {
            action:
                LogCommand::Clear {
                    keep_days,
                    all_namespaces,
                    apply,
                },
        } => nocturne::retention::clear_logs(
            c,
            ns,
            keep_days,
            all_namespaces,
            apply,
        ),
        Command::ExportDir {
            uri,
            with_metadata,
            output,
        } => nocturne::export_dir::export_with_options(
            c,
            ns,
            &output,
            uri.as_deref(),
            with_metadata,
        ),
        Command::Backup { output } => nocturne::maintenance::backup(c, &output),
        Command::Doctor => nocturne::maintenance::doctor(c),
        Command::Vacuum => nocturne::maintenance::vacuum(c),
        Command::Reindex => {
            c.execute_batch("BEGIN IMMEDIATE")?;
            let _guard = nocturne::store::TransactionGuard::new(c);
            nocturne::search::rebuild(c, None)?;
            c.execute_batch("COMMIT")?;
            Ok(json!({"reindexed":true}))
        }
        Command::Init | Command::Diff { .. } => unreachable!(),
    }
}

fn input(text: Option<String>, file: Option<PathBuf>) -> Result<String> {
    if text.is_some() == file.is_some() {
        return Err(Error::Invalid(
            "provide exactly one inline text or file input".into(),
        ));
    }
    if let Some(t) = text {
        return Ok(t);
    }
    let file = file.ok_or_else(|| Error::Invalid("missing file".into()))?;
    if file == std::path::Path::new("-") {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|error| input_error("stdin", error))?;
        Ok(s)
    } else {
        std::fs::read_to_string(&file)
            .map_err(|error| input_error(&file.display().to_string(), error))
    }
}

fn input_error(source: &str, error: std::io::Error) -> Error {
    let message = format!("{source}: {error}");
    if error.kind() == std::io::ErrorKind::InvalidData {
        Error::Invalid(message)
    } else {
        Error::Storage(message)
    }
}

fn json_input(file: PathBuf, kind: &str) -> Result<Value> {
    let label = file.display().to_string();
    serde_json::from_str(&input(None, Some(file))?).map_err(|error| {
        Error::Invalid(format!("invalid {kind} in {label}: {error}"))
    })
}

fn attributes_input(file: PathBuf) -> Result<Value> {
    let label = file.display().to_string();
    let value = json_input(file, "attributes")?;
    if !value.is_object() {
        return Err(Error::Invalid(format!(
            "attributes in {label} must be a JSON object"
        )));
    }
    Ok(value)
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp
                    | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{e}");
                return;
            }
            eprintln!(
                "{}",
                json!({"error":{"code":"invalid","message":e.to_string()}})
            );
            std::process::exit(2)
        }
    };
    let json_mode = cli.json;
    let search_mode = matches!(cli.command, Command::Search(_));
    let stats_mode = matches!(cli.command, Command::Stats);
    let diff_mode = matches!(cli.command, Command::Diff { .. })
        || matches!(
            &cli.command,
            Command::Review {
                action: ReviewCommand::Diff { .. }
            }
        );
    let doctor_mode = matches!(cli.command, Command::Doctor);
    let boot_mode =
        matches!(&cli.command, Command::Boot(BootArgs { action: None, .. }));
    match run(cli) {
        Ok(mut v) => {
            // Statistics definitions belong in help; preserve the JSON contract.
            if stats_mode
                && !json_mode
                && let Some(obj) = v.as_object_mut()
            {
                obj.remove("semantics");
            }
            let output = if json_mode {
                format!("{v}\n")
            } else if diff_mode {
                v["unified"].as_str().unwrap_or_default().to_owned()
            } else if search_mode {
                search_text(&v)
            } else {
                format!("{}\n", text_value(&v, 0, true))
            };
            use std::io::Write;
            if let Err(error) =
                std::io::stdout().lock().write_all(output.as_bytes())
            {
                if error.kind() == std::io::ErrorKind::BrokenPipe {
                    return;
                }
                eprintln!(
                    "{}",
                    json!({"error":{"code":"storage","message":error.to_string()}})
                );
                std::process::exit(5);
            }
            if v.get("valid") == Some(&Value::Bool(false)) {
                let empty = Vec::new();
                let issues = v["issues"].as_array().unwrap_or(&empty);
                let (code_str, exit_code) =
                    if issues.iter().any(|i| i["kind"] == "storage") {
                        ("storage", 5)
                    } else if issues.iter().any(|i| i["kind"] == "conflict") {
                        ("conflict", 4)
                    } else {
                        ("invalid", 2)
                    };
                let issue_count = issues.len();
                let problem_source_count = v["problem_source_count"]
                    .as_u64()
                    .map(|n| n as usize)
                    .unwrap_or_else(|| {
                        issues
                            .iter()
                            .filter_map(|i| i["source"].as_str())
                            .collect::<BTreeSet<_>>()
                            .len()
                    });
                let samples: Vec<Value> =
                    issues.iter().take(3).cloned().collect();
                eprintln!(
                    "{}",
                    json!({
                        "error": {
                            "code": code_str,
                            "issue_count": issue_count,
                            "problem_source_count": problem_source_count,
                            "samples": samples,
                            "message": format!(
                                "import validation failed: {issue_count} issue(s) in {problem_source_count} source(s)"
                            )
                        }
                    })
                );
                std::process::exit(exit_code);
            }
            if doctor_mode && v.get("ok") == Some(&Value::Bool(false)) {
                let count =
                    v["issues"].as_array().map_or(1, std::vec::Vec::len);
                eprintln!(
                    "{}",
                    json!({
                        "error": {
                            "code": "integrity",
                            "issue_count": count,
                            "message": format!("doctor detected {count} integrity issue(s)")
                        }
                    })
                );
                std::process::exit(1);
            }
            if boot_mode
                && let Some(failed) =
                    v["failed"].as_array().filter(|f| !f.is_empty())
            {
                let count = failed.len();
                eprintln!(
                    "{}",
                    json!({
                        "error": {
                            "code": "not_found",
                            "failed_count": count,
                            "message": format!("boot failed to load {count} configured URI(s)")
                        }
                    })
                );
                std::process::exit(3);
            }
        }
        Err(e) => {
            let (mut detail, exit) = match &e {
                Error::ImportValidation {
                    code,
                    issue_count,
                    problem_source_count,
                    samples,
                    ..
                } => (
                    json!({"code":match code { 5 => "storage", 4 => "conflict", _ => "invalid" },
                        "issue_count":issue_count, "problem_source_count":problem_source_count, "samples":samples}),
                    *code,
                ),
                Error::Invalid(_) => (json!({"code":"invalid"}), 2),
                Error::NotFound(_) => (json!({"code":"not_found"}), 3),
                Error::Conflict(_) => (json!({"code":"conflict"}), 4),
                Error::RollbackConflict {
                    blocking_uris,
                    related_changesets,
                    foreign_references,
                    ..
                } => (
                    json!({"code":"conflict","reason":"rollback_conflict",
                        "blocking_uris":blocking_uris,"related_changesets":related_changesets.iter()
                            .map(|change| json!({"id":change.id,"batch":change.batch})).collect::<Vec<_>>(),
                        "foreign_references":foreign_references}),
                    4,
                ),
                Error::VersionConflict {
                    uri,
                    namespace,
                    expected,
                    current,
                } => (
                    json!({"code":"conflict","reason":"version_mismatch","uri":uri,
                        "namespace":namespace,"expected_version":expected,"current_version":current}),
                    4,
                ),
                Error::PatchNoMatch => (
                    json!({"code":"conflict","reason":"patch_no_match","matches":0}),
                    4,
                ),
                Error::PatchAmbiguous { matches } => (
                    json!({"code":"conflict","reason":"patch_ambiguous","matches_at_least":matches}),
                    4,
                ),
                Error::Storage(_) => (json!({"code":"storage"}), 5),
            };
            detail["message"] = json!(e.to_string());
            eprintln!("{}", json!({"error":detail}));
            std::process::exit(exit)
        }
    }
}

fn search_text(page: &Value) -> String {
    let offset = page["offset"].as_u64().unwrap_or(0);
    let count = page["results"].as_array().map_or(0, |r| r.len() as u64);
    let total = page["total"].as_u64();
    let more = page["has_more"] == true;
    if total == Some(0) {
        return "No matching nodes.\n".to_owned();
    }
    if page["limit"] == 0 {
        return match total {
            Some(total) => format!(
                "Total matching nodes: {total}; no results requested (--limit 0).\n"
            ),
            None => "Matching nodes exist; no results requested (--limit 0).\n"
                .to_owned(),
        };
    }
    let summary = if count == 0 {
        format!(
            "No results at offset {offset}; matching nodes exist before this offset."
        )
    } else {
        format!("Showing results {}–{}.", offset + 1, offset + count)
    };
    let mut output = summary;
    if let Some(total) = total {
        output.push_str(&format!(" Total matching nodes: {total}."));
    }
    if more {
        output.push_str(&format!(
            " More results: use --offset {}.",
            offset + count
        ));
    } else {
        output.push_str(" No more results.");
    }
    output.push('\n');
    if count > 0 {
        output.push('\n');
        output.push_str(&text_value(&page["results"], 0, true));
        output.push('\n');
    }
    output
}

fn text_value(value: &Value, indent: usize, record: bool) -> String {
    let prefix = " ".repeat(indent);
    match value {
        Value::Object(fields) if !fields.is_empty() => {
            // Only result records have CLI semantics; nested objects may be arbitrary user data.
            let memory = record
                && fields.contains_key("uri")
                && fields.contains_key("version")
                && fields.contains_key("content");
            let review = record
                && ["id", "status", "created_at", "namespace"]
                    .iter()
                    .all(|key| fields.contains_key(*key));
            let priority: &[&str] = if memory {
                &[
                    "uri",
                    "version",
                    "node_uuid",
                    "priority",
                    "disclosure",
                    "content",
                    "attrs",
                    "associations",
                    "children",
                ]
            } else if record
                && ["table", "before", "after"]
                    .iter()
                    .all(|key| fields.contains_key(*key))
            {
                &["table", "before", "after"]
            } else if review {
                &["id", "batch", "status", "created_at", "namespace"]
            } else {
                &[]
            };
            let mut entries = fields.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(name, _)| {
                priority
                    .iter()
                    .position(|key| *key == name.as_str())
                    .unwrap_or(priority.len())
            });
            entries
                .into_iter()
                .map(|(name, value)| {
                    if review
                        && name == "namespace"
                        && value.as_str() == Some("")
                    {
                        return format!("{prefix}namespace: \"\"");
                    }
                    // The children command's context repeats this memory's URI; flatten only its display.
                    let value = if memory
                        && name == "children"
                        && value.get("context") == fields.get("uri")
                        && value.get("children").is_some_and(Value::is_array)
                    {
                        &value["children"]
                    } else {
                        value
                    };
                    if value.is_object()
                        || value.is_array()
                        || value.as_str().is_some_and(|s| s.contains('\n'))
                    {
                        format!(
                            "{prefix}{name}:\n{}",
                            text_value(value, indent + 2, false)
                        )
                    } else {
                        format!(
                            "{prefix}{name}: {}",
                            text_value(value, 0, false)
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Value::Array(items) if !items.is_empty() => items
            .iter()
            .map(|item| text_value(item, indent, record))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) | Value::Array(_) | Value::Null => {
            format!("{prefix}(none)")
        }
        Value::String(text) => text
            .split('\n')
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => format!("{prefix}{value}"),
    }
}
