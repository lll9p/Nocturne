# CLI workflows

Replace `memory.db` and `project` below with the user-authorized `--db` and `--namespace`. Check `nocturne <command> --help` if an option is unfamiliar. Place temporary input files in a temporary location and remove them once the command finishes.

## Bounded retrieval and navigation

```text
nocturne --db memory.db --namespace project --json search "session renewal" --limit 5 --offset 0
nocturne --db memory.db --namespace project --json search "session renewal" --with-total --limit 5
nocturne --db memory.db --namespace project --json search "session renewal" --with-total --limit 0
nocturne --db memory.db --namespace project --json search "renewal" --contains --attr status=active --limit 5
nocturne --db memory.db --namespace project --json read core://session-policy --no-track
nocturne --db memory.db --namespace project --json children core://
nocturne --db memory.db --namespace project --json index --domain core --limit 50 --offset 0 --with-total
nocturne --db memory.db --namespace project --json tree core:// --depth 2 --limit 50
nocturne --db memory.db --namespace project --json paths --domain core
nocturne --db memory.db --namespace project --json history core://session-policy
```

- **Search modes:** Default FTS requires all tokenized terms (`AND`); `--or` matches any tokenized term; `--contains` scans active bodies, paths/URIs, disclosures, and namespace triggers for the literal substring (ASCII case-insensitive; case-sensitive for non-ASCII characters; incompatible with `--or`). FTS supports CJK (`jieba`) plus full Unicode alphanumeric scripts and combining marks, but rejects raw Boolean/FTS syntax (`AND`, `OR`, quotes, `*`, parentheses).
- **Attribute filters:** Repeatable `--attr KEY=VALUE` matches exact top-level string fields in `attrs` (literal keys, not JSON paths). Attributes are not FTS-indexed.
- **Pagination envelopes:** Without `--with-total`, `search --json` and `index --json` return a JSON array. With `--with-total`, both return `{total, offset, limit, has_more, results}`, and `--limit 0` counts without loading bodies/snippets. Request `--with-total` only when total size informs paging: exact totals traverse all matches (`search` deduplicates by `node_uuid`; `index` counts routes).

## Create, update, patch, and metadata

`version` is a database-wide integer content ID (`memory_id`), distinct from `node_uuid` and changeset `change_id`. Replace `42` below with the actual `version` from `read`.

```text
# Create a child under an existing parent or domain root (--title is the URI segment name)
nocturne --db memory.db --namespace project --json create core:// --title session-policy --file /tmp/decision.md --priority 2 --disclosure "When changing session expiry or renewal" --attrs-file /tmp/attrs.json

# Full-body and/or edge-metadata update in one transaction
nocturne --db memory.db --namespace project --json --batch session-fix update core://session-policy --file /tmp/revised.md --expect-version 42

# Exact literal block replacement (no '...' wildcards, no '\n' escape translation; CAS checked before matching)
nocturne --db memory.db --namespace project --json --batch session-fix patch core://session-policy --old-file /tmp/old.txt --new-file /tmp/new.txt --expect-version 42

# Genuine addition at the end of a record (not for leaving stale claims above corrections)
nocturne --db memory.db --namespace project --json append core://session-policy --file /tmp/addition.md --expect-version 42

# Edge metadata only (pass --disclosure "" to clear; returns the updated memory object)
nocturne --db memory.db --namespace project --json meta core://session-policy --priority 1 --disclosure "When changing idle session timeout" --expect-version 42

# Shared node JSON attributes (--set stores top-level strings; --file replaces the JSON object, {} clears)
nocturne --db memory.db --namespace project --json attrs core://session-policy --set status=active --unset draft --expect-version 42
```

- **Direct verification:** `create`, `update`, `patch`, `append`, and `meta` return the complete updated memory JSON (`uri`, `version`, `node_uuid`, `content`, `priority`, `disclosure`, `attrs`, `changed`, and `change_id` when `changed: true`). Inspect that JSON directly rather than running an extra `read`, and delete temporary input files immediately.
- **Edge vs. node metadata:** `priority` (`0..=999`, lower = higher priority) and `disclosure` live on edges; `attrs` live on the node and are shared across all aliases. Custom `attrs.priority` / `attrs.disclosure` keys emit `warnings` and never alter edge metadata.

## Aliases, triggers, and route refactoring

```text
# Add an alias route: alias SOURCE DESTINATION (creates an independent edge for DESTINATION)
nocturne --db memory.db --namespace project --json alias core://session-policy notes://session-policy --priority 2 --disclosure "When investigating login renewal" --expect-version 42

# Manage namespace trigger keywords idempotently (pass multiple words in one call)
nocturne --db memory.db --namespace project --json trigger add core://session-policy renewal expiry --expect-version 42
nocturne --db memory.db --namespace project --json trigger edit core://session-policy --add timeout --remove renewal --expect-version 42
nocturne --db memory.db --namespace project --json trigger list core://session-policy

# Atomic subtree relocation (rewrites matching boot entries for the selected namespace)
nocturne --db memory.db --namespace project --json rename core://session-policy session-expiry --expect-version 42
nocturne --db memory.db --namespace project --json move core://session-expiry archive://session-expiry --expect-version 42

# Remove a single route (protects reachable children; node becomes an orphan only if no route remains anywhere)
nocturne --db memory.db --namespace project --json unlink notes://session-policy --expect-version 42
```

- **Edge independence:** `alias SOURCE DESTINATION` creates its own edge so changing `priority` or `disclosure` on `notes://session-policy` does not overwrite `core://session-policy`. If `SOURCE` has children, `alias` derives descendant routes under `DESTINATION` that share those child edges; check `paths --node <UUID>` before editing derived child metadata.
- **Trigger indexing:** `trigger add`, `remove`, and `edit` all return `{uri, node_uuid, keywords, added, removed, skipped, changed}`. Pure CJK keywords update the namespace `jieba` dictionary and retokenize the namespace; batch multiple CJK keyword edits into one `trigger edit/add` call on large libraries.

## Historical restore, orphan recovery, and startup configuration

Use only when requested or authorized by the user:

```text
# Restore a node's content from an earlier version ID shown by `history`
nocturne --db memory.db --namespace project --json restore core://session-policy 38 --expect-version 42

# Inspect and recover pathless nodes last unlinked in this namespace
nocturne --db memory.db --namespace project --json orphans list
nocturne --db memory.db --namespace project --json orphans show 38
nocturne --db memory.db --namespace project --json orphans restore 38 core://session-policy --priority 2

# Inspect or configure namespace startup memories and database-wide named presets
nocturne --db memory.db --namespace project --json boot --no-track
nocturne --db memory.db --namespace project --json boot list
nocturne --db memory.db --namespace project --json boot set core://session-policy
nocturne --db memory.db --namespace project --json preset list
nocturne --db memory.db --namespace project --json preset create default-work --file /tmp/boot-map.json --activate
```

`boot` loads only the active preset's list for the selected `--namespace` (no fallback to `""`). If any configured URI is missing, `boot` prints `{loaded, failed}` on `stdout`, emits `not_found` JSON on `stderr`, and exits `3`.

## Review queue, net diffs, and rollback

`--batch <BATCH_ID>` is a global write-grouping flag (valid before or after the subcommand). In `review diff/accept/rollback`, pass `--by-batch` to treat `<ID>` as a batch name rather than a single changeset UUID.

```text
nocturne --db memory.db --namespace project --json review list --status pending --limit 20
nocturne --db memory.db --namespace project --json review diff session-fix --by-batch
nocturne --db memory.db --namespace project --json review accept session-fix --by-batch
nocturne --db memory.db --namespace project --json review rollback session-fix --by-batch
```

Inspect `review diff` before `accept` or `rollback`, and run either only with explicit authorization. Rollback fails atomically with exit `4` (`reason: "rollback_conflict"`) if later edits or surviving references depend on the target state.

## Exit codes and error stream

Every nonzero exit writes `{"error": {"code": ..., "message": ...}}` to `stderr`:
- `0`: success
- `1`: `doctor` integrity failure (`code: "integrity"`, full report also on `stdout`)
- `2`: invalid arguments, URI, non-UTF-8 input, missing `--db`, or invalid `import-dir --dry-run` (`code: "invalid"`)
- `3`: target not found, or `boot` failed to resolve configured URIs (`code: "not_found"`)
- `4`: concurrency or state conflict (`version_mismatch`, `patch_no_match`, `patch_ambiguous`, `rollback_conflict`, occupied URI)
- `5`: SQLite or filesystem I/O error (`code: "storage"`)
