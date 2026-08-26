# Ragavan

Ragavan makes concurrent Git worktrees behave like isolated development
environments without changing the commands developers already use.

> [!NOTE]
> Ragavan is in early development. The current vertical slice supports
> PowerShell, Bun, and simple Vite development scripts. It is a product proof,
> not broad stack support yet.

The project's durable product and architectural constraints live in
[Ragavan Foundations](docs/FOUNDATIONS.md).

## Install on Windows

Windows releases are distributed as self-contained Chocolatey packages. The
package puts `ragavan` on `PATH` and installs its PowerShell integration for the
user running Chocolatey:

```powershell
choco install ragavan -y
```

Open a new PowerShell session after installation or upgrade. Future versions
use the ordinary Chocolatey workflow; no Ragavan-specific updater or server is
needed:

```powershell
choco upgrade ragavan -y
choco uninstall ragavan -y
```

The first release remains unavailable until its package passes Chocolatey
community moderation.

## Try the current slice

Repository enrollment is local, applies to every worktree, and never modifies
tracked project files. Ragavan records it in the repository-local Git
configuration that Git shares across worktrees.

Install the local build, then install its PowerShell integration once:

```powershell
cargo install --path crates/ragavan
ragavan install
```

Ragavan detects the current supported shell and adds one managed block to its
user profile. If automatic detection is unavailable, select PowerShell
explicitly with `ragavan install powershell`. Open a new PowerShell session to
load the integration automatically. To use the session that performed the
installation immediately, run:

```powershell
Invoke-Expression (ragavan hook powershell | Out-String)
```

Then enable any worktree once and keep using the existing command:

```powershell
ragavan enable
bun dev
```

For a package with a simple Vite script such as `"dev": "vite"`, Ragavan adds a
stable worktree-specific port and requires Vite to use it. Before Bun starts,
Ragavan skips occupied ports and coordinates simultaneous worktrees. It retains
the assignment across restarts and holds its lease until Bun stops, including
when the terminal interrupts the process. `bun run dev` works as well. Every
other Bun command passes through unchanged, as do Bun commands outside enabled
repositories.

Ragavan keeps port assignments and process-scoped locks in the user's local
application-state directory. It never writes them into the repository, and no
daemon is involved.

Enrollment can be inspected or reversed at any time:

```powershell
ragavan status
ragavan disable
```

Enrollment and integration operations also support structured output for
scripts and agents. Successful JSON is written to stdout, failures remain
non-zero and are written to stderr, and every value carries its schema version:

```console
$ ragavan status --json
{"enrollment":"enabled","schema_version":1}
```

Persistent shell integration is also reversible. This removes only Ragavan's
managed profile block; it does not remove the binary or change repository
enrollment:

```powershell
ragavan uninstall
```

## Workspace

The workspace is divided by current ownership, not anticipated implementation
layers:

| Crate | Owns |
| --- | --- |
| `ragavan` | Executable process entry point |
| `ragavan-cli` | CLI grammar, output contracts, and composition |
| `ragavan-core` | Shared identities, ports, enrollment, and launch plans |
| `ragavan-git` | Repository enrollment and worktree discovery |
| `ragavan-runtime` | Port allocation, process supervision, and resource lifecycles |
| `ragavan-adapters` | Command-runner and development-stack variants |
| `ragavan-shell` | Persistent shell integration, profiles, hooks, and their transport protocol |

The CLI library composes the capability crates and the executable only supplies
its process arguments. Git, runtime, adapters, and shell do not depend on each
other; shared domain values flow through `ragavan-core`.
Inside `ragavan-adapters`, command runners such as Bun resolve package scripts
into a runner-neutral representation consumed by stack adapters such as Vite.
The runner and stack registries are the only variant composition points; shell,
CLI, and runtime code remain independent of individual tools.

## Development

The pinned Rust toolchain includes the formatting and linting components used by
CI.

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```
