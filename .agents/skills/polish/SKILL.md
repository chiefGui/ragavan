---
name: polish
description: Use only when explicitly requested to polish this task's code changes.
---

# Polish

## Establish the target change set

- Identify agent-authored edits from this task's conversation, summaries, edit history, and delegated work; never infer authorship from repository state alone.
- Inventory every authored hunk and added, deleted, renamed, or generated file, then capture the initial scoped diff for comparison.
- Polish that inventory and every affected part of the same requested behavior required to leave one coherent result.
- Treat the original diff as a draft, not a design constraint; rewrite or remove affected existing code when that produces the better result.
- Preserve every unrelated or unattributable hunk, including pre-existing, user, other-agent, dirty, staged, and concurrent changes.
- If multiple target change sets remain plausible, ask which request to polish.
- If the target change set is empty, report `Changes: none` and `Verification: Not run - no target changes to verify`, then stop without inspecting unrelated worktree changes.

## Reconstruct intent

- Read applicable project instructions and code guidelines completely.
- Re-read the request and identify required behavior, contracts, invariants, exclusions, and observable effects.
- Trace concrete affected paths from each entry point to their authoritative owners and outward representations, including callers whose behavior can change, boundaries, lifetimes, tests, dependency directions, sibling or parallel paths, and evidenced hot paths.
- Treat the current implementation and repository precedent as evidence, not constraints.

## Challenge the result

Run the checklist in order against the resulting behavior, not merely the added lines.

- [ ] Treat applicable project instructions and code guidelines as authoritative.
- [ ] Audit correctness and security across affected callers, boundaries, failure paths, state transitions, lifetimes, concurrency, resource ownership, authorization, and externally observed representations.
- [ ] Add or repair contract-level tests for changed behavior and fixed defects; never weaken expectations to match the implementation.
- [ ] For every added or changed file, type, abstraction, state, branch, option, hook, translation, name, and copied pattern, verify its necessity, ownership, placement, and compliance with the applicable guidelines.
- [ ] After correctness is established, attempt to remove, merge, inline, colocate, or rewrite structural elements until the result is the smallest coherent design allowed by the contracts and guidelines.
- [ ] Fix regressions, unbounded work, and evidenced waste in affected hot paths; do not micro-optimize.
- [ ] Remove residue introduced or made obsolete by the task's changes, including dead code, stale comments, scaffolding, obsolete paths, compatibility residue, and test-only production hooks.

Passing validation establishes behavior, not design quality.

## Repair completely

- Make every repair already authorized by the request; never use a flag instead of making an authorized repair.
- If a required repair needs new authority, external coordination, or a product decision, leave the defect unresolved and request the missing input.
- Immediately before each edit, re-read the exact target content used to construct the patch; if it changed, recompute the edit from the fresh version.
- After each batch, inspect the resulting hunks against the fresh pre-edit content and confirm that no concurrent or unowned change was reverted.
- If overlapping ownership or intent cannot be separated safely, do not edit the overlap; report the required coordination or authority.
- Work in coherent edit batches and recheck affected checklist items after each batch.
- After the planned repairs, review the complete affected diff and run one complete audit.
- Repeat only when that audit finds a new material in-scope defect.
- Finish when no authorized material repair remains; never cycle between equivalent designs.

## Record unresolved material flags

A flag records a concrete material defect in the affected behavior that remains unresolved when polish ends. Repaired defects belong only in `Before vs. After`, never in `Flags`.

- Assign `F1`, `F2`, and subsequent IDs in discovery order among unresolved findings.
- Create one flag per root cause and group its related evidence.
- Report every unresolved material flag; never cap or hide the list.
- Exclude speculation, subjective preference, inconsequential observations, and incidental issues outside the affected behavior.
- Leave a flag open only when its repair requires new authority, information, external work, or a product decision.
- Mark a flag deferred only when the user explicitly defers it; record the unfinished work.

### Severity

- Critical: credible risk of a security or privacy breach, data loss or corruption, process-wide crash, or unusable or materially incorrect primary behavior.
- High: likely material failure, regression, bounded realistic crash, or sustained design, performance, or test risk.
- Medium: bounded defect with a clear recurring maintenance, performance, test, or usability cost.
- Low: localized defect with concrete value; never use Low for taste or optional cleanup.

## Verify the result

- Run the narrowest meaningful verification for the changed behavior.
- Record every check actually run and every materially relevant check deliberately omitted, with its result or reason; never enumerate checks outside the affected behavior.
- Treat a failure proven unrelated to the polished changes as verification evidence, not a defect in the result.
- If correctness cannot be established because required verification fails or cannot run, report what blocks completion.
- Put verification or environment blockers only in `Verification`; never invent a defect flag for them.

## Report the outcome

Do not report an overall status.

### Use direct language

- Make every outcome and flag understandable at first read.
- Name the actual result or defect with the simplest established engineering term that fits.
- State concrete behavior and consequence; add detail only when it changes understanding.
- Never invent taxonomy, use euphemisms, stack qualifiers, or turn a simple fact into architectural prose.
- Never narrate the audit or repeat the same fact across the title and its details.

Include `Before vs. After` when polish changed the result. Group entries by material outcome and omit mechanical edits.

```md
Before vs. After:

- <plain outcome>
  - Before: <old behavior or structure>
  - After: <new behavior or structure>
```

Report only unresolved flags, ordered `Critical`, `High`, `Medium`, then `Low`, with discovery order preserved within each severity.

```md
Flags:

- F1 | High | <plain defect name>
  - Finding: <direct evidence>
  - Impact: <credible consequence>
  - State: Open - <required action, authority, information, or decision>

- F2 | Medium | <plain defect name>
  - Finding: <direct evidence>
  - Impact: <credible consequence>
  - State: Deferred - User explicitly deferred <repair>; Remaining: <unfinished work>
```

When no material defects remain unresolved:

```md
Flags:

- None.
```

Always report verification.

```md
Verification:

- <check>: <result>
- <materially relevant omitted check>: <reason>
```
