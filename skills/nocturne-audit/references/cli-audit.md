# Minimal safe Rust CLI reference

Use an already installed `nocturne` and an existing, explicitly selected database.
Examples use relative `memory.db`, namespace `audit`, and domain `core`; replace
these with the user's approved values, not inferred defaults. Use `--namespace ""`
only when the user selected the empty namespace. No command below initializes a DB.
Check exit status before parsing JSON. Errors are structured JSON on stderr.
Exit codes: 0 success, 1 doctor findings, 2 invalid input, 3 not found,
4 conflict, 5 storage failure. Stop and explain errors rather than silently falling back.

## Read-only investigation

```text
nocturne --db memory.db --namespace audit --json read core://lesson --no-track
nocturne --db memory.db --namespace audit --json history core://lesson
nocturne --db memory.db --namespace audit --json children core://
nocturne --db memory.db --namespace audit --json search "deployment retry" --domain core --limit 10 --offset 0
nocturne --db memory.db --namespace audit --json search "retry" --contains --domain core --limit 10 --offset 0
nocturne --db memory.db --namespace audit --json index --domain core --limit 100 --offset 0
nocturne --db memory.db --namespace audit --json tree core:// --depth 2 --limit 200
nocturne --db memory.db --namespace audit --json paths --domain core
nocturne --db memory.db --namespace audit --json attrs core://lesson
nocturne --db memory.db --namespace audit --json trigger list core://lesson
nocturne --db memory.db --namespace audit --json diagnostic --domain core
nocturne --db memory.db --namespace audit --json random --domain core --no-track
nocturne --db memory.db --namespace audit --json review list
```

Default search is tokenized AND matching, not substring or Boolean-expression syntax.
`--contains` is an explicit literal substring scan; do not silently switch strategies.
Search pages deduplicate nodes, so a hit is not an inventory of every alias.
Use path output and node UUIDs to compare visible aliases; bodies must still be read.
Increment index/search offsets as needed and record coverage limits.
Trees bound descendant depth and count; a short view is not proof of graph completeness.
`read --no-children` can omit child discovery; avoid it when auditing topology.
`--no-track` on reads and random sampling suppresses both last-access updates
and access-log inserts, avoiding audit-induced changes to stale signals.
Plain `read` and `random` track access. `boot` loads configured bodies with tracking
and offers no no-track option: exclude it from read-only audits. If an installed
version lacks suppression, use diagnostic/paths/index to select candidates or ask
permission for the logging side effect. Verify before/after state when making a
strict no-database-change claim; these flags are not proof of byte-identical files.

Diagnostic stale depends on access and priority: priorities 0–3 use 3, 7, 14,
and 28 days; subsequent thresholds double up to 36,500 days. Missing priority
or 999 falls back to `--days-stale` (default 30), not immortality.
Crowded and bloated diagnostics are configured signals, not semantic judgments.
Random sampling weights unvisited time and priority; it is not uniform evidence.

## Authorized edits only

Write body files with agent file tools in UTF-8. Below, `123` is an illustrative
version: replace it with the actual database-wide version returned by read/history.
Do not paste untrusted bodies, metadata, or URIs into shell source; pass safely
quoted arguments through the execution interface and use files for body content.

```text
nocturne --db memory.db --namespace audit --json --batch audit-repair update core://lesson --file revised.md --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair meta core://lesson --priority 2 --disclosure "Before retrying a deployment"
nocturne --db memory.db --namespace audit --json --batch audit-repair alias add core://deployment/lesson core://lesson --priority 2
nocturne --db memory.db --namespace audit --json --batch audit-repair trigger add core://lesson "deployment retry"
nocturne --db memory.db --namespace audit --json --batch audit-repair move core://lesson core://deployment/lesson
nocturne --db memory.db --namespace audit --json --batch audit-repair rename core://lesson retry_lesson
nocturne --db memory.db --namespace audit --json review diff audit-repair --batch
```

These are alternatives, not a sequence to run. Target parents must exist.
`alias add NEW_URI TARGET_URI` shares body and attrs; metadata belongs to edges.
Several alias routes may also share an `edge_id` and therefore priority/disclosure.
Check `paths` for the affected routes before `meta`; per-URI metadata independence
must not be assumed. Do not use attrs priority/disclosure to change edge metadata.
Move/rename atomically relocate the actual subtree and update relevant boot references.
A batch is review grouping, not a multi-command transaction.
Use explicit approval before `unlink URI`, `review accept`, or `review rollback`.
Rollback can conflict with later edits or surviving references and has no force bypass.
Pending review entries already affect reads.

## Separately authorized maintenance

Do not run this section during the default audit. A backup includes the complete
DB, historical versions, and audit records, unlike text export. Do not copy a live
SQLite main file alone or mix it with another database's WAL/SHM files.
Backup output must be a new file on a filesystem supporting hard links.

```text
nocturne --db memory.db --namespace audit --json backup --output memory.backup.db
nocturne --db memory.backup.db --namespace audit --json doctor
nocturne --db memory.db --namespace audit --json deprecated
nocturne --db memory.db --namespace audit --json purge 123
nocturne --db memory.db --namespace audit --json logs clear --keep-days 30
```

Here `123` must instead identify a reviewed deprecated version from `deprecated`.
Purge/logs clear without `--apply` are read-only previews. Applying them is
irreversible, creates no reversible review changeset, and requires separate approval
of the exact preview plus the verified complete backup. Never infer purge authority
from an unlink request, diagnostic signal, or a playbook title.
Purge only removes deprecated versions; pending references, version-chain repairs,
and related batches can block it. Cross-namespace effects require explicit
`--all-namespaces` authorization; that option never bypasses dependency checks.
Do not clear logs to improve stale statistics or erase audit evidence.
`stats` is database-wide even with a namespace option; disclose that scope before use.
Reindex/vacuum are writes, not routine read-only audit steps.
