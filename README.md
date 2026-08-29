# Ragavan

Ragavan gives every Git worktree its own isolated development environment automatically.

> [!NOTE]
> Ragavan is a proof-of-concept and lacks many features. Use at your own risk.

## Install

```powershell
cargo install --git https://github.com/chiefGui/ragavan --locked ragavan
ragavan install
```


## Quick start

Enable Ragavan once from any git project:

```powershell
ragavan enable
```

Have fun.

## Dashboard

Inspect every repository and stable development-service assignment known to Ragavan:

```powershell
ragavan dashboard
```

Use `--current` to limit the snapshot to the repository containing the current directory. This still includes every linked worktree in that repository. Both forms support the global `--json` option.

The dashboard identifies managed development services by repository, worktree, and repository-relative package scope. An `active` lease means Ragavan currently holds that service's coordination lock; it does not claim that the process is healthy or reachable. An `inactive` lease is a retained stable port assignment with no running lease.

Successful enrollment registers a repository for global discovery, and each managed development launch refreshes that registration. Registrations whose Git directory can no longer be found remain visible as `unavailable`. Disabling a repository removes its registration while retaining its stable port assignments, which remain visible as `unregistered`.

If two live Git directories contain the same copied Ragavan repository ID, management stops with an identity-conflict diagnostic rather than combining their services.

## Architecture

`ragavan-application` is the front-end-independent product boundary. It owns repository enrollment, dashboard reconciliation, shell-hook composition, and intercepted-command execution. The CLI supplies process inputs and renders typed application outcomes as terminal text or JSON; it does not read Git or runtime state directly. A graphical frontend can consume the same application workflows and dashboard model without invoking or depending on the CLI.

## Supported features

- Automatic project and development-stack detection
- Transparent use of existing project commands
- Stable port assignments across restarts and worktree moves
- Port collision avoidance and active-process coordination
- Repository-wide enrollment for existing and future worktrees
- One-shot global and current-repository management dashboard
- Local state only: no tracked files, per-worktree configuration, or daemon
- Clear failure when safe isolation cannot be guaranteed

## License

[MIT](LICENSE)
