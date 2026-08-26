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

## Supported features

- Automatic project and development-stack detection
- Transparent use of existing project commands
- Stable port assignments across restarts and worktree moves
- Port collision avoidance and active-process coordination
- Repository-wide enrollment for existing and future worktrees
- Local state only: no tracked files, per-worktree configuration, or daemon
- Clear failure when safe isolation cannot be guaranteed

## License

[MIT](LICENSE)
