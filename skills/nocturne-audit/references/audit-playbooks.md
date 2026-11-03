# Symptom-selected audit playbooks

Read only the relevant sections. Every section produces a proposal first, not
permission to change data. Use the command syntax in [cli-audit.md](cli-audit.md).

## Conflicts and prior calibration

1. Read both claims, their supporting cases, relevant parents, and history.
2. State the apparent contradiction precisely, with URIs and evidence dates.
3. Separate verified observations from generalizations, preferences, and hypotheses.
4. Test whether scope or time differences explain it. If not, preserve the real
   conflict rather than asserting that both claims are somehow true.
5. A reliable counterexample can refute an absolute claim without establishing
   its opposite. Propose a narrower boundary or an unresolved question when warranted.
6. Identify dependent conclusions that need review, without assuming all descendants
   are invalid. Preserve facts even if the interpretation they supported is rejected.

For a single outdated prior, state its original conditions, what changed, and what
future decision would differ. Do not rewrite simply to echo the latest feedback.
Report uncertainty when source reliability or temporal ordering cannot be established.

## Evidence and value

Ask what decision the body informs and what evidence gives it that role.
Check quotations, event dates, provenance, outcomes, and the mechanism behind advice.
Label unsupported explanations; avoid replacing them with another confident story.
A missing recent access is not proof of irrelevance, falsity, or expiration.
Reference material, historical evidence, and rare safety procedures may deserve retention.

Recommend retaining coherent source records, narrowing unsupported interpretations,
or collecting a specific missing observation. Where text seems redundant, compare
unique facts and ownership before suggesting consolidation. “No behavioral value”
is an audit hypothesis, not authorization for unlink or permanent deletion.

## Repeated failures and duplication

1. List incident URIs and actual event times; find any earlier attempted remedy.
2. Verify whether the remedy existed before each later incident. Created/edited
   timestamps alone may not establish when the underlying events occurred.
3. Reconstruct the retrieval path before the repeated action: what parent would be
   opened, what disclosure would be visible, and which triggers could be encountered?
4. Distinguish a remedy that was never retrieved, advice that was retrieved but
   ambiguous, an incorrect remedy, and an unrelated failure with similar wording.
   Do not infer any of these without evidence; list competing explanations if needed.
5. Suggest a causal repair: an alias at the real entry path, an observable disclosure,
   a precise technical procedure, or a revised claim supported by the incident evidence.

Do not manufacture personality diagnoses from repeated technical mistakes.
Do not write another generic “remember next time” summary when the useful lesson
already exists but its trigger path is missing. Propose a concrete replay test:
from the pre-error context, can a reader reach the right advice before acting?

Compare duplicate candidates by node UUID first. Two paths to one UUID are aliases,
not duplicate bodies needing consolidation. For separate nodes, preserve unique
supporting incidents, source ownership, and independent update requirements.
A shared maintained lesson may suit an alias; independent or divergent claims may not.
Consolidation proposals should identify exactly which facts survive and where.

## Retrieval topology

Start from the actual input or imminent action, not an ideal taxonomy.
Inspect the parents a reader would naturally visit and the disclosure shown there.
A disclosure should describe when to read, not just summarize the body.
Prefer observable signals such as “Before retrying a deployment after timeout”
over “When I realize I am making a mistake.” Provide proposed wording in the report.

Check lexical triggers for useful cross-domain associations and false positives.
A noisy term may need narrowing or removal, but only after authorization.
Use aliases for multiple entry points to the same maintained content. Check which
routes share an `edge_id` before proposing context-specific disclosure and priority;
changing shared edge metadata affects all its routes. Explain shared-body consequences.
Do not clone text merely to place it under another parent.

Priority is relative retrieval ordering and affects sampling/staleness signals.
Do not tune it solely to silence diagnostics. No universal priority quota or child
count cap applies. A busy parent warrants investigation only when its actual
retrieval cost or relevance is poor. Use atomic move/rename for relocation.

## Decomposition

Read the whole body. Ask whether parts have genuinely different retrieval contexts,
sources, or update lifecycles. Length alone does not justify splitting.
Compare the cost of reading a coherent reference with the cost of multiple lookups.

If splitting is justified, propose concept boundaries, target paths, disclosures,
and the supporting facts each result retains. Preserve independently useful parent
content when it serves ordinary cases; a navigational index may also be intentional,
so do not impose a universal ban or generate hollow parents just to tidy the tree.
Ensure each proposed result is understandable without hidden conversational context.

A plan to split is not permission to create, rewrite, unlink, or purge nodes.
Validate the proposed retrieval flow before asking for approval, then read back
all approved results and compare against the evidence-preservation plan.
