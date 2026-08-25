# Ragavan

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

> [!NOTE]
> Ragavan is in early development. Its first slice manages local repository
> enrollment; development-environment isolation is not implemented yet.

The project's durable product and architectural constraints live in
[Ragavan Foundations](docs/FOUNDATIONS.md).

## Current commands

Repository enrollment is local, applies to every worktree, and never modifies
tracked project files. Ragavan records it in the repository-local Git
configuration that Git shares across worktrees.

```console
ragavan enable
ragavan status
ragavan disable
```

## Development

The pinned Rust toolchain includes the formatting and linting components used by
CI.

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```
