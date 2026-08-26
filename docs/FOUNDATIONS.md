# Ragavan Foundations

## Purpose

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

## Core promise

After one user-level integration and one local repository enrollment, supported
development stacks keep using their normal commands, such as `bun dev`,
`npm start`, and `docker compose up`. Enrollment happens at most once for the
repository and applies to every current and future worktree.

Ragavan owns the orchestration behind those commands. The repository should not
need Ragavan-specific configuration or application changes.

## Non-negotiables

- Zero project configuration for supported development stacks.
- Git remains the source of truth for worktrees and branches.
- Repository enrollment is explicit, local, reversible, and shared by all of its
  worktrees.
- User-level integration is explicit and never repeated per repository or
  worktree.
- Individual worktrees require no setup.
- Normal development commands remain normal development commands.
- Ragavan does not edit application source or package manifests to gain control.
- A worktree's identity survives branch renames and path changes.
- Ragavan state is local, reversible, and kept outside tracked project files.
- Automatic behavior is observable and explainable.
- Unsupported cases fail clearly instead of making dangerous guesses.
- The common path stays quiet, fast, and conceptually small.

## Scope

- A **repository** is the single enrollment boundary shared by its worktrees.
- A **worktree** is an environment boundary and has an identity independent of
  its current branch or path.
- A **service** is an independently running development target within a
  worktree.
- For package runners, an explicit single-package target identifies the service;
  otherwise the nearest package directory does. Runner and script aliases do
  not affect identity.
- Every allocated resource belongs to one service and has one explicit cleanup
  lifecycle.

Stack-specific support may describe required launch adjustments, but it does not
own process execution, terminal behavior, resource allocation, or cleanup.

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

The repository is enrolled once. Every worktree then uses its existing
development commands without additional Ragavan setup.

When Ragavan recognizes the repository, isolation feels automatic. When it does
not, it explains exactly what prevented isolation and what capability is missing.
