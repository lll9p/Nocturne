# Nocturne

[中文](README.md) | English

**Nocturne** is a local-first, versioned memory engine with reversible writes, delivered as a single self-contained executable (`nocturne`). SQLite and full-text search are bundled into the binary; no external runtime or background server is required.

This project was inspired by [Dataojitori/nocturne_memory](https://github.com/Dataojitori/nocturne_memory) and developed from a fork of that repository, with the implementation rewritten and modified.

Versions are managed in [`Cargo.toml`](Cargo.toml): the application uses `workspace.package.version` (shown by `nocturne --version`), while the database format uses `package.metadata.nocturne.schema-version` and does not automatically change with application releases. Initialize a new database at a new path with `nocturne init`; unrecognized or older database formats are rejected directly without automatic upgrades. Alongside core reads, writes, and full-text search, the engine includes recursive directory import (`import-dir`), readable directory export (`export-dir`), historical version restore (`restore`), changeset rollback (`review rollback`), and online consistent backups (`backup`).

---

## Build and Install

The project uses **Rust 2024 edition**. Install the current stable Rust toolchain, then run from the repository root:

```sh
cargo build --locked --release
cargo install --locked --path .
nocturne --help
```

- `Cargo.lock` is tracked in version control; pass `--locked` for reproducible builds and installs.
- CI runs formatting checks, Clippy, release builds, and portable acceptance on Ubuntu, macOS, and Windows, and uploads binaries as GitHub Actions workflow artifacts rather than GitHub Releases.
- Windows release binaries built by CI link the static CRT (`+crt-static`) so they do not require a separate Visual C++ redistributable. To build a static-CRT binary locally on Windows MSVC, set `RUSTFLAGS` explicitly:
  ```sh
  RUSTFLAGS='-C target-feature=+crt-static' cargo build --locked --release --bin nocturne
  ```

---

## Agent Skills

[`skills/`](skills/README.md) provides two independently installable, portable skills: `nocturne` for everyday retrieval, recording, and correction, and `nocturne-audit` for evidence and retrieval-path audits, read-only by default. Neither requires a specific operating system, agent, or MCP server.

After committing and pushing these files, install them with [Vercel Skills CLI](https://github.com/vercel-labs/skills):

```text
npx skills add https://github.com/lll9p/Nocturne --skill nocturne nocturne-audit
```

For an unpublished checkout, run `npx skills add . --skill nocturne nocturne-audit` from the repository root. The installer lets you choose agents; add `--global` for user-level installation. **Installing a skill does not install the Nocturne executable.** Install the CLI separately and supply an authorized database and namespace. See the [skills guide](skills/README.md) for details.

---

## Quick Start

The following commands run sequentially from start to finish, covering database initialization, memory creation, search, exact patching, and batch review:

```sh
# 1. Initialize a new database (only `init` creates a database file, and it refuses existing files)
nocturne --db memory.db init

# 2. Create a memory node under `core://` (`core://architecture`) grouped under the `setup` review batch
printf 'Single-binary SQLite memory store with full-text search and reversible writes.\n' > note.md
nocturne --db memory.db --batch setup create core:// --title architecture --file note.md --priority 1 --disclosure 'Core architecture overview'

# 3. Read the memory (content, version ID, edge metadata, children, and node attributes)
nocturne --db memory.db read core://architecture

# 4. Search full text and apply an exact literal patch in the same batch
nocturne --db memory.db search 'reversible'
nocturne --db memory.db --batch setup patch core://architecture --old 'full-text search' --new 'jieba-tokenized FTS5 search'

# 5. Inspect the review queue, view the net diff for the `setup` batch, and accept pending changes
nocturne --db memory.db review list --status pending
nocturne --db memory.db review diff setup --by-batch
nocturne --db memory.db review accept --all
```

Run `nocturne <COMMAND> --help` to inspect the full option set for any command.

---

## Core Concepts and Data Model

### 1. Database Path and Namespaces
- **Explicit path and initialization**: Every database command requires `--db <PATH>` or `NOCTURNE_DB` (there is no implicit user-directory fallback; only standalone `diff` runs without a database). Only `init` creates a database file, preventing mistyped paths from silently creating empty databases.
- **Resolution order**: Explicit flags `--db <PATH>` and `--namespace <NAME>` take precedence over environment variables `NOCTURNE_DB` and `NOCTURNE_NAMESPACE`. When `--namespace` is omitted, the default namespace is the empty string `""`. Pass `--db` and `--namespace` explicitly in scripts and automation.
- **Global flag placement**: `--db`, `--namespace`, `--json`, and the write-grouping flag `--batch <BATCH_ID>` are all global and may appear before or after the subcommand.

### 2. Nodes, Routes (URIs), and Edge Metadata
- **URIs and aliases**: A URI (such as `core://architecture`) identifies a route within the selected namespace. Both domains and newly created path segments allow Unicode letters, numbers, combining marks, `-`, and `_` (not restricted to ASCII, e.g., `文献://水泥/阿利特`). Multiple URIs (aliases) can point to the same stable node UUID and share its body content and JSON attributes.
- **Edge metadata vs. node attributes**:
  - **Edge metadata**: `priority` (integer `0–999`, where lower numbers mean higher priority) and `disclosure` belong to the **graph edge**, governing hierarchy annotations, search tie-breaking, and review weights. Every explicit `alias <SOURCE> <DESTINATION>` creates an independent edge so its `priority` and `disclosure` never overwrite the source route (only descendant routes auto-derived under an aliased parent share the underlying child edges).
  - **Node attributes**: JSON key-value objects managed via `attrs` belong to the **node** and are shared across every alias pointing to that node.

### 3. Database-Wide Version IDs
- Every new content version receives a database-wide integer `memory_id` (displayed as `version` in output and accepted by `--expect-version` and `restore`).
- **Important**: Versions are **database-wide integer IDs**, not per-node counters starting at 1. Always use the actual version number returned by `read` or `history`.

### 4. Immediate Writes and the Three Removal Operations
- **Writes take effect immediately**: Every write updates business tables and records a reversible audit changeset (identified by a UUID `change_id`) in the same SQLite transaction. `review accept` simply marks an already effective change as reviewed; it does not publish pending drafts.
- **Three distinct removal/undo operations**:
  1. `unlink <URI>`: Removes a route in the selected namespace while protecting dependent children from becoming unreachable. If no route remains in any namespace, the node becomes a recoverable pathless node (`orphans`).
  2. `review rollback <ID>`: Reverts a changeset or batch back to its baseline state.
  3. `purge <MEMORY_ID>`: Permanently and irreversibly deletes a deprecated historical version (`deprecated`).

---

## Daily Workflow Guide

### Writing, Atomic Editing, and Node Attributes

```sh
# Create a child node with initial node attributes from a JSON file
nocturne --db memory.db create core:// --title guidelines --file spec.md --priority 1 --attrs-file attrs.json

# Update content and edge metadata atomically in one transaction (omit --file for metadata-only updates)
nocturne --db memory.db update core://guidelines --file spec_v2.md --priority 2 --disclosure 'Read before coding'

# Exact literal block replacement (--old/--new or --old-file/--new-file; no wildcards, regexes, or escape translation; checks --expect-version first)
nocturne --db memory.db patch core://guidelines --old-file old_part.md --new-file new_part.md

# Optimistic concurrency: pass the actual <VERSION> returned by `read` (supported across update/patch/append/meta/attrs/alias/move/rename/unlink/trigger)
nocturne --db memory.db update core://guidelines --file spec_v3.md --expect-version <VERSION>

# Append UTF-8 text and update edge metadata alone (pass '' to --disclosure to store an empty string; both return the updated memory record)
nocturne --db memory.db append core://guidelines --file extra.md
nocturne --db memory.db meta core://guidelines --priority 1 --disclosure ''

# Inspect and edit shared node JSON attributes (--set values are always stored as top-level strings)
nocturne --db memory.db attrs core://guidelines
nocturne --db memory.db attrs core://guidelines --set status=active --set owner=team --unset draft
nocturne --db memory.db attrs core://guidelines --file attrs.json

# Manage trigger keywords idempotently (add/remove/edit return one unified JSON shape; pure CJK words also update the namespace tokenizer dictionary)
nocturne --db memory.db trigger add core://guidelines Architecture Spec
nocturne --db memory.db trigger edit core://guidelines --add Guide --remove Spec
nocturne --db memory.db trigger list core://guidelines
```

- **Path segment semantics**: `create <PARENT_URI> --title <NAME>` appends `NAME` as a child path segment beneath an existing parent URI; `--title` is a route segment name, not a free-form display title.
- **Byte fidelity and no-op writes**: Content is stored as exact UTF-8 bytes—preserving NUL, CRLF, combining characters, and missing trailing newlines without NFC normalization. If a write produces no net change to content or edge metadata, it returns `changed: false` without creating a new version, changeset, or empty review batch.
- **Attribute naming warnings**: Custom fields named `priority` or `disclosure` inside node attribute JSON do not alter edge metadata; instead, they produce root-level `warnings` in the output or preview.
- **Standard input (`stdin`)**: `--file -` reads stdin for `create`, `update`, `append`, `attrs`, and `preset create/update`. `--attrs-file -` is supported on `create` and `import-dir`, and `-` is supported on one side of `patch` or `diff`. A single command invocation cannot consume stdin twice.

### Search and Bounded Browsing

Within the selected namespace, Nocturne supports three text search modes and exact top-level attribute filtering. Pagination always pages **deduplicated nodes** rather than aliases (default `--limit 10 --offset 0`):

```sh
# 1. Default tokenized FTS search (jieba terms combined with AND, ranked by BM25)
nocturne --db memory.db search 'memory architecture' --domain core

# 2. Any-term tokenized search (--or matches any tokenizer term; requires a query)
nocturne --db memory.db search 'memory architecture' --or --domain core

# 3. Literal substring scan (--contains scans raw substrings without jieba or BM25)
nocturne --db memory.db search '阿利特' --contains --limit 50 --offset 0

# 4. Exact top-level string attribute filters (repeatable with AND; works alone or with text queries)
nocturne --db memory.db search --attr status=active --attr owner=team
nocturne --db memory.db search '晶型' --contains --attr 'source=GB/T 21372'
```

- **Tokenized scripts and query syntax**: Tokenized search modes (default and `--or`) index CJK terms (`jieba`) alongside full Unicode alphanumeric scripts and combining marks (Latin, Greek such as `β-C2S`, Cyrillic, etc., with NFC normalization and `unicode61` folding). Queries are plain terms rather than a Boolean or FTS expression language; bare Boolean operators inside multi-word expressions or raw FTS syntax return an invalid-input error (`2`).
- **Literal substring scan (`--contains`)**: Default FTS matches jieba tokens. When a shorter CJK term (such as `阿利特`) is embedded inside a longer compound token (such as `阿利特晶型`), pass `--contains` to scan active bodies, paths/URIs, disclosures, and current-namespace trigger keywords for the entire query as a literal substring (without tokenization or Boolean operator parsing; `%` and `_` are literal characters, and ASCII matching is case-insensitive). `--contains` is an explicit scan mode—**not an automatic fallback and not a unigram index**—and cannot be combined with `--or`.
- **Attribute filter rules**: `--attr KEY=VALUE` matches top-level string values only. Dots in keys are treated as literal characters rather than JSON paths; numbers, booleans, arrays, and nested objects are not matched, and attributes are not indexed in FTS.

#### Search Boundaries and Exact Counts

Human-readable output reports the current page and whether more results exist. By default, it looks ahead by one distinct node instead of traversing all matches for a count. Request exact counts explicitly:

```sh
nocturne --db memory.db --json search mineralizer --with-total --limit 5
# Count only, without loading result bodies or generating snippets
nocturne --db memory.db --json search mineralizer --with-total --limit 0
```

With `--with-total`, JSON becomes `{ "total": 40, "offset": 0, "limit": 5, "has_more": true, "results": [...] }` (illustrative numbers). Without it, JSON remains a result array. `total` counts distinct matching nodes after namespace, domain, attribute, and query filtering; aliases are not counted twice. Counts and the returned page share one read snapshot. Out-of-range pages still include the count. Count-only results are empty; `has_more` indicates whether matching nodes remain after the offset.

Exact counts traverse all matching candidates; `--contains` additionally scans text. Only returned results load bodies for snippets. Counts describe **query size, not research recall**; counts from different queries can overlap. Exploratory questions require alternative search directions, source reading, and explicit gaps rather than treating the first relevant page as exhaustive. Routine lookups need not request counts.

Navigation and version history commands (replace `<VERSION>` with an actual version ID from `history`):

```sh
# Read a memory (--no-track skips updating access metadata; --no-children skips the children query)
nocturne --db memory.db read core://guidelines --no-track --no-children

# Inspect children, recent updates, version history, and restore a historical version
nocturne --db memory.db children core://
nocturne --db memory.db recent --limit 10
nocturne --db memory.db history core://guidelines
nocturne --db memory.db restore core://guidelines <VERSION>

# Paginated route index (supports --with-total envelope) and bounded tree traversal (safety caps: depth 128, 10,000 total descendants)
nocturne --db memory.db index --domain core --limit 100 --offset 0 --with-total
nocturne --db memory.db tree core:// --depth 2 --limit 200

# Discover namespaces and list routes without loading content bodies
nocturne --db memory.db namespaces
nocturne --db memory.db paths --domain core
nocturne --db memory.db paths --all-namespaces
```

### Review, Batches, and Safe Rollback

```sh
# Group multiple writes into one review batch (global --batch <BATCH_ID> may appear before or after the subcommand)
nocturne --db memory.db --batch refactor-notes create core:// --title part1 --file p1.md
nocturne --db memory.db update core://guidelines --file spec_v2.md --batch refactor-notes

# Filter pending reviews, inspect the net batch diff, and roll back the batch (--by-batch selects batch mode; omit it when passing a single changeset UUID)
nocturne --db memory.db review list --status pending --limit 20
nocturne --db memory.db review diff refactor-notes --by-batch
nocturne --db memory.db review rollback refactor-notes --by-batch

# Accept all pending changes in the selected namespace (add --all-namespaces for the whole database)
nocturne --db memory.db review accept --all
```

- **`--batch <BATCH_ID>` vs. `--by-batch`**:
  - **Global `--batch <BATCH_ID>`**: Tags a write operation with a namespace-scoped batch identifier for grouped review. **It does not merge separate processes or separate CLI invocations into a single database transaction.**
  - **`review diff/accept/rollback <ID> --by-batch`**: A boolean flag telling `review` to interpret `<ID>` as a batch name rather than a single changeset UUID. Human-readable batch diffs pair content versions by node UUID and show the net start-to-finish unified diff without intermediate versions (`--json` preserves the full row-snapshot array).
- **Conflict-safe rollback (no force bypass)**: If a later conflicting edit or a surviving route/reference blocks rollback, the operation fails atomically with exit code `4` (`reason: "rollback_conflict"`) without overwriting later work or partially reverting the batch. **There is no `--force` flag.** Inspect the reported `blocking_uris`, `related_changesets`, and `foreign_references` (which expose only current-namespace details while keeping foreign namespaces private), revert the later changesets first, or group related writes into the same batch from the start. Ordinary `read` access tracking never blocks rollback.

### Path Refactoring, Aliases, and Pathless Memories (Orphans)

```sh
# Rename the final path segment (subtree follows) or move a subtree to an existing parent/domain root
nocturne --db memory.db rename core://guidelines coding_standards
nocturne --db memory.db move core://coding_standards archive://coding_standards

# Add an alias route with independent edge metadata (alias <SOURCE> <DESTINATION>), or unlink a single route
nocturne --db memory.db alias archive://coding_standards notes://standards --priority 2
nocturne --db memory.db unlink notes://standards

# Discover, inspect, and restore nodes with no remaining URI in any namespace (<MEMORY_ID> from `orphans list`)
nocturne --db memory.db orphans list
nocturne --db memory.db orphans show <MEMORY_ID>
nocturne --db memory.db orphans restore <MEMORY_ID> core://recovered_standards --priority 1
```

- `rename` and `move` relocate only the selected namespace's URI subtree, prechecking all destination routes for cycles and collisions in a single reversible changeset. Boot references for the selected namespace are rewritten across all presets without touching other namespaces.
- `orphans` operates exclusively on nodes with **no URI in any namespace** (to inspect historical versions of nodes that still have a route, use `history` or `deprecated`). By default, only nodes whose last recorded `unlink` belongs to the selected namespace are exposed; pass `--all-namespaces` explicitly to inspect or restore imported pathless nodes without audit provenance or to perform cross-namespace recovery.

### Directory Import and Readable Export

```sh
# Validate and preview a recursive directory import without modifying the database
nocturne --db memory.db import-dir ./notes --into core:// --sanitize --priority 1 --attrs-file attrs.json --dry-run

# Execute the import in a single reversible transaction
nocturne --db memory.db --batch import-notes import-dir ./notes --into core:// --sanitize --priority 1 --attrs-file attrs.json

# Export the namespace; optionally select a domain or subtree with lit:// or lit://docs
nocturne --db memory.db export-dir --output ./readable-export

# Export a branch with attributes, edge metadata, and sharing information
nocturne --db memory.db export-dir lit://docs --output ./docs-export --with-metadata

# Preview and restore the snapshot in a fresh database, preserving URIs in the current namespace
nocturne --db restored.db init
nocturne --db restored.db import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json --dry-run
nocturne --db restored.db import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json
```

- **`import-dir`**: Recursively imports UTF-8 `.md` and `.txt` files (creating empty-body nodes for directories and skipping/reporting hidden entries, symlinks, and unsupported file types; `README` files receive no special treatment). `--sanitize` preserves valid Unicode name characters and existing `-`/`_`, replaces runs of other characters (spaces, dots, punctuation, brackets, quotes, symbols) with `-`, and trims only leading/trailing replacement runs (preserving original valid `-`/`_`). **Existing routes and sanitization collisions are never overwritten or merged.** Any validation error blocks a real import completely (returning counts and up to three samples; use `--dry-run --json` for the complete `mappings` and `issues` report).
- **Source paths**: Add `--source-path-attr source_path` to an ordinary import to store each new file or directory's original input-relative path as a top-level string attribute in the same transaction, without per-node follow-up writes. Paths include file extensions and use `/` separators. These are names at import time, not names before external preprocessing; no machine-absolute paths are stored and bodies remain unchanged. Omitting the option preserves existing behavior. The key must be nonempty and absent from shared `--attrs-file` attributes (including `null` values). Preview shows the selected key and each source path. The option conflicts with `--manifest`; manifest roundtrips preserve existing source attributes.
- **`export-dir [URI]`**: Without a URI, exports the current namespace. `lit://` selects a domain; `lit://docs` selects that path and its descendants using path-segment boundaries, without expanding to other aliases or namespaces. Bodies retain their original bytes and stream to `domain/path.md` from one read snapshot. A sibling staging directory is published atomically without replacing existing output or recording accesses or changesets.
- **`--with-metadata`**: Adds `nocturne-manifest.json` with the selected URIs, node attributes, edge priority/disclosure, and sharing relationships. `import-dir --manifest <FILE>` validates file references, metadata, and destination paths, then restores them in one reversible transaction with fresh database IDs. Empty directory nodes retain attributes; a content-bearing `parent.md` and `parent/` map to one node. Parent paths outside the exported root must already exist in the destination database.
- **Manifest imports** preserve original URIs in the current `--namespace` and cannot combine `--manifest` with `--into`, `--sanitize`, `--priority`, `--disclosure`, `--attrs-file`, or `--source-path-attr`. The manifest and referenced files must remain inside the export directory; traversal and symlinks are rejected. Ordinary imports continue to use file-layout naming and do not automatically read manifests.
- **Snapshot scope**: A manifest snapshot exchanges current content and metadata. Use a database `backup` to retain history, audits, triggers, and startup presets. Plain exports contain only bodies and directories; reimporting a content-bearing parent without a manifest still encounters a file/directory name collision.

### Startup Memories (Boot) and Presets

```sh
# Configure and read startup memories for the selected namespace (--no-track skips updating access timestamps and logs)
nocturne --db memory.db boot set core://architecture
nocturne --db memory.db boot list
nocturne --db memory.db boot --no-track
nocturne --db memory.db boot

# Manage database-wide named startup presets (`boot.json` e.g. {"": ["core://agent"], "work": ["core://spec"]})
nocturne --db memory.db preset create work --file boot.json --activate
nocturne --db memory.db preset list
nocturne --db memory.db preset show work
nocturne --db memory.db preset duplicate work writing
nocturne --db memory.db preset activate writing
```

- Presets are database-wide named startup URI maps (not access controls or memory copies); at most one preset is active at a time, and all edits are audited and reversible.
- Running `boot` loads only the active preset's entry for the selected `--namespace` (never falling back across namespaces to `""`). If any configured URI fails to load, `boot` prints the full `loaded`/`failed` report to `stdout`, writes structured `not_found` JSON to `stderr`, and exits with status `3`.

---

## Governance, Maintenance, and Safety

### Governance Diagnostics and Weighted Random Review

```sh
# Report stale memories, overcrowded parents, broken parent routes, duplicate aliases, and largest bodies
nocturne --db memory.db diagnostic --domain core --days-stale 30 --max-children 10 --min-bytes 2048

# Sample an active, reachable node weighted by unvisited days and edge priority (--no-track stays read-only)
nocturne --db memory.db random --domain core --no-track
```

- **Staleness thresholds and output keys**: Node age is measured from `coalesce(last_accessed_at, created_at)`. Priorities `0`, `1`, `2`, and `3` become stale after `3`, `7`, `14`, and `28` unvisited days respectively, doubling for each subsequent priority up to `36,500` days. Missing priority or **`priority = 999` falls back to `--days-stale` (default 30 days); it does not mean "never stale"**. Output sections are `stale_nodes`, `crowded_nodes`, `broken_parent_routes` (subpaths missing their immediate parent route in the domain, distinct from pathless `orphans`), `duplicate_aliases`, and `longest_memories`.
- **Domain-scoped governance**: Keep curated knowledge and reference literature in separate domains within a namespace (e.g., `core://` and `lit://`) so you can search across both domains while running `diagnostic --domain core` on curated nodes alone, without inflating literature priorities just to silence staleness warnings (since priority also controls `random` selection weight: `max(unvisited_days, 0.5) * max(0.5^priority, 1e-12)`).

### Purging Deprecated Versions and Clearing Logs (Irreversible)

```sh
# List historical/deprecated content versions visible in the selected namespace
nocturne --db memory.db deprecated

# Preview and execute permanent deletion of a deprecated version (<MEMORY_ID> from `deprecated`; read-only preview without --apply)
nocturne --db memory.db purge <MEMORY_ID>
nocturne --db memory.db purge <MEMORY_ID> --apply

# Preview and clear access logs older than 30 days (--keep-days 0 selects all; preview without --apply)
nocturne --db memory.db logs clear --keep-days 30
nocturne --db memory.db logs clear --keep-days 30 --apply
```

> **Safety Warning**
> - Both `purge` and `logs clear` run as **read-only previews by default**; nothing is deleted unless `--apply` is passed explicitly.
> - These operations are **irreversible maintenance** and do not create reviewable changesets. **Create a `backup` first.**
> - Only deprecated versions may be purged. Pending review changes referencing the node, repaired version chains, or the same namespace's affected review batch block deletion. Effects on shared nodes or another namespace's audit records, keywords, or logs require explicit `--all-namespaces` (which never bypasses pending-dependency checks). When a pathless node has multiple remaining versions, purge older versions first and the `unlink`-dependent provenance version last.
> - Logical deletion is not cryptographic erasure and does not automatically shrink the SQLite file on disk (run `vacuum` to compact free pages).

### Capacity Statistics, Integrity Checks, Compaction, and Standalone Diff

```sh
# Database-wide read-only capacity report (row counts, logical UTF-8 bytes, log retention, SQLite/WAL pages)
nocturne --db memory.db stats

# Rebuild derived search indexes, compact free SQLite pages, and verify database + FTS5 integrity
nocturne --db memory.db reindex
nocturne --db memory.db vacuum
nocturne --db memory.db doctor

# Database-free UTF-8 text comparison (supports --old/--new or --old-file/--new-file, with - for stdin)
nocturne diff --old-file old.txt --new-file new.txt
```

- **`stats`**: Reports database-wide read-only metrics regardless of `--namespace`. Text sizes use UTF-8 bytes; storage sizes use SQLite pages. Node counts exclude ROOT, and pathless-node sizes include all retained versions. Human output shows the metrics; definitions are available in `stats --help` and the JSON `semantics` field. Statistics do not perform cleanup or estimate space recoverable by deletion.
- **Shared body storage**: `stats.contents` separates stored, audit-only, and unreferenced body counts and bytes; `doctor` reports unreferenced leftovers. Bodies reside in an immutable content table shared by versions and audit snapshots; search reads the body through a join instead of retaining another full copy. Review output expands the complete body on demand, and accepting or rolling back a review does not discard its referenced content. The root `Cargo.toml` defines the database format; keep the matching old executable for older databases, which the new executable does not convert.

- **`vacuum`**: Checkpoints the WAL, runs SQLite `VACUUM`, and checkpoints again to reclaim free pages and shrink the database file without deleting history or audit records. Stop competing readers and writers beforehand and allow up to roughly twice the database size in temporary disk space (a blocked checkpoint reports `conflict`).

### Backup and Restore Procedure

```sh
# Create an online consistent backup and verify its integrity
nocturne --db memory.db backup --output memory.backup.db
nocturne --db memory.backup.db doctor
```

1. **Always use `backup`**: Never copy a live SQLite database file directly without its WAL state. The backup destination filesystem must support hard links, which Nocturne uses to publish the completed backup atomically without overwriting an existing file.
2. **Safe restoration**: Stop all writers -> keep the current database file together with its `-wal` and `-shm` sidecars as a separate recovery set -> copy the backup to a new path -> run `nocturne --db <NEW_PATH> doctor` to verify integrity before switching `--db` to the restored file. **Never mix `-wal` or `-shm` sidecars from one database with another database file.**
3. **Format boundary**: Backups must use the supported database format, configured by `package.metadata.nocturne.schema-version` in `Cargo.toml`; restoring a backup is not an upgrade path for older databases.

---

## Automation, Output Format, and Exit Codes

- **Stream contract**: Default output is human-readable text. With `--json`, successful results are written to `stdout` as a single JSON value (not JSONL). Regardless of whether `--json` is passed, **every nonzero exit (1–5) writes structured `{"error": ...}` JSON to `stderr`** with a stable `code` field (plus specific `reason` fields on conflicts).
- **Diagnostic reports on `stdout`**: `doctor` (exit `1`), `import-dir --dry-run` validation failures (exit `2`/`4`/`5`), and `boot` with missing URIs (exit `3`) write their complete diagnostic/preflight report to `stdout` **in addition to** emitting structured error summary JSON on `stderr`.
- **Automation best practices**: Always pass `--json`, `--db`, and `--namespace` explicitly in scripts, and **check the process exit code first**: nonzero exits always provide structured error JSON on `stderr`, with full diagnostic payloads available on `stdout` where applicable.

| Exit Code | Error Code / Case | Meaning |
| :---: | :--- | :--- |
| `0` | — | Success |
| `1` | `integrity` | `doctor` detected database, graph, or FTS5 inconsistencies (full report on `stdout`, error JSON on `stderr`) |
| `2` | `invalid` | Invalid arguments, missing `--db`, bad URI, non-UTF-8 input, or `import-dir --dry-run` input issues |
| `3` | `not_found` | Target not found in the selected namespace, or `boot` encountered failed URI references (`loaded`/`failed` report on `stdout`) |
| `4` | `conflict` | State or concurrency conflict: `version_mismatch`, `patch_no_match`, `patch_ambiguous`, `rollback_conflict`, occupied URI, or import collision |
| `5` | `storage` | Underlying SQLite failure or filesystem I/O error (including actual file I/O errors during `import-dir --dry-run`) |

---

## Development, Verification, and Repository Layout

### Development Checks and Portable Acceptance

Standard workspace checks:

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
```

After building the release binary (`cargo build --locked --release`), run the Rust `xtask` acceptance suites:

```sh
# Portable acceptance: copies the binary to a temp directory outside the repo with an empty PATH (run by CI on all 3 OSes)
cargo xtask portable-smoke
cargo xtask portable-smoke --binary /path/to/nocturne

# Optional large-library scale check: synthetic 1,578-document (~110 MiB) corpus + 59 curated nodes
cargo xtask large-library-smoke --binary /path/to/nocturne --output results.json
```

Use `cargo run --locked -p xtask -- portable-smoke` when locked dependency resolution is required. See [xtask acceptance documentation](xtask/README.md) for coverage and measurement details. Ordinary success workflows belong in `portable-smoke`; regression tests target specific failure risks, with retention reasons documented in [test source comments](tests/).

### Repository Layout

The root directory is a single Cargo workspace containing the default root package `nocturne` (`nocturne` library and `nocturne` binary) and the `xtask` acceptance runner:

- `src/`: Core storage engine, embedded initial SQLite schema (`src/schema.sql`), FTS5/jieba search, graph routing, and CLI implementation.
- `tests/`: Focused regression tests guarding data loss, namespace isolation, index consistency, and process/I/O boundaries, each with its retention rationale in a source comment.
- `xtask/`: Rust portable and large-library acceptance tools (see [xtask/README.md](xtask/README.md)).
- `skills/`: Independently installable, portable Agent Skills for everyday memory operations and read-only-first audits.

Distributed under the [MIT License](LICENSE), retaining the original copyright notice.
