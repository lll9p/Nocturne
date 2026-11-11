# Release acceptance tasks

```text
cargo xtask portable-smoke [--binary PATH]
cargo xtask large-library-smoke [--binary PATH] [--output PATH]
```

Build the application in release mode explicitly before running. The default executable is repository `target/release/nocturne[.exe]`; a missing executable is an error, not a request to build or use debug. `--binary` selects an already built executable. Run `cargo xtask --help` for help. There is no Python runtime, old executable baseline, legacy import, or schema upgrade fixture.

Both tasks copy only the executable into a temporary directory outside the repository, clear PATH and Nocturne environment overrides, use fresh processes, and operate exclusively on temporary databases. Success prints JSON; any assertion or tool failure exits unsuccessfully. `--output` writes the same JSON report (and explicitly replaces that requested report file). Timings are wall-clock milliseconds including process start. On Windows, labeled commands measure peak working set through GetProcessMemoryInfo using a duplicated process handle. Other platforms report `null`, never an estimate.

## Coverage map

`portable.rs` covers the single-executable workflow:

- Chinese creation, machine/human read, review timestamp, aliases, trigger search, patch, detailed rollback conflict and rollback.
- Boot, rename/move and rollback, orphan show/restore and rollback; capacity arithmetic.
- Direct 99-node SQLite fixture using immutable `contents` and `memories.content_id`, plus the original node: 100-node read/search/write and process-start medians, seven cold processes each; reindex, doctor, backup doctor.
- Compound content/metadata writes, trigger edits, preset copy/activation, namespace privacy, diagnostic/random, database-free diff, deprecated-version purge and log retention cleanup.
- Unicode domains, shared node attributes, reduced reads, AND/OR and attribute search, attribute rollback.
- Naming preflight and sanitization for three problematic names, warnings and separate edge/node metadata, batch rollback.
- Literal short Chinese contains, exact UTF-8 bytes including NUL/CRLF/decomposed Unicode/no trailing newline, no-op update, readable review diffs, namespace export byte identity and unchanged reviews.
- 59 individual creates versus one batch import: timings, dry-run nonmutation, search/index offset identity, bounded graph, review counts, attributes, rollback cleanup, doctor and vacuum review preservation.
- Subtree export with `--with-metadata`, manifest dry-run and successful import into the current non-default namespace of a fresh database; body/metadata identity, excluded sibling, source review preservation and destination doctor.

`large.rs` retains the synthetic fixture generation: 1,578 papers, 35 buckets, approximately 110 MiB of valid UTF-8, one 3.6 MB paper and 59 curated nodes. It checks import counts; three search queries; 1,000-result search and offset identity; literal contains; index page identity; bounded tree; SHA-256 verification of all 1,578 exported papers; backup doctor and business-table fingerprint; identical search order on backup and after vacuum; vacuum/doctor and unchanged business data. Reports include source and executable sizes, executable SHA-256, SQLite allocated/free bytes and per-object dbstat capacity, and labeled process timings. Physical categories partition dbstat bytes into body (including the immutable `contents` table and memory indexes), audit, search, FTS and other objects. Logical UTF-8 body bytes are measured with SQLite BLOB lengths: stored immutable bodies, expanded memory versions, and expanded before/after audit references (reference totals can count a body multiple times). Fingerprints include `contents`, expanded `memory_rows`, and expanded before/after audit bodies; they compare backup/vacuum within the current schema, not row hashes across schema versions.

Additional cross-platform failure checks preserve existing backup/export outputs and confirm that an invalid UTF-8 import leaves the business-table fingerprint unchanged. Fingerprints stream typed, length-delimited SQLite values through SHA-256 rather than buffering the library.

## Test scope

These tasks validate the actual executable. There are no separate tests for the fixture generator, SHA-256 dependency, median helper, or removed command-line flags. The normal regression suite keeps only risks not already demonstrated by the portable workflow; see the risk-specific comments in the [test sources](../tests/). The large-library task remains an optional scale check, not another copy of the ordinary test suite.
