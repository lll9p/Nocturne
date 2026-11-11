# Provenance and adaptation

Upstream: https://github.com/Dataojitori/nocturne_memory

This skill consolidates and adapts six earlier Nocturne audit workflows into one entry point with symptom-routed playbooks:

- **memory-audit:** dual evidence-quality and retrieval-topology tracks; targeted, random, and diagnostic entry points.
- **memory-audit-belief-duel:** testing conflicting claims against observations, chronology, and scope without false reconciliation.
- **memory-audit-dead-data-purge:** separating verified facts from unsupported stories while preserving load-bearing evidence (its historical name never authorizes irreversible CLI `purge`).
- **memory-audit-discoverability:** replaying real questions against parents, disclosures, aliases, triggers, `boot`, and relative priority.
- **memory-audit-node-decomposition:** splitting by retrieval context and lifecycle rather than byte count alone.
- **memory-audit-pattern-extraction:** tracing recurring failures to missing retrieval paths or faulty remedies before writing another summary.

Rewritten for the standalone Rust `nocturne` CLI (schema v3): explicit `--db` and `--namespace` scope, `--no-track` across `read`/`random`/`boot`, independent alias edge metadata, unified `stderr` error JSON, complete `backup` prerequisites, and explicit approval gates. Persona directives, autonomous unapproved edits, fixed size/child quotas, and routine edit interception are removed.

Original material: Copyright (c) 2025 Salem.
Adaptation: Copyright (c) 2026 lll9p.
Both notices and the full MIT license ship in the bundled `LICENSE`.
