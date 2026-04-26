# ClawEnv → ClawPod (moved)

> **This repository has moved.**
>
> ClawEnv has been merged into the **Atta** monorepo, where it lives as
> the **ClawPod** desktop product alongside its sibling components
> (AttaGo mobile app, AttaCloud server, shared protocol).
>
> - **New home**: <https://git.asfly.ltd/Atta/AttaGo> (under `ClawPod/`)
> - **Last commit on this repo**: tagged [`pre-monorepo`](https://github.com/openatta/ClawEnv/releases/tag/pre-monorepo)
> - **History preserved**: all 234 commits live on inside the Atta
>   monorepo with `ClawPod/` as their root prefix (via
>   `git filter-repo --to-subdirectory-filter ClawPod`).

## Why the move

ClawEnv (sandbox installer + manager) and AttaGo bridge (cloud
connectivity for AI agents) were designed to be the same product seen
from two angles. Combining them lets a single `ClawPod` install give
users:

- Local sandbox install / upgrade / lifecycle for OpenClaw, Hermes,
  and other "claw" AI agents (the original ClawEnv mission)
- **Plus** mobile remote control via the AttaGo phone app, routed
  through the AttaCloud signaling service (the AttaGo bridge mission)

Anyone who only wants the cloud-relay piece can still run
`attago-bridge` standalone — both deployments share one source of
truth at `ClawPod/crates/atta-bridge/` in the new repo.

## Where to find things

| You're looking for | New location |
|---|---|
| ClawEnv source (core/cli/tauri) | `ClawPod/{core,cli,tauri}/` in the Atta monorepo |
| Bridge source (was AttaGo/bridge) | `ClawPod/crates/atta-bridge/` |
| ClawEnv docs (v2 design, sandbox, etc.) | `ClawPod/docs/` |
| Cross-product architecture | `docs/` at the Atta repo root |
| Bridge design | `ClawPod/crates/atta-bridge/docs/BRIDGE_DESIGN.md` |
| Issue tracker | Will move to the Atta monorepo's issue tracker; existing issues here remain readable |

## For existing clones

If you have a working clone of this repo:

```sh
# Fetch + check out the pre-monorepo tag if you need ClawEnv-specific
# state for any reason.
git fetch origin pre-monorepo
git checkout pre-monorepo

# Otherwise: clone the Atta monorepo and use ClawPod/ inside it.
git clone https://git.asfly.ltd/Atta/AttaGo.git Atta
cd Atta/ClawPod
```

This repo is now **archived**. New work happens only inside the Atta
monorepo.
