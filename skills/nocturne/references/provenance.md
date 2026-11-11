# Provenance

Adapted from the system prompts and MCP tool reminders of
[Dataojitori/nocturne_memory](https://github.com/Dataojitori/nocturne_memory),
originally copyright (c) 2025 Salem, under the MIT license.

This skill rewrites those workflows for the standalone Rust `nocturne` CLI (schema v3):
explicit `--db` and `--namespace` authorization, evidence-first retrieval, strict literal
patching and CAS updates, independent alias edge metadata, and clean temporary-file
handling. Legacy persona mandates, automatic background persistence, mandatory startup
protocols, and MCP calls are omitted.
Current adaptations are copyright (c) 2026 lll9p. Full notices and MIT terms ship in
this skill's `LICENSE`.

All operational references live in this directory; no upstream Python/MCP runtime is
used. Installing this skill via [vercel-labs/skills](https://github.com/vercel-labs/skills)
does not install the `nocturne` binary, initialize databases, or grant execution permissions.
