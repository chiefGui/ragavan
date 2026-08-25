---
name: code-quality
description: Use only when explicitly requested to improve code quality.
---

# Code

## Minimize total machinery

- Required behavior, invariants, and boundary contracts set the floor; among correct designs satisfying them, choose less code, fewer concepts, types, files, layers, paths, states, and transitions.
- Add a structural element only when it owns a required distinction that no existing owner can absorb coherently.
- Prefer deletion, consolidation, and direct implementation; add indirection or coordination only when required by an owned boundary, variation, or workflow.

## Distrust precedent

- Treat existing code as context, not authority; proximity, frequency, age, passing tests, and successful execution do not justify reuse.
- When implementations disagree, derive one model from required behavior, invariants, and ownership; never combine models or preserve parallel representations for consistency.
- Preserve required external contracts, not internal structure.
- Repair a defective authoritative path before extending it; never preserve it by adding a parallel or compensating implementation.

## Keep vocabulary small

- Use one canonical term per concept and one concept per term within each model.
- Add a concept only when a required distinction changes rules, ownership, identity, quantity, lifecycle, or valid operations; otherwise remove it rather than rename it.
- A distinction does not by itself justify a type, abstraction, or file.
- Name by owned meaning; use a mechanism name only when the mechanism is itself the contract.
- Treat a vague or mechanical name as unresolved meaning or ownership; repair the design instead of elaborating the name.
- Translate genuinely different meanings at their boundary; never merge them or invent a hybrid model.
- Represent a required distinction in the smallest form that preserves its meaning; never encode distinct states through sentinels or interacting flags.

## Own behavior once

- Give every rule, source of truth, mutable state, workflow, translation, resource, and lifecycle one authoritative owner.
- Keep behavior with the owner of the rule, state, resource, or lifecycle it governs; never invent an owner merely to connect existing owners.
- Keep each invariant and every mutation governed by it at the same owner.
- Let callers request outcomes through the owner's contract; never make them coordinate its internals.
- Place cross-owner sequencing with the owner of the resulting outcome; participants expose intent and retain their invariants.
- Create a workflow owner only when sequencing has independent rules over ordering, atomicity, recovery, or completion; routing and synchronization alone are not ownership.
- Derive data at use; store it only when required by an invariant or measured cost, with one authoritative source and one synchronization owner.

## Keep one authoritative path

- Make mechanisms depend on the rules they carry out, never the reverse.
- Keep dependencies between owners acyclic.
- Organize structure around owned concepts and boundaries, not technical steps.
- Maintain one path from intent to effect; extend, simplify, or replace it, but never bypass, shadow, or duplicate it.
- Put a conceptual change at its owner and refactor every affected path and boundary required to restore that ownership.
- Remove every layer, indirection, hook, or path that owns no current rule, boundary translation, lifecycle, or demonstrated variation.

## Colocate by default

- Keep implementation, including single-use details, inside the smallest scope that fully owns it.
- Extract only an independently governed rule, invariant, contract, state, translation, or lifecycle; reuse, file size, and shorter scopes alone do not justify extraction.
- Create a file only for an independently owned concept; keep cohesive ownership together.

## Protect boundaries

- Expose operations that express intent, not internal representation.
- Translate representations at their boundary and reject malformed input; let the rule's owner reject invalid meaning.
- Preserve only required boundary contracts; repair incidental exposure instead of treating it as a contract.
- Translate required compatibility once into the canonical model and keep it out of internals.

## Make variants additive

- Model only demonstrated variation.
- Keep a variation closed only when the domain defines an intrinsically finite set; handle it exhaustively at one owner.
- When the domain does not define an intrinsically finite set, the second case establishes an extension axis: put every case behind the smallest stable seam owned by the consumer before completing it.
- After the axis exists, let a new variant change only variant-specific behavior and composition, never the stable workflow, existing variants, or unrelated owners.
- Select variants once where they are composed; never scatter variant checks or duplicate the workflow.
- Use the smallest seam that satisfies these rules; a seam does not imply a new type.
- Require every variant to preserve the contract's meaning and substitutability.

## Make state, lifetimes, and effects explicit

- Expose state changes as named transitions; never let callers compose raw mutations.
- Prevent invalid creation and reject invalid transitions at the state owner.
- Represent lifecycle phases as states, not flag combinations.
- Keep correctness-sensitive ordering explicit in one workflow.
- Decide before initiating effects; keep effect initiation explicit in the primary control flow.
- Never let work or resources outlive their owner without explicit ownership transfer.
- At concurrency boundaries, define mutation authority, completion, interruption, and cleanup.
- Never hide effects, costly work, or unbounded work behind operations that appear passive or cheap.

## Preserve failure meaning

- Put an expected negative outcome in the contract only when callers require distinct handling; never model defects as expected outcomes.
- Preserve failure identity and cause across boundaries.
- Intercept a failure only to recover, translate it, or add actionable context.
- Never swallow failure, return plausible success, or add silent retry, fallback, or degradation.

## Keep code direct

- Structure code so its owner, boundary crossings, and primary path are apparent without tracing unrelated files.
- Write the primary path in execution order; keep local details nearby and trivial one-use plumbing inline.
- Make call sites express intent in the canonical vocabulary.
- Let semantics determine form; never introduce structure merely for symmetry.
- Deduplicate rules at their owner, not incidental syntax.
- Use comments for constraints, intent, and non-obvious tradeoffs; never narrate the implementation.

## Optimize from evidence

- Optimize measured bottlenecks; never complicate ordinary paths for hypothetical cost.
- Give every cache a bound and invalidation rule; bound queues and concurrent work.

## Test contracts

- Make behavior controllable and observable through the caller contract; repair ownership or the contract instead of adding test-only access or indirection.
- Make each test name one behavior and assert only its observable outcomes, invariants, transitions, and effects.
- Never assert internal control flow; assert calls, ordering, or representation only when they are the contract.
- Hardcode only test-owned inputs and literals whose exact value is the contract.
- Read externally owned data and configuration from their authoritative source; never copy their current values into expectations.
- Use distinct test-owned values so an incorrect input, mapping, or target cannot pass.
- Build expected results from the contract and test inputs; never invoke or copy the implementation under test.
- Replace dependencies only at design boundaries; excessive setup, internal access, or pervasive replacement means the boundary is wrong.
- Control nondeterminism at its boundary; never rely on delays, shared state, or test order.
- Never change an expectation merely to match the implementation; change it only when the named contract changes.
