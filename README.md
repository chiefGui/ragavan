# Ragavan

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

> [!NOTE]
> Ragavan currently contains only its foundations and workspace scaffold. It
> does not have user-facing behavior yet.

The project's durable product and architectural constraints live in
[Ragavan Foundations](docs/FOUNDATIONS.md).

## Development

The repository is a Rust workspace. Its pinned toolchain includes the formatting
and linting components used by CI.

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```
