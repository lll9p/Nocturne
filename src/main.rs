use clap::{Parser, Subcommand};
use nocturne::{
    changes,
    error::{Error, Result},
    graph,
    store::Store,
    views,
};
use serde_json::{Value, json};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    version = env!("CARGO_PKG_VERSION"),
    about = "Local, versioned memory with SQLite search and reversible writes"
)]
struct Cli {
    /// Database file; init creates it, other commands require an existing file.
    #[arg(long, env = "NOCTURNE_DB", global = true)]
    db: Option<PathBuf>,
    /// Select the URI namespace; the default is the empty namespace.
    #[arg(long, env = "NOCTURNE_NAMESPACE", default_value = "", global = true)]
    namespace: String,
    /// Print successful results as JSON; errors are always JSON.
    #[arg(long, global = true)]
    json: bool,
    /// Group writes for review; place before the subcommand. Not a write transaction.
    #[arg(long, id = "write_batch", value_name = "BATCH_ID")]
    batch: Option<String>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Create a new database; refuse existing files.
    Init,
    /// Create a child memory from a UTF-8 file (use --file - for stdin).
    Create {
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
    },
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
    /// Replace a unique literal or start...end block without fuzzy matching.
    Patch {
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
    },
    /// Append UTF-8 text from a file or stdin.
    Append {
        uri: String,
        /// UTF-8 content file; use - for stdin.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Read UTF-8 text from stdin instead of --file.
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        expect_version: Option<i64>,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
    },
    /// Update shared edge priority or disclosure.
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
    Index {
        #[arg(long)]
        domain: Option<String>,
        /// Maximum number of routes; omit for all routes.
        #[arg(long)]
        limit: Option<usize>,
        /// Skip routes in stable domain/path order.
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
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
    },
    /// Move one URI subtree to an exact destination URI.
    Move { source: String, destination: String },
    /// Add another URI for a node: alias add DESTINATION SOURCE.
    Alias {
        #[arg(value_parser = ["add"])]
        action: String,
        destination: String,
        source: String,
        #[arg(long, value_parser = clap::value_parser!(i64).range(0..=999))]
        priority: Option<i64>,
        #[arg(long)]
        disclosure: Option<String>,
    },
    /// Remove a URI and protect dependent children from becoming unreachable.
    Unlink { uri: String },
    /// Manage keywords: add/remove URI WORD..., edit URI --add WORD --remove WORD, or list [URI].
    Trigger {
        #[arg(value_parser = ["add", "remove", "edit", "list"])]
        action: String,
        uri: Option<String>,
        keywords: Vec<String>,
        #[arg(long)]
        add: Vec<String>,
        #[arg(long)]
        remove: Vec<String>,
    },
    /// Import UTF-8 .md/.txt files and directories in one reversible transaction.
    ImportDir(ImportDirArgs),
    /// Export the selected namespace to a new Markdown directory (not a database backup).
    ExportDir {
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
        #[arg(value_parser = ["list", "diff", "accept", "rollback"])]
        action: String,
        /// Changeset ID, or a batch name with --batch.
        id: Option<String>,
        /// Treat ID as a batch name instead of a single changeset ID.
        #[arg(long, id = "review_batch")]
        batch: bool,
        /// Accept every pending change in the selected namespace.
        #[arg(long)]
        all: bool,
        /// With accept --all, explicitly include every namespace.
        #[arg(long)]
        all_namespaces: bool,
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
    /// Read configured startup memories, or use set/list to configure them.
    Boot {
        #[arg(value_parser = ["set", "list"])]
        action: Option<String>,
        uris: Vec<String>,
    },
    /// Manage named startup presets (not memory access restrictions).
    Preset {
        #[command(subcommand)]
        action: PresetCommand,
    },
    /// Check database, graph, version, and search integrity (exit 1 on issues).
    Doctor,
    /// Report stale memories, crowded parents, duplicate aliases, and large content.
    Diagnostic {
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
    },
    /// Report database-wide record counts, logical bytes, and SQLite page usage.
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
#[derive(clap::Args)]
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
}
#[derive(clap::Args)]
struct ImportDirArgs {
    directory: PathBuf,
    /// Existing parent URI receiving the directory's contents.
    #[arg(long)]
    into: String,
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
}
#[derive(clap::Args)]
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
    /// Match any tokenizer term instead of requiring all terms.
    #[arg(long = "or", requires = "query", conflicts_with = "contains")]
    or_mode: bool,
    /// Match the whole query as a literal substring, without tokenization or FTS ranking.
    #[arg(long, requires = "query")]
    contains: bool,
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
        let old = input(old, old_file, false)?;
        let new = input(new, new_file, false)?;
        return nocturne::text_diff::compare(&old, &new, &old_name, &new_name);
    }
    let path = match cli.db {
        Some(path) => path,
        None => directories::ProjectDirs::from("", "nocturne", "nocturne")
            .ok_or_else(|| {
                Error::Invalid(
                    "cannot determine data directory; pass --db".into(),
                )
            })?
            .data_dir()
            .join("memory.db"),
    };
    let ns = &cli.namespace;
    if ns.chars().any(|c| c.is_control()) {
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
            let rows = q.query_map(rusqlite::params![ns, limit], |r| Ok(json!({
                "node_uuid": r.get::<_, String>(0)?, "created_at": r.get::<_, String>(1)?,
                "uri": format!("{}://{}", r.get::<_, String>(2)?, r.get::<_, String>(3)?),
                "priority": r.get::<_, i64>(4)?, "disclosure": r.get::<_, Option<String>>(5)?
            })))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(json!(rows))
        }
        Command::Trigger {
            action,
            uri,
            keywords,
            add,
            remove,
        } => {
            if action == "list" {
                if !keywords.is_empty() || !add.is_empty() || !remove.is_empty()
                {
                    return Err(Error::Invalid(
                        "trigger list does not edit keywords".into(),
                    ));
                }
                let node = uri
                    .as_deref()
                    .map(|u| graph::resolve(c, ns, u))
                    .transpose()?;
                return nocturne::glossary::list(c, ns, node.as_deref());
            }
            let uri = uri
                .ok_or_else(|| Error::Invalid("trigger requires URI".into()))?;
            let (add, remove) = match action.as_str() {
                "add" | "remove" if !keywords.is_empty() && add.is_empty() && remove.is_empty() => {
                    if keywords.len() == 1 {
                        return store.write(ns, cli.batch.as_deref(), |c| {
                            let node = graph::resolve(c, ns, &uri)?;
                            if action == "add" {
                                nocturne::glossary::add(c, ns, &node, &keywords[0])
                            } else {
                                nocturne::glossary::remove(c, ns, &node, &keywords[0])
                            }
                        });
                    }
                    if action == "add" {
                        (keywords, vec![])
                    } else {
                        (vec![], keywords)
                    }
                }
                "edit" if keywords.is_empty() && (!add.is_empty() || !remove.is_empty()) => {
                    (add, remove)
                }
                _ => {
                    return Err(Error::Invalid(
                        "expected trigger add/remove URI WORD... or edit URI --add/--remove WORD"
                            .into(),
                    ))
                }
            };
            store.write(ns, cli.batch.as_deref(), |c| {
                nocturne::glossary::edit(c, ns, &uri, &add, &remove)
            })
        }
        Command::Alias {
            action,
            destination,
            source,
            priority,
            disclosure,
        } => {
            if action != "add" {
                return Err(Error::Invalid("expected alias add".into()));
            }
            store.write(ns, cli.batch.as_deref(), |c| {
                graph::alias_with_meta(
                    c,
                    ns,
                    &destination,
                    &source,
                    priority,
                    disclosure.as_deref(),
                )
            })
        }
        Command::Rename { uri, new_name } => {
            store.write(ns, cli.batch.as_deref(), |c| {
                nocturne::paths::rename(c, ns, &uri, &new_name)
            })
        }
        Command::Move {
            source,
            destination,
        } => store.write(ns, cli.batch.as_deref(), |c| {
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
        Command::Unlink { uri } => store
            .write(ns, cli.batch.as_deref(), |c| graph::unlink(c, ns, &uri)),
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
                )?;
                Ok(json!({"updated":uri}))
            })
        }
        Command::Patch {
            uri,
            old,
            new,
            old_file,
            new_file,
            expect_version,
            priority,
            disclosure,
        } => {
            if old_file.as_deref() == Some(std::path::Path::new("-"))
                && new_file.as_deref() == Some(std::path::Path::new("-"))
            {
                return Err(Error::Invalid(
                    "stdin cannot be consumed twice".into(),
                ));
            }
            let old = input(old, old_file, false)?;
            let new = input(new, new_file, false)?;
            store.write(ns, cli.batch.as_deref(), |c| {
                let v = graph::read(c, ns, &uri, false)?;
                let text = nocturne::patch::apply(
                    v["content"].as_str().unwrap(),
                    &old,
                    &new,
                )?;
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
            stdin,
            expect_version,
            priority,
            disclosure,
        } => {
            let extra = input(None, file, stdin)?;
            if extra.is_empty() {
                return Err(Error::Invalid(
                    "append requires non-empty text".into(),
                ));
            }
            store.write(ns, cli.batch.as_deref(), |c| {
                let v = graph::read(c, ns, &uri, false)?;
                graph::edit(
                    c,
                    ns,
                    &uri,
                    Some(&format!(
                        "{}{}",
                        v["content"].as_str().unwrap(),
                        extra
                    )),
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
        Command::Index {
            domain,
            limit,
            offset,
        } => views::index_page(c, ns, domain.as_deref(), limit, offset),
        Command::History { uri } => graph::history(c, ns, &uri),
        Command::Boot { action, uris } => match action.as_deref() {
            None => views::boot(c, ns),
            Some("list") => views::boot_uris(c, ns),
            Some("set") => store.write(ns, cli.batch.as_deref(), |c| {
                views::set_boot(c, ns, &uris)
            }),
            _ => Err(Error::Invalid("expected boot set/list".into())),
        },
        Command::Create {
            uri,
            title,
            file,
            priority,
            disclosure,
            attrs_file,
        } => {
            if file == std::path::Path::new("-")
                && attrs_file.as_deref() == Some(std::path::Path::new("-"))
            {
                return Err(Error::Invalid(
                    "stdin cannot be consumed twice".into(),
                ));
            }
            let attrs = attrs_file.map(attributes_input).transpose()?;
            let text = input(None, Some(file), false)?;
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
                    let stored = nocturne::attrs::set(
                        c,
                        ns,
                        result["uri"].as_str().unwrap(),
                        attrs,
                    )?;
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
        }) => match file {
            Some(file) => {
                let attrs = attributes_input(file)?;
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::attrs::set(c, ns, &uri, &attrs)
                })
            }
            None if !set.is_empty() || !unset.is_empty() => {
                let set = set
                    .iter()
                    .map(|value| nocturne::attrs::parse_assignment(value))
                    .collect::<Result<Vec<_>>>()?;
                store.write(ns, cli.batch.as_deref(), |c| {
                    nocturne::attrs::edit(c, ns, &uri, &set, &unset)
                })
            }
            None => nocturne::attrs::get(c, ns, &uri),
        },
        Command::ImportDir(ImportDirArgs {
            directory,
            into,
            dry_run,
            sanitize,
            priority,
            disclosure,
            attrs_file,
        }) => {
            let attrs = attrs_file.map(attributes_input).transpose()?;
            let plan = nocturne::import_dir::plan_with_sanitize(
                c, ns, &directory, &into, sanitize,
            )?;
            if dry_run {
                let mut preview = plan.preview();
                preview["priority"] = json!(priority);
                preview["disclosure"] = json!(disclosure);
                if let Some(attrs) = &attrs {
                    let warnings = nocturne::attrs::warnings(attrs);
                    if !warnings.is_empty() {
                        preview["warnings"] = json!(warnings);
                    }
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
            let text = file
                .map(|file| input(None, Some(file), false))
                .transpose()?;
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
            nocturne::search::search_page(
                c,
                ns,
                query.as_deref(),
                domain.as_deref(),
                (limit, offset),
                mode,
                &filters,
            )
        }
        Command::Review {
            action,
            id,
            batch,
            all,
            all_namespaces,
        } => {
            if all {
                if action != "accept" || id.is_some() || batch {
                    return Err(Error::Invalid(
                        "--all requires review accept without ID or --batch"
                            .into(),
                    ));
                }
                return changes::accept_all(c, ns, all_namespaces);
            }
            if all_namespaces {
                return Err(Error::Invalid(
                    "--all-namespaces requires review accept --all".into(),
                ));
            }
            if action == "list" {
                return changes::list(c, ns);
            }
            let id =
                id.ok_or_else(|| Error::Invalid("review requires ID".into()))?;
            match action.as_str() {
                "diff" if !cli.json => Ok(
                    json!({"unified":changes::diff_text(c, ns, &id, batch)?}),
                ),
                "diff" => changes::diff(c, ns, &id, batch),
                "accept" => changes::accept(c, ns, &id, batch),
                "rollback" => changes::rollback(c, ns, &id, batch),
                _ => Err(Error::Invalid("unknown review action".into())),
            }
        }
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
        Command::Diagnostic {
            domain,
            days_stale,
            max_children,
            longest_limit,
            min_bytes,
        } => nocturne::reports::diagnostic(
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
        Command::ExportDir { output } => {
            nocturne::export_dir::export(c, ns, &output)
        }
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
        _ => unreachable!(),
    }
}
fn input(
    text: Option<String>, file: Option<PathBuf>, stdin: bool,
) -> Result<String> {
    if text.is_some() as u8 + file.is_some() as u8 + stdin as u8 != 1 {
        return Err(Error::Invalid(
            "provide exactly one text, file or stdin input".into(),
        ));
    }
    if let Some(t) = text {
        return Ok(t);
    }
    if stdin || file.as_deref() == Some(std::path::Path::new("-")) {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|error| input_error("stdin", error))?;
        Ok(s)
    } else {
        let file = file.unwrap();
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
    serde_json::from_str(&input(None, Some(file), false)?).map_err(|error| {
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
    let diff_mode = matches!(cli.command, Command::Diff { .. })
        || matches!(&cli.command, Command::Review { action, .. } if action == "diff");
    let doctor_mode = matches!(cli.command, Command::Doctor);
    let boot_mode = matches!(cli.command, Command::Boot { action: None, .. });
    match run(cli) {
        Ok(v) => {
            let output = if json_mode {
                format!("{v}\n")
            } else if diff_mode {
                v["unified"].as_str().unwrap_or_default().to_owned()
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
                let storage = v["issues"].as_array().is_some_and(|issues| {
                    issues.iter().any(|issue| issue["kind"] == "storage")
                });
                std::process::exit(if storage { 5 } else { 2 });
            }
            if doctor_mode && v.get("ok") == Some(&Value::Bool(false)) {
                std::process::exit(1);
            }
            if boot_mode
                && v["failed"]
                    .as_array()
                    .is_some_and(|failed| !failed.is_empty())
            {
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

fn text_value(value: &Value, indent: usize, record: bool) -> String {
    let prefix = " ".repeat(indent);
    match value {
        Value::Object(fields) if !fields.is_empty() => {
            // Only result records have CLI semantics; nested objects may be arbitrary user data.
            let memory = record
                && fields.contains_key("uri")
                && fields.contains_key("version")
                && fields.contains_key("content");
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
            } else if record
                && ["id", "status", "created_at", "namespace"]
                    .iter()
                    .all(|key| fields.contains_key(*key))
            {
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
