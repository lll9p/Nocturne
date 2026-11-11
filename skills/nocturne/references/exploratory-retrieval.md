# Exploratory retrieval

Use this guide when surveying methods, alternatives, mechanisms, or failure conditions—not for single-fact lookups already answered by verified context. Always pass `read --no-track` during investigation.

## Four-step loop

1. **Frame distinct search angles.** Derive object terms, mechanism/method terms, and boundary or counterevidence terms from the question. Do not assume near-synonyms refer to the same mechanism before checking context.
2. **Probe with small pages.** Run `search <terms> --limit 5` (add `--with-total` only when deciding whether to page or narrow a large hit set). Remember engine boundaries:
   - Default FTS tokenizes CJK (`jieba`) and Unicode alphanumeric words (Latin, Greek, Cyrillic, combining marks).
   - Use explicit `--contains` when probing a short stem embedded inside longer compound terms or exact notation fragments; `--contains` is ASCII case-insensitive only.
3. **Read originals and branch from terminology.** Open promising URIs with `read --no-track`, inspect applicability conditions and limitations, and branch into newly discovered terms, parents, or children.
   - Deduplicate by `node_uuid` across aliases and queries.
   - Check provenance (`attrs` / source locators): multiple chapters or summaries from the same work have distinct `node_uuid`s without constituting independent corroboration, whereas distinct experiments within one paper can provide separate evidence.
4. **Stop on question coverage, not a numeric quota.** Stop when major alternatives, operating conditions, and material contradictions are either supported by source passages or isolated as explicit evidence gaps. Neither five reads nor a clean first page proves exhaustiveness.

When the task includes authorized knowledge curation, synthesize the verified findings into question-oriented nodes with source/passage locators and explicit conditions per [corpus workflows](corpus-workflows.md).

## Proportional reporting

Present source-linked answers with their conditions first, followed by a brief note naming the directions checked, the scope read, and any unresolved conflicts or unexamined gaps (“not found in checked sources” rather than “does not exist”).
