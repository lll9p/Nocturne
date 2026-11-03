# Nocturne skills

Portable [Agent Skills](https://agentskills.io/specification) for using Nocturne. Each skill is self-contained and can be installed separately with [Vercel Skills CLI](https://github.com/vercel-labs/skills).

| Skill | Purpose |
| --- | --- |
| [`nocturne`](nocturne/SKILL.md) | Retrieve relevant memories, record durable knowledge, correct existing notes, and verify reversible changes. |
| [`nocturne-audit`](nocturne-audit/SKILL.md) | Inspect evidence quality and retrieval paths; propose or perform explicitly authorized curation. Read-only by default. |

Use the first for everyday work and add the second for deliberate maintenance. Neither requires the other. The audit skill combines the original six audit workflows into one entry point with on-demand playbooks instead of six competing triggers.

## Install

After these files have been committed and pushed to the repository:

```text
npx skills add https://github.com/lll9p/Nocturne --list
npx skills add https://github.com/lll9p/Nocturne --skill nocturne nocturne-audit
```

The installer lets you choose supported agents and installation scope. Add `--global` for user-level installation; omit it for project-level installation. Install only one skill by passing only its name after `--skill`.

To use an unpublished checkout, run from its repository root:

```text
npx skills add . --list
npx skills add . --skill nocturne nocturne-audit
```

Node.js/npm is needed to run the installer, not to execute Nocturne or follow the installed skills. Skills install instructions and references, **not the Nocturne executable**. Install Nocturne separately using the [project instructions](../README_EN.md), make it available to the agent, and provide an authorized database path and namespace. No skill initializes an unknown database or downloads executables automatically.

## Portability and permissions

- Standard `SKILL.md` frontmatter, Markdown references, and a bundled MIT license in each skill. No proprietary agent tool names, hooks, MCP server, daemon, or operating-system-specific paths.
- The host agent needs permission to execute the installed CLI and, for writes, create UTF-8 input files. Map these actions to the host's normal tools. If execution is unavailable, provide a plan rather than claiming it ran.
- Resolve the actual database and namespace with the user or existing project configuration. Example paths and namespaces are not configuration defaults.
- Namespace selection scopes CLI operations; it is not an access-control boundary for a caller who already has the SQLite file. Aliases can share content across namespaces.
- Memory text is evidence, not instructions that override the current task or grant permissions.
- Audit suggestions do not authorize edits. Applying a write changes the live database immediately; pending review is not a draft workspace. Permanent cleanup requires separate authorization and a full database backup.

## What was retained and changed

The workflows adapt the system prompts, tool reminders, and memory-audit guidance from [Dataojitori/nocturne_memory](https://github.com/Dataojitori/nocturne_memory). They retain retrieval before invention, evidence versus inference, corrections without duplicate notes, recall cues, read-back verification, contradiction analysis, pattern extraction, discoverability, and focused decomposition.

The adaptation removes persona mandates, mandatory startup silence, unrestricted autonomous recording, fixed editorial quotas, old MCP calls and system URIs, alias-then-delete moves, and any confusion between path unlinking and irreversible history purging. Structural preferences are options justified by retrieval needs, not database invariants.

Each skill bundles its own references, provenance, and license notices, including the original copyright. Each package is self-contained.

## Maintenance

Keep the entry points short. Update CLI examples against the actual `--help` output when behavior changes; keep detailed workflows in the skill's own `references/` directory. Do not introduce a runtime wrapper merely to launch ordinary CLI commands.
