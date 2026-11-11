# Memory quality

## Store durable, self-contained evidence

Within the user's persistence authorization, record stable project facts, confirmed decisions, reusable technical findings, and verified corrections:

- State the claim and the exact scope/environment where it holds.
- Record provenance (source document/locator, empirical observation, or user confirmation) and chronology when dates matter.
- Keep material limits, exceptions, and rejected alternatives alongside the rule.
- Separate unconfirmed hypotheses from verified facts instead of blending them into authoritative prose.

Example: “Confirmed for `auth-api` production (2026-05-08 user correction): idle session expiry is 30 minutes; the 7-day setting applies only to refresh-token retention.”

For knowledge curation across literature, write problem-oriented nodes answering target questions with comparative conditions, method differences, disagreements, and passage locators, keeping raw originals as an optional evidence layer when authorized.

## Correct in place rather than duplicating

Read the existing node and parent context first. Check whether apparent contradictions actually reflect different environments, versions, or dates. When a claim is superseded:

- Update the existing `node_uuid` with `--expect-version` (`update` or literal `patch`), preserving context that remains valid.
- Never create a competing node asserting the opposite, append a contradiction below stale text, or tweak `priority` to bury a wrong claim.
- If a node is shared across namespaces (inspectable when authorized via `paths`) and editing its shared body/attrs would exceed current scope, clarify with the user before writing.

## Design plausible retrieval paths

- **Hierarchy:** Place one independently reusable concept under an existing parent whose topic naturally scopes it; avoid hollow wrapper layers or arbitrary child-count/length splits.
- **Disclosure:** State *when* to recall the record (“When changing cache invalidation after schema migrations”), not a generic label (“Cache notes”).
- **Aliases:** Use `alias SOURCE DESTINATION` when a second domain or parent genuinely serves a distinct entry workflow; each explicit alias maintains its own edge `priority` and `disclosure`. Relocate routes atomically with `rename` / `move` rather than alias-plus-unlink.
- **Triggers:** Bind concise, natural synonyms or cross-domain terms via `trigger`. Avoid invented marker tokens.
- **Priority (`0..=999`):** Controls sibling/tie-break ordering, weighted `random` sampling, and `diagnostic` staleness thresholds (`0/1/2/3` = `3/7/14/28` days, then doubling up to `36,500` days; `999` uses `--days-stale`). It is neither a confidence score nor an access control.

## Record decision-relevant gaps once

When an authorized investigation finds a recurring, decision-relevant gap, document it inside the relevant topic node (or a dedicated node only if tracked independently): state what is missing, scope/date checked, sources and query angles examined, partial findings, and the next concrete source to check. Do not log routine empty searches.
