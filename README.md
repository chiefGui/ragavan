# Ragavan

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

> [!NOTE]
> Ragavan is in early development. The current vertical slice supports
> PowerShell, Bun, and simple Vite development scripts. It is a product proof,
> not broad stack support yet.

The project's durable product and architectural constraints live in
[Ragavan Foundations](docs/FOUNDATIONS.md).

## Try the current slice

Repository enrollment is local, applies to every worktree, and never modifies
tracked project files. Ragavan records it in the repository-local Git
configuration that Git shares across worktrees.

Install the local build and load the PowerShell hook for the current shell:

```powershell
cargo install --path .
Invoke-Expression (ragavan hook powershell | Out-String)
```

Then enable any worktree once and keep using the existing command:

```powershell
ragavan enable
bun dev
```

For a package with a simple Vite script such as `"dev": "vite"`, Ragavan adds a
stable worktree-specific port and requires Vite to use it. `bun run dev` works
as well. Every other Bun command passes through unchanged, as do Bun commands
outside enabled repositories.

The hook currently lasts for one PowerShell session. Port selection is stable,
but occupied-port and hash-collision coordination is intentionally left for the
next slice. Vite fails clearly instead of silently selecting a different port.

Enrollment can be inspected or reversed at any time:

```powershell
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
