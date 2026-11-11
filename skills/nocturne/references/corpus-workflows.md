# Corpus workflows

Use for authorized knowledge curation, source archiving, and directory snapshot exchange. Confirm `--db`, `--namespace`, source directory, and target URI scope before writing. Write any temporary helper files (such as `intro.md` or `attrs.json`) to a temporary path and clean them up afterward.

## Choose the deliverable before importing

Follow the user's established intent; clarify only material ambiguities that change the deliverable:

- **Archive, migrate, or mirror sources:** preserve exact UTF-8 bodies and source-path mappings. Complete this directly without imposing synthesis or rewriting.
- **Organize knowledge or build a research library:** identify target questions, material scope, and required depth; select and read relevant sources; synthesize reusable, question-oriented nodes. Importing raw files or splitting chapters is an archival/navigation step, not completed curation.

Compact curation sequence:

1. **Select and read:** classify candidate sources as core, related, or deferred against the target questions. No fixed relevance ratio or quota applies. Retain intact originals in an evidence domain (such as `lit://`) when authorized and useful.
2. **Extract with rigor:** capture supported conclusions, applicability conditions, method differences, and genuine disagreements. Cross-check critical numbers, formulas, units, and OCR-derived tables against reliable passages; flag uncertain readings explicitly.
3. **Synthesize around questions:** organize curated nodes (e.g., under `core://`) by the engineering or research questions they answer, linking back to source URIs and section/page locators. One node may compare multiple works; one work may support several nodes.
4. **Verify answerability:** test representative questions against the curated nodes, confirming conditions, citations, unresolved conflicts, and unprocessed material. Judge archival fidelity by counts and byte/hash identity; judge curation by traceable question answering.

## Preserve provenance on import

Associate each imported source node with its original relative path before renaming or rebucketing. Ordinary `import-dir` supports `--source-path-attr <KEY>` to store each new file or directory node's input-root-relative path (with `/` separators, original punctuation, Unicode, and file extension) in `attrs` within the same transaction:

- Off by default; leaves bodies untouched.
- Merges with `--attrs-file`, rejecting `<KEY>` if already present in shared attrs (including `null`).
- Captures names at import time, not names discarded by external preprocessing before `import-dir`. If preprocessing renamed files earlier and originals remain available, pair exported bodies with originals via SHA-256 content hashes rather than guessing from sanitized names.

## Preview and execute ordinary directory import

Ordinary `import-dir` recursively ingests UTF-8 `.md` and `.txt` files, creates empty-body nodes for directories, and skips/reports hidden entries, symlinks, and other extensions (`README` has no special treatment). Existing target URIs and sanitization collisions block the entire import.

```text
# Create a container node if needed, then run a full dry-run preflight
nocturne --db memory.db --namespace project --json create lit:// --title docs --file /tmp/intro.md
nocturne --db memory.db --namespace project --json import-dir ./papers --into lit://docs --sanitize --source-path-attr source_path --dry-run
```

- `--sanitize` keeps Unicode letters, digits, combining marks, `-`, and `_`, collapses runs of other characters into `-`, and trims leading/trailing replacement `-` without inventing collision suffixes.
- Inspect `valid`, `issue_count`, `skipped`, and `mappings` in `--dry-run --json` (for very large directories, redirect or parse the JSON output rather than dumping megabytes into context). When `valid` is `false`, `stdout` carries the full report while `stderr` emits bounded error JSON and exits `2`, `4`, or `5`.
- Choose `--priority` intentionally: `0/1/2/3` trigger stale review after `3/7/14/28` unvisited days (doubling thereafter up to `36,500` days; `999` uses `--days-stale`, default `30`). Keep literature in `lit://` and run `diagnostic --domain core` on curated knowledge instead of inflating literature priorities merely to mute staleness.

```text
nocturne --db memory.db --namespace project --json --batch corpus-import import-dir ./papers --into lit://docs --sanitize --priority 2 --source-path-attr source_path
nocturne --db memory.db --namespace project --json diagnostic --domain core
```

Verify imported `files` and `directories` counts against the preview, sample UTF-8 byte fidelity and `source_path` attributes, and test representative searches. Retain the batch/changeset ID without auto-accepting review.

## Export and restore snapshots with metadata

`export-dir [URI]` exports the namespace (when `URI` is omitted), a domain (`lit://`), or a subtree (`lit://docs`) to a new directory without tracking access or expanding into other aliases/namespaces.

```text
nocturne --db memory.db --namespace project --json export-dir lit://docs --output ./docs-export --with-metadata
nocturne --db restored.db --namespace project --json init
nocturne --db restored.db --namespace project --json import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json --dry-run
nocturne --db restored.db --namespace project --json --batch corpus-restore import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json
```

- Initialize `restored.db` only at an explicitly authorized new path; for deeper subtrees, create external ancestor URIs in the destination first.
- `--with-metadata` writes `nocturne-manifest.json` (v1), preserving exact URIs in the target `--namespace`, node `attrs` (including existing `source_path`), independent and shared edge `name`/`priority`/`disclosure`, and shared node identities with fresh database IDs.
- `--manifest` conflicts with `--into`, `--sanitize`, `--priority`, `--disclosure`, `--attrs-file`, and `--source-path-attr`.
- Manifest snapshots exchange current content and graph metadata; use `backup` to preserve version history, review audits, triggers, and startup presets.
