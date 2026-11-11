# Symptom-selected audit playbooks

Load only the section matching the observed symptom. Every playbook produces an evidence-backed proposal first, never unilateral edits. Command syntax lives in [cli-audit.md](cli-audit.md).

## Conflicts and prior calibration

1. Read both nodes, their supporting examples, relevant parents, and `history`.
2. State the exact conflict with URIs, `version` IDs, and event/source dates.
3. Separate verified observations from generalizations, preferences, and hypotheses.
4. Test whether environment, version, or chronology explains the divergence. If a genuine contradiction remains, keep it explicit instead of inventing a false synthesis.
5. One verified counterexample refutes a universal rule without proving the opposite universal; propose a narrower scope condition or an explicit open question.
6. Flag downstream nodes that rely on the superseded assumption while preserving verified underlying facts.

## Evidence and value

Judge against the collection's purpose:
- **Source archive:** verify UTF-8 fidelity and provenance mappings (`attrs` / `source_path`); do not demand summarizing raw sources.
- **Curated knowledge:** check whether target questions are answered with applicability conditions, method comparisons, visible disagreements, and passage locators, and note unprocessed sources.

Ask what concrete decision each curated claim informs and what evidence supports its causal explanation. Lack of recent access (`stale_nodes`) is a prompt to check currency, not proof that reference data or rare failure procedures are worthless. Propose narrowing unsupported claims, gathering a missing observation, or consolidating true duplicates—never treating “low recent access” as automatic permission to unlink or purge.

## Repeated failures and duplication

1. Compare incident chronology against when the existing note or remedy was written (do not confuse database `created_at` with incident dates).
2. Reconstruct the pre-action lookup path: which parent, `boot` list, `disclosure`, or `search` / `trigger` query would an agent naturally hit before acting?
3. Distinguish four root causes: (a) advice was unreachable from the pre-action context, (b) advice was retrieved but too vague, (c) the remedy itself was wrong, or (d) the new failure had a different mechanism.
4. Propose a targeted causal fix—an `alias SOURCE DESTINATION` at the natural entry parent, an observable `disclosure`, a `trigger` synonym, or a corrected technical procedure—plus a replay query to test it. Do not add generic “be careful next time” summaries or personality commentary.
5. Check `node_uuid` first when evaluating duplicates: two URIs sharing one `node_uuid` are already aliases, not duplicate bodies. Consolidate separate nodes only when they share one maintenance lifecycle, preserving distinct incident evidence.

## Retrieval topology

Start from realistic user questions or impending engineering actions within scope:

1. Replay representative questions with bounded `search` and natural `children` / `tree` navigation (`read --no-track`), deduplicating by `node_uuid`.
2. Record where discovery fails rather than treating empty `disclosure` or `trigger` fields on raw literature as defects. Audit curated decision nodes separately from archival sources.
3. Propose repairs only for observed discovery gaps:
   - **Disclosure:** phrase as an observable trigger (“Before retrying a timed-out deployment”), not a topic label.
   - **Triggers:** bind verified domain synonyms (batch multiple CJK terms in one `trigger edit` call; remove noisy false-positive terms only with approval).
   - **Aliases & moves:** add `alias SOURCE DESTINATION` when two entry contexts need the same maintained body with context-specific edge `priority` / `disclosure` (check `paths --node <UUID>` only if editing auto-derived child edges). Use atomic `move` / `rename` to relocate misplaced subtrees.
   - **Broken parent routes:** if `diagnostic` reports `broken_parent_routes`, inspect whether the missing intermediate container should be created or the subpath moved.
4. After approved repairs, replay the representative questions and report actual reachability and remaining gaps.

## Decomposition

Distinguish **chapter navigation** (making sections of a long reference addressable) from **knowledge synthesis** (answering cross-source questions). Length alone never mandates splitting; weigh single-read coherence against multi-lookup overhead.

When users repeatedly need specific sections of a large work:
- **Lightweight index:** keep the intact source node and add a navigational node with section headings and locators.
- **Selective chapter nodes:** extract high-traffic chapters as child nodes citing the intact original URI, keeping the full source preserved unless authorized otherwise. Remember that multiple excerpts from one work share provenance and do not count as independent corroboration.

When a curated node mixes genuinely independent concepts or update lifecycles, propose exact split boundaries, target URIs, disclosures, and which facts stay in each node so every resulting record stands on its own.
