# Ragavan Foundations

## Purpose

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

## Core promise

After installing Ragavan and running `ragavan init`, supported repositories keep
using their normal commands, such as `bun dev`, `npm start`, and
`docker compose up`.

Ragavan owns the orchestration behind those commands. The repository should not
need Ragavan-specific configuration or application changes.

## Non-negotiables

- Zero project configuration for supported development stacks.
- Git remains the source of truth for worktrees and branches.
- Normal development commands remain normal development commands.
- Ragavan does not edit application source or package manifests to gain control.
- A worktree's identity survives branch renames and path changes.
- Ragavan state is local, reversible, and kept outside tracked project files.
- Automatic behavior is observable and explainable.
- Unsupported cases fail clearly instead of making dangerous guesses.
- The common path stays quiet, fast, and conceptually small.

## Conceptual model

```text
Project -> Worktree -> Session -> Resources
```

- A **project** represents one Git repository and its shared worktree state.
- A **worktree** has a stable identity independent of its current branch or path.
- A **session** is a supervised command running within a worktree context.
- **Resources** are leases owned by that session, such as ports, URLs, process
  groups, and container namespaces.

Adapters recognize supported development stacks and describe the environment or
command adjustments they require. Adapters do not execute processes themselves.
The runtime remains the sole owner of process execution, terminal behavior,
resource allocation, and cleanup.

## Initial proof

Two worktrees of the same JavaScript repository can run the same `bun dev`
command simultaneously. Each receives a stable identity and URL, neither
requires repository changes, and both are cleaned up correctly when stopped.

This is a proof of the product promise, not a commitment to a JavaScript-only
architecture.

## Not initially

- Universal isolation for arbitrary binaries that ignore all supported
  conventions.
- Automatic cloning or migration of databases.
- Cloud environment orchestration.
- A graphical interface.
- A replacement for Git's branch or worktree workflow.
- A general-purpose task runner or package manager.
- A configuration language for cases Ragavan has not learned to recognize.

## Success

The user learns `ragavan init` and nothing else.

When Ragavan recognizes the repository, isolation feels automatic. When it does
not, it explains exactly what prevented isolation and what capability is missing.

