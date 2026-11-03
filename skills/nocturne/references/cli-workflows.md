# CLI workflows

All examples assume an already installed CLI and explicitly authorized existing database
`memory.db`, namespace `project`. Replace these with the actual authorized scope.
Use `nocturne <command> --help` to check the installed interface. Do not initialize or
install anything merely to make an example work.

## Bounded retrieval

```text
nocturne --db memory.db --namespace project --json search session --limit 10 --offset 0
nocturne --db memory.db --namespace project --json search session --contains --limit 10 --offset 0
nocturne --db memory.db --namespace project --json read core://session-policy --no-track
nocturne --db memory.db --namespace project --json children core://
nocturne --db memory.db --namespace project --json tree core:// --depth 2 --limit 50
nocturne --db memory.db --namespace project --json history core://session-policy
```

FTS uses tokenization and defaults to AND between terms. `--or` means any token;
`--contains` scans the entire literal query as a substring (ASCII case-insensitive),
including active body, URI, disclosure, and namespace triggers. Neither accepts a
Boolean query language. A compound like `CacheInvalidationPolicy` may require
`search invalidation --contains` after zero tokenized hits. Read original bodies.
Search pages deduplicate by node; use bounded pagination only as needed.
`--attr status=active` filters exact top-level string attributes; attributes are not FTS text.

## Correct an existing record without changing its identity

Read the original body and actual `version`. Use the agent's file tool to write
`corrected.md` as UTF-8, preserving unaffected material and useful provenance.
The illustrative integer `42` below must be replaced by the actual returned version.

```text
nocturne --db memory.db --namespace project --json read core://session-policy --no-track
nocturne --db memory.db --namespace project --json --batch session-correction update core://session-policy --file corrected.md --expect-version 42
nocturne --db memory.db --namespace project --json read core://session-policy --no-track
nocturne --db memory.db --namespace project --json review diff session-correction --batch
```

For a narrow correction, write `old-block.txt` and `new-block.txt` with the file tool:

```text
nocturne --db memory.db --namespace project --json patch core://session-policy --old-file old-block.txt --new-file new-block.txt --expect-version 42
```

Use a unique literal old block; no fuzzy matching. On no match, ambiguity, or version
mismatch, reread and reconcile. Do not weaken the guard or apply an unrelated replacement.
`append --file addition.md --expect-version 42` is suitable only for a genuine addition,
not for leaving a stale assertion next to its correction. Check installed help first.

`version` is the database-wide integer content ID; `node_uuid` is stable identity;
`change_id` is an audit changeset UUID. They are not interchangeable.
Metadata-only changes do not generate a new content version. CAS guards the content
version, not a complete snapshot of all metadata or topology.

## Create only when no suitable node exists

Read the parent and related records first. A domain root such as `core://` is a
creation/navigation anchor, not necessarily a readable memory body; inspect its
`children` instead of inventing a filler parent to satisfy a read.
Write `decision.md` with the file tool:

```text
nocturne --db memory.db --namespace project --json create core:// --title session-policy --file decision.md --priority 2 --disclosure "When changing session expiry or renewal"
nocturne --db memory.db --namespace project --json read core://session-policy --no-track
```

`--title` is a child URI segment, not arbitrary display text. Unicode letters, digits,
combining marks, hyphens, and underscores are supported. Do not assume ASCII-only names.
Priority/disclosure are edge metadata; attrs are shared node JSON, not substitutes
for edge metadata. All aliases share body and attrs, potentially across namespaces.
Several routes can also share one `edge_id`. Inspect `paths` before a metadata edit:
priority/disclosure changes affect all routes sharing that edge, not just the given URI.

## Targeted discoverability and structure

Check destination parents, collisions, and shared-node impact before authorized changes:

```text
nocturne --db memory.db --namespace project --json alias add notes://session-policy core://session-policy --priority 2 --disclosure "When investigating login renewal"
nocturne --db memory.db --namespace project --json trigger add core://session-policy renewal expiry
nocturne --db memory.db --namespace project --json trigger list core://session-policy
nocturne --db memory.db --namespace project --json rename core://session-policy session-expiry
nocturne --db memory.db --namespace project --json move core://session-expiry archive://session-expiry
```

These illustrate separate choices, not a sequence to execute indiscriminately.
`alias add DESTINATION SOURCE` adds a route to the same node. `rename`/`move` move
an existing URI subtree atomically; destination parent must exist.
`unlink URI` removes a route, not historical content; child reachability is protected.
Do not unlink, restore, or reorganize records without authorization. Purging history,
log deletion, full-library audits, and global configuration are outside this skill.

## Review and conflict handling

```text
nocturne --db memory.db --namespace project --json review list
nocturne --db memory.db --namespace project --json review diff session-correction --batch
```

Global `--batch NAME` goes before the command and labels that write. After
`review diff/rollback ID`, `--batch` is a Boolean flag interpreting ID as a batch name.
Without it, ID is a changeset UUID. Grouping is not a cross-command transaction.

Only on separate explicit authorization, inspect the diff and use `review accept ID`
(or `review accept NAME --batch`) for the intended change. Do not automatically accept,
use `accept --all`, or use `--all-namespaces`. Writes already took effect before review.

If explicitly asked to undo, inspect `review diff ID` before `review rollback ID`.
Rollback is atomic and can fail on later edits or surviving references, including
cross-namespace references. Respect `rollback_conflict`; there is no force bypass.
Do not remove blockers without further authorization or expose unrelated namespaces.

## Verify results and errors

Check process status first, then success JSON. Errors are structured JSON on stderr.
Exit codes: 0 success; 1 doctor integrity warnings; 2 invalid input; 3 not found;
4 conflict; 5 storage/I/O failure. Some diagnostic commands report on stdout while
exiting nonzero. Never suppress or treat these errors as successful operations.

Read back every changed node, checking UUID, full body, relevant metadata and triggers.
`changed: false` means no net change: no new version, changeset, or empty batch.
Keep multiline content in UTF-8 files, not shell interpolation or pipelines.
