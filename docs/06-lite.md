# 6. ClawLite — Offline Installer Flavor

## Overview

**ClawLite** is a flavor of ClawEnv for users who receive a folder from
a technician containing the `ClawLite` binary plus pre-packaged
`.tar.gz` environment bundles. Double-click the binary → pick a
bundle → install → use. Post-install the user lands in the **full
ClawEnv main UI** — same Home, ClawPage, SandboxPage, Settings, tray
menu. The only thing Lite changes is the install entry point: every
path that would launch the online install wizard instead launches
`LiteInstallFlow` (offline bundle scanner).

The binary is distributed under the name **ClawLite** (product name)
with identifier `com.attaspace.clawlite` — distinct from the full
`ClawEnv` (identifier `com.attaspace.clawenv`) so both can coexist on
one machine.

## Architecture: one binary, one frontend, config-switched install

ClawLite and ClawEnv are produced from the **same** Rust binary and the
**same** webview frontend bundle. The only compile/bundle-time
difference is the Tauri config override in
`lite/clawlite.tauri.conf.json`, which changes `productName`,
`identifier`, `version`, and window dimensions. At runtime,
`src/App.tsx` reads `tauri::app::getName()` and swaps the install
component:

```
                     App (src/App.tsx) — ONE binary
                              │
                     getName() at mount
                    ┌─────────┴─────────┐
                    │                    │
             "ClawEnv"               "ClawLite"
                    │                    │
            InstallWizard           LiteInstallFlow
            (online + local +       (offline bundle scan
             native + native-        → import → API-key hint)
             import, 7 steps)
```

Every management feature — multi-instance sidebar, ClawPage tabs,
export/delete/config/upgrade, system tray, health monitor, exec
approval, quit dialog — is inherited from the main app with zero
duplicated code. There is **no second `dist/` build**, **no second
`index.tsx`**, **no second `node_modules`** — the lite flavor is 100%
a runtime/config artifact.

## File layout

```
src/
├── App.tsx                                # getName() gate → picks install component
└── pages/Install/
    ├── index.tsx                          # InstallWizard (ClawEnv default)
    ├── StepWelcome.tsx / StepProgress.tsx / ...  (shared steps)
    └── lite/
        ├── LiteInstallFlow.tsx            # 5-step offline orchestrator
        ├── LiteStepScan.tsx               # bundle scanner + "Choose file…"
        └── LiteStepApiKeyHint.tsx         # web-UI-only API-key hint

lite/
└── clawlite.tauri.conf.json               # ONLY override: productName / identifier / window
```

**Deleted in v0.3.1** (fold-in phase): `lite/src/`, `lite/index.html`,
`lite/vite.config.ts`, `lite/tsconfig.json`, `lite/package.json`,
`lite/node_modules`, `lite/dist`. The earlier side-by-side dist design
made debugging painful (two diverging frontend artifacts); folding
everything into `src/` removes that variable.

## The `InstallComponentProps` contract

Defined in `src/App.tsx`:

```ts
export type InstallComponentProps = {
  onComplete: (instances: Instance[]) => void;
  onBack?: () => void;
  defaultInstanceName?: string;
  clawType?: string;
  clawTypes?: ClawType[];
};
```

Both `InstallWizard` and `LiteInstallFlow` honour this. The three
`App.tsx` wire-up sites (first-run, `install_window` URL flow,
upgrade-available + background) render via `<Dynamic component={Install()} ...>`
so the active component can be swapped by the `getName()` check after
initial mount.

## LiteInstallFlow — 5 steps

```
0  Welcome          instance name (editable) + lang switch
1  Pick bundle      scan dir; filter by claw type if preset; "Choose file..." escape hatch
2  Confirm plan     product / version / bundle / mode / instance name
3  Install          reuse StepProgress (install_openclaw IPC)
4  API-key hint     web-UI-only, claw-specific wording + baked-proxy advisory
```

**No network step.** Lite bundles are offline-first — native-import
ships a self-contained `node_modules`, sandbox bundles ship a full VM
image; neither path reaches out to npm/github.

| # | Step | Component |
|---|------|-----------|
| 0 | Welcome | `StepWelcome` (shared). Honours `defaultInstanceName` prop, runs the same name-collision check as `InstallWizard` (`list_instances` + regex) |
| 1 | Pick bundle | `LiteStepScan` (lite-only). Filters by `filterClawType` when `clawType` prop is set — user hitting "+" on the Hermes tab only sees Hermes bundles |
| 2 | Confirm | inline in `LiteInstallFlow` |
| 3 | Install | `StepProgress` (shared) |
| 4 | API-key hint | `LiteStepApiKeyHint` (lite-only). Baked-proxy advisory banner shown here when `check_instance_proxy_baked_in` returns a non-empty config |

## Management UI: inherited verbatim

After install, `onComplete(instances)` hands off to the main
`MainLayout`. The user sees the full ClawEnv UI — no Lite-specific
management code path. All these work without modification:
multi-instance sidebar, ClawPage (start/stop/restart/export/delete/
config/upgrade/proxy/logs), SandboxPage, Settings, tray, health
monitor, quit dialog, exec-approval dialog.

## Build

From repo root:

```bash
# ClawEnv (main)
cargo tauri build

# ClawLite
( cd tauri && cargo tauri build --config ../lite/clawlite.tauri.conf.json )

# Both + deploy to ~/Desktop/ClawEnv
bash scripts/dev-deploy-macos.sh
```

Note the **`cd tauri &&`** for Lite — Tauri CLI's `--config` treats the
config file's parent directory as the Tauri project root and looks for
a `Cargo.toml` there. `lite/` has no `Cargo.toml` (the Rust binary is
shared with `tauri/`), so we invoke from `tauri/` with
`../lite/clawlite.tauri.conf.json` as the relative path. Paths inside
the merged config are resolved against `tauri/`.

The config filename was deliberately renamed from `tauri.conf.json` to
`clawlite.tauri.conf.json` — Tauri CLI auto-scans sibling directories
for files named `tauri.conf.json` and mis-identifies `lite/` as a
second Tauri project, which pollutes watchers and, in some release
builds, appeared to interfere with asset embedding. Renaming sidesteps
the scan.

Output paths (same `target/release/bundle/` tree as the main app):

- macOS: `ClawLite.app`, `ClawLite_0.3.1_<arch>.dmg`
- Windows: `ClawLite_0.3.1_<arch>-setup.exe` (NSIS), `ClawLite_0.3.1_<arch>_en-US.msi`

## Version SSOT

`scripts/check-version-sync.sh` enforces agreement across **six** files:
`core/Cargo.toml`, `cli/Cargo.toml`, `tauri/Cargo.toml`,
`tauri/tauri.conf.json`, `lite/clawlite.tauri.conf.json`, `package.json`.
No `lite/package.json` anymore — lite doesn't have a separate
frontend project.

## CI

Lite CI integration is **deferred** until the Lite flow has been
validated manually on macOS + Windows. Once stable, the CI matrix
job `release-bundle` should add a second build step per platform
with `cd tauri && cargo tauri build --config ../lite/clawlite.tauri.conf.json`
and upload both `ClawEnv` and `ClawLite` bundles to the release.

## Regression checklist (manual)

After any change to install wiring:

1. Fresh run (no instances): drop one bundle next to ClawLite, launch,
   scanner finds it, goes through install, lands in **Home with 1
   instance tab**.
2. Home "+" button: opens LiteInstallFlow in secondary window with
   `clawType=<active>`, scanner filters correctly; install completes,
   window closes, new instance appears in Home.
3. ClawPage tab "+": same as #2 but `clawType` pinned to the tab's
   claw type.
4. Name collision: attempt install with same name as existing →
   Welcome step shows error, Next disabled.
5. `filterClawType` mismatch: on Hermes tab "+", folder only has
   OpenClaw bundles → empty-filtered message, "Choose file..."
   offers escape.
6. Cancel from secondary install: click Back on Welcome (with
   `onBack` provided) → window closes, no instance created.
7. Baked-proxy advisory: import a sandbox bundle that carried
   `/etc/profile.d/proxy.sh` → yellow banner on API-key hint step.
