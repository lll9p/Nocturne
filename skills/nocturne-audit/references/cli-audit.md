# Minimal safe Rust CLI reference

Substitute the user-approved `--db`, `--namespace`, and `--domain` for `memory.db`, `audit`, and `core` below. Check process exit status before parsing JSON: every nonzero exit emits `{"error": {"code": ..., "message": ...}}` on `stderr` (`0` ok, `1` `doctor` integrity failure, `2` invalid input/dry-run validation, `3` not found/`boot` missing URI, `4` conflict, `5` storage/I/O). `doctor`, `boot`, and `import-dir --dry-run` also write their full structured report to `stdout` on nonzero exit.

## Read-only investigation

```text
nocturne --db memory.db --namespace audit --json read core://lesson --no-track
nocturne --db memory.db --namespace audit --json history core://lesson
nocturne --db memory.db --namespace audit --json children core://
nocturne --db memory.db --namespace audit --json boot --no-track
nocturne --db memory.db --namespace audit --json boot list
nocturne --db memory.db --namespace audit --json search "deployment retry" --domain core --limit 5 --offset 0
nocturne --db memory.db --namespace audit --json search "deployment retry" --domain core --with-total --limit 5
nocturne --db memory.db --namespace audit --json search "retry" --contains --domain core --limit 5
nocturne --db memory.db --namespace audit --json index --domain core --limit 100 --offset 0 --with-total
nocturne --db memory.db --namespace audit --json tree core:// --depth 2 --limit 200
nocturne --db memory.db --namespace audit --json paths --domain core
nocturne --db memory.db --namespace audit --json attrs core://lesson
nocturne --db memory.db --namespace audit --json trigger list core://lesson
nocturne --db memory.db --namespace audit --json diagnostic --domain core
nocturne --db memory.db --namespace audit --json random --domain core --no-track
nocturne --db memory.db --namespace audit --json orphans list
nocturne --db memory.db --namespace audit --json orphans show 123
nocturne --db memory.db --namespace audit --json review list --status pending --limit 50
```

- **Search & pagination:** Default search is tokenized `AND` across CJK (`jieba`) and Unicode alphanumeric terms; `--or` matches any term; `--contains` performs an explicit ASCII case-insensitive substring scan (case-sensitive for non-ASCII). With `--with-total`, `search` and `index` return `{total, offset, limit, has_more, results}` (`--limit 0` counts without loading bodies). Avoid `read --no-children` when auditing hierarchy.
- **No-track discipline:** `--no-track` on `read`, `random`, and `boot` suppresses both `last_accessed_at` updates and `memory_access_logs` inserts. `boot` reads only the active preset's list for the selected `--namespace` (never falling back to `""`).
- **Diagnostic semantics:** `diagnostic` reports `stale_nodes`, `crowded_nodes`, `broken_parent_routes` (active subpaths missing their immediate parent route in the same domain, distinct from pathless `orphans`), `duplicate_aliases`, and `longest_memories`. Node age uses `coalesce(last_accessed_at, created_at)`: priorities `0/1/2/3` become stale after `3/7/14/28` days, doubling thereafter up to `36,500` days; missing priority or `999` uses `--days-stale` (default `30`).

## Approved repairs (after user authorization)

Write temporary UTF-8 body/patch files to a temporary directory and delete them once the command finishes. Replace `123` with the current integer `version` from `read` / `history`.

```text
nocturne --db memory.db --namespace audit --json --batch audit-repair update core://lesson --file /tmp/revised.md --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair patch core://lesson --old-file /tmp/old.txt --new-file /tmp/new.txt --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair meta core://lesson --priority 2 --disclosure "Before retrying a timed-out deployment" --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair alias core://lesson core://deployment/lesson --priority 2 --disclosure "Before retrying a timed-out deployment" --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair trigger edit core://lesson --add "deployment retry" --remove "retry" --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair move core://lesson core://deployment/lesson --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair rename core://lesson retry_lesson --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair restore core://lesson 98 --expect-version 123
nocturne --db memory.db --namespace audit --json --batch audit-repair orphans restore 98 core://recovered_lesson --priority 2
nocturne --db memory.db --namespace audit --json review diff audit-repair --by-batch
```

- `patch` performs strict literal matching (no `...` wildcards or `\n` escape translation) and checks `--expect-version` first.
- `alias SOURCE DESTINATION` creates an independent edge for `DESTINATION`; only children auto-derived beneath an aliased parent share child `edge_id`s (inspect via `paths --node <UUID>`).
- `update`, `patch`, `meta`, `alias`, `restore`, and `orphans restore` return the full updated memory record in `--json`, so no redundant follow-up `read` is needed to check those fields.
- Require explicit approval before `unlink`, `review accept` (`--by-batch`), or `review rollback` (`--by-batch`).

## Separately authorized irreversible maintenance

Never run during a routine audit. Always create and verify a consistent `backup` first (destination filesystem must support hard links; never copy a live SQLite main file without its WAL state).

```text
nocturne --db memory.db --namespace audit --json backup --output memory.backup.db
nocturne --db memory.backup.db --namespace audit --json doctor
nocturne --db memory.db --namespace audit --json stats
nocturne --db memory.db --namespace audit --json deprecated
nocturne --db memory.db --namespace audit --json purge 98
nocturne --db memory.db --namespace audit --json logs clear --keep-days 30
```

- `stats` is database-wide regardless of `--namespace`; disclose that scope before running. `reindex` and `vacuum` take write locks and are maintenance operations, not read-only checks.
- `purge <MEMORY_ID>` and `logs clear` preview by default and delete permanently only when `--apply` is passed after explicit approval of the preview. `purge` deletes deprecated versions only and refuses pending dependencies or edits that would alter surviving orphan namespace ownership.
