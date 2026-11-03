# Provenance and adaptation

Upstream: https://github.com/Dataojitori/nocturne_memory

This skill adapts ideas from six earlier Nocturne audit skills:

- **memory-audit:** independent cognitive-evidence and retrieval-topology tracks;
  targeted, random, and diagnostic entry points.
- **memory-audit-belief-duel:** test conflicting claims against observations and
  scope; avoid false reconciliation and preserve counterexamples.
- **memory-audit-dead-data-purge:** distinguish observations from interpretation,
  check practical value, and preserve load-bearing facts. Its historical name
  does not grant permission for Rust CLI purge.
- **memory-audit-discoverability:** evaluate parents, disclosure, aliases, triggers,
  and relative priority against real retrieval situations.
- **memory-audit-node-decomposition:** split by concept and retrieval context,
  not merely length; preserve useful context and independently readable results.
- **memory-audit-pattern-extraction:** compare recurring incidents with earlier
  remedies and their chronology before inventing another summary.

The instructions are rewritten for the current Rust `nocturne` CLI, not the former
Python/MCP tools. This package is self-contained and does not require another skill
at runtime.

Removed or revised: persona and emotional-pressure directives; unapproved writes
and deletions; fixed size/child/priority caps; automatic rewriting; alias-plus-delete
moves; assumptions that pending review means unapplied drafts. Read-only reports,
explicit authorization, shared-node consequences, complete backups, previews, and
namespace boundaries replace those unsafe defaults.

Original material: Copyright (c) 2025 Salem.
Adaptation: Copyright (c) 2026 lll9p.
Both notices and the full MIT grant are retained in the bundled LICENSE.
