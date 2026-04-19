# Changelog

Notable changes per release. This project loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); dates are the tag
date. Entries group by area so users can skim the bits that matter to them.

## v0.2.12 — 2026-04-19

Hotfix: post-boot verify-and-reinstall base packages when provision's
apk silently fails.

Symptom: user installs with system proxy → `[38%] VM created...` →
`[40%] Installing OpenClaw...` → `[40%] [5s] /tmp/clawenv-install.sh:
line 3: npm: not found` → `Installing OpenClaw failed (exit 127)`.

Root cause: Lima cloud-init runs provision's `apk update && apk add
$PACKAGES` very early in VM boot. If the host proxy (Clash/Surge/etc)
isn't up YET at that exact moment, or there's a transient `Connection
refused` during the 4-mirror fallback, apk fails silently for some or
all packages — but Lima still reports the VM as running. Subsequent
install step then fails on the first missing binary (npm).

Fix: after `apply_to_sandbox` writes the persistent proxy config,
verify critical binaries (`npm`, `git`, `curl`) are present. If any
missing, re-run `sudo apk update && sudo apk add --no-cache <base>`
with up to 3 retries and exponential backoff. Runs only for
Lima/WSL2 (Native uses our own node/git downloads; Podman bakes
packages into the image at build time).

## v0.2.11 — 2026-04-19

Hotfix: `/etc/profile.d/proxy.sh` write was failing with Permission denied
in all sandbox backends. Pre-existing latent bug: `limactl shell` /
`wsl -d` / `podman exec` default to the unprivileged `clawenv` user, not
root. v0.2.7 and earlier masked this because proxy was injected via the
VM's provision YAML (which runs as root during boot); v0.2.8's unified
resolver moved the write to post-boot `backend.exec`, exposing it.

- `apply_to_sandbox` / `clear_sandbox` now stream the script through
  `sudo tee` / use `sudo chmod` / `sudo rm` for root-owned files. Lima
  cloud-init, WSL provision, and Podman Containerfile all grant the
  clawenv user NOPASSWD sudo, so this works on all three backends.
- `npm config set` stays non-sudo so per-user `.npmrc` in
  `/home/clawenv/.npmrc` is what ends up getting written — sudoing npm
  would write to root's config, which the running claw doesn't read.
- `mirrors::apply_mirrors` had the same latent bug for
  `/etc/apk/repositories` — also migrated to `sudo tee`.

## v0.2.10 — 2026-04-19

SSH robustness + UI linkage. Follow-up fixes against v0.2.9 proxy install.

**Fixes**:
- `apply_to_sandbox` / `clear_sandbox` merge 4 separate `backend.exec`
  calls into a single heredoc-fed shell invocation. Previously Lima's SSH
  ControlMaster right after VM boot got hammered and occasionally killed
  the 2nd or 3rd exec with `kex_exchange_identification: Connection reset
  by peer`. One exec = one SSH roundtrip, no warmup-window race.
- `background::run_background_script` same treatment — setup + launch
  merged into one exec, wrapped in retry.
- New `proxy_resolver::exec_with_retry(backend, cmd, label)` — exponential
  backoff (1s → 3s → 9s) specifically for SSH-level transients (exit 255,
  `Connection reset`, `kex_exchange_identification`, etc). Non-transient
  errors propagate immediately. Used by apply_to_sandbox, clear_sandbox,
  and background_setup.

**UI — claw ↔ VM linkage**:
- `SandboxPage` VmCard header now shows the claw instance name prominently
  with the sandbox_id as a grey secondary label (managed VMs only).
- `ClawPage` info table has a new "VM" row with the sandbox_id for non-
  native instances. Native claws (sandbox_id = "native") hide the row.
- `Instance` TS type gains optional `sandbox_id` field; `InstanceSummary`
  API response carries `sandbox_id` for frontend correlation.

## v0.2.9 — 2026-04-19

Hotfix for a regression introduced in v0.2.8's Phase 2 refactor. All three
sandbox backends (Lima / WSL / Podman) relied on proxy exports being in the
VM's first-boot provision script to reach Alpine CDN during `apk update`.
v0.2.8 removed those exports and moved proxy application to a post-boot
hook — but that runs AFTER provision has already hung on direct CDN fetches
in regions where the CDN is blocked (e.g. China).

Symptom: `limactl start` stuck at `Waiting for the final requirement 1 of 1:
"boot scripts must have finished"` for 10 minutes, then fatal with
`did not receive an event with the "running" status`. UI shows
`[35%] Installing system packages...` repeating.

Fix — **provision-time proxy restored; export scrub + post-boot apply kept**:

- `install.rs::provision_preamble` resolves `Scope::Installer` and embeds
  `export http_proxy=... export https_proxy=...` inline so Lima's YAML
  `provision:` script and WSL's bootstrap script have proxy before their
  `apk update/add` lines.
- `SandboxOpts` gains `http_proxy/https_proxy/no_proxy` fields. Podman
  backend passes them as `--build-arg HTTP_PROXY=... HTTPS_PROXY=...` to
  `podman build` — Docker/Podman's predefined proxy ARGs flow into
  `RUN apk` layers automatically without baking into the image's runtime env.
- `assets/podman/Containerfile` documents that HTTP_PROXY is handled via
  predefined ARGs (no explicit `ENV` — would persist into exported images
  and break on import across networks).
- Post-boot `apply_to_sandbox` (writes `/etc/profile.d/proxy.sh`) and
  export-time scrub both kept as-is. The provision preamble is transient;
  the persistent config is still the single source of truth for future
  VM shells.
- `docs/23-proxy-architecture.md` §9 updated with the "provision 三拍子"
  contract (provision-time inline export + post-boot persistent write +
  export-time scrub), clarifying which mechanism owns which window.

## v0.2.8 — 2026-04-19

Major proxy system overhaul — unified architecture, single resolver, correct
per-VM vs Native separation. Closes the 0.2.5 "install stuck at 6%" customer
bug and eliminates every class of proxy-related divergence we've hit. See
`docs/23-proxy-architecture.md` for the full spec.

**Proxy architecture (`docs/23-proxy-architecture.md`)** —
- **Unified ProxyResolver** (`core/src/config/proxy_resolver.rs`): every
  proxy read goes through `Scope::{Installer, RuntimeNative, RuntimeSandbox}
  ::resolve()`. No more scattered `if config.proxy.enabled else env else ...`
  across the codebase. Three scopes, one priority chain each, one apply
  surface (`apply_env` / `apply_child_cmd` / `apply_to_sandbox`).
- **Native = OS system proxy only, by design**. ClawPage has no proxy UI;
  `set_instance_proxy` IPC rejects Native; `InstanceConfig.proxy` is
  ignored for Native. Rule enforced at three layers so it can't erode.
- **Per-VM proxy UI moves to SandboxPage/VmCard**. Proxy belongs to the
  sandbox VM, not the claw instance. One VM, one proxy config, applies
  to every claw running inside it.
- **OS proxy watcher** (30s poll): Tauri notices when user toggles Clash,
  updates env, emits `os-proxy-changed` event. Subsequent start_instance
  picks up the new proxy automatically; no ClawEnv restart needed.
- **Per-VM proxy auth + Keychain**: ProxyModal manual mode supports HTTP
  basic auth. Password stored in `proxy-password-<instance>` keychain
  entry — never in config.toml, never in bundle exports. Deleted with
  the instance.
- **VM-internal connectivity test**: ProxyModal has "Test connectivity"
  buttons for international / CN / all target sets. `test_instance_network`
  IPC runs curl inside the VM, returns per-target ok + http_code + latency.

**Mirror infrastructure (`assets/mirrors.toml`)** —
- Unified `assets/mirrors.toml` replaces `git-release.toml` + hardcoded
  URL lists. Every asset (dugite, MinGit, Node.js, Lima, Alpine minirootfs)
  has `version` + `urls` + optional `[asset.sha256]` in one file. Loader:
  `core/src/config/mirrors_asset.rs`.
- CN mirror fallback for all assets: Node.js (npmmirror + huaweicloud +
  tsinghua), Alpine (tuna + aliyun), dugite (ghfast.top + ghproxy).

**Download path consolidation** —
- `core/src/platform/download.rs::download_with_progress` is now the single
  downloader. Streaming chunks, 60s per-chunk stall detection, 1 MiB /
  500ms progress throttle, sha256 verify, mirror URL fallback. Used by
  Git, Node, Lima, WSL distro, Podman image downloads.
- `download_silent` variant for contexts without a progress channel
  (update checker, sandbox backend trait).

**Install-time proxy unified with runtime** —
- `provision_preamble` no longer injects proxy exports at VM create time.
  Proxy is now applied via `apply_to_sandbox` as a post-boot hook (before
  apk/npm). Single apply path, no baked-in-on-create divergence.
- Sandbox export (`clawcli export`) scrubs `/etc/profile.d/proxy.sh` from
  the VM before tarring. Manifest records `proxy_was_configured = true` —
  import wizard reads this to prompt for fresh proxy config on the new host.

**Diagnostics** —
- New `clawcli proxy diagnose [--instance NAME]` prints every scope's
  resolved triple + source + current env. One command for support.
- All proxy-related tracing now uses `clawenv::proxy` target — filter with
  `RUST_LOG=clawenv::proxy=debug clawcli ...`.

**Platform matrix documented (CLAUDE.md)** —
- Linux GUI is explicitly unsupported going forward. CLI + sandbox on
  Linux keeps working; existing Linux GUI code is kept but not maintained.
  New GUI features only synchronise macOS + Windows.

## v0.2.7 — 2026-04-18

Two-pronged release: **bundle manifest protocol** (formalise the
export/import contract so importers stop guessing claw identity) and
**Hermes dashboard split** (give Hermes its real management UI instead of
a blank page where the gateway used to live). Plus quite a lot of old
debt paid down — version sync, CI `-D warnings` gate, Windows cross-
platform warnings that had been accumulating.

### Features

- **Bundle manifest (v1)** — every `.tar.gz` export now carries a
  `clawenv-bundle.toml` at archive root with `claw_type`, `sandbox_type`,
  `schema_version`, source platform, clawenv version. Import bails fast
  if the manifest is absent, wrong sandbox type, or newer schema. Drops
  the old "probe each claw's version command in a loop" heuristic — the
  manifest is authoritative. Podman/WSL get a wrapped outer tar (with
  manifest + `payload.tar` inner) because `podman save` / `wsl --export`
  produce container/distro tars we can't append to. See
  `docs/18-bundle-format.md` for the full schema + V1→V2 migration path.
- **Hermes dashboard as a separate daemon** — Hermes splits UI and API:
  `hermes dashboard` serves the React web UI + OpenAI-compatible API
  server, `hermes gateway run` serves messaging bridges (Telegram/
  Discord/WhatsApp) and is opt-in / manual. ClawDescriptor gets
  `dashboard_cmd` + `dashboard_port_offset`; `InstanceConfig.gateway`
  gets `dashboard_port`; start/stop/restart/upgrade/health-probe all
  track it. OpenClaw is unchanged (no dashboard_cmd = old behaviour).
- **Install-time Hermes provisioning** — chown `/opt/hermes` to the
  sandbox user, pre-build the React dashboard (`cd web && npm install
  && npm run build`), and auto-start `hermes dashboard` alongside the
  claw. User's first "Open Control Panel" click no longer waits 3+
  minutes for an npm build.
- **Tauri export → CLI薄壳** — `export_sandbox` / `export_native_bundle`
  now `spawn clawcli export --json` and translate JSON progress events
  into `export-progress` Tauri events. Deletes ~200 lines of duplicated
  tar/podman logic that used to live in `tauri/src/ipc/export.rs`.
  Aligns with CLAUDE.md铁律 8: "CLI 是核心，GUI 是薄壳".
- **One-shot legacy migration** — pre-v0.2.7 Hermes instances in
  `config.toml` had `dashboard_port = 0`; first `clawcli` run after
  upgrade detects that, computes `gateway_port + 5`, writes back AND
  patches the Lima VM's `lima.yaml` to forward the new port. Idempotent.
- **Export cancel works** — `run_cli_streaming` surfaces the child PID
  via callback; `export_cancel` taskkill/SIGTERM's the CLI. Used to be
  a silent no-op.
- **e2e scripts + Podman real-backend CI test** —
  `scripts/e2e-bundle.sh` (live instance roundtrip) and
  `scripts/e2e-bundle-offline.sh` (synthetic-bundle contract checks on
  CI); `core/tests/podman_roundtrip.rs` runs `podman save → wrap →
  unwrap → podman load` against a real Alpine image on Linux CI runners.
- **Version SSOT** — 3 × `Cargo.toml` + `tauri.conf.json` +
  `package.json` all hold the same version; `scripts/check-version-sync.sh`
  is a CI gate that fails on drift. (v0.2.5 shipped with `Cargo.toml =
  0.1.0` stamped into exported manifests — that particular embarrassment
  won't recur.)
- **`docs/README.md`** index added; new `docs/18-bundle-format.md`
  documents the manifest schema + wrap structure + evolution rules.

### Bug fixes

- **"Open Control Panel" empty page for Hermes** — the underlying
  reason the dashboard split was necessary. Previously
  `gateway_cmd = "gateway --port {port}"` invoked `hermes gateway`
  (messaging management), which errored at launch with "invalid choice:
  '3000'". Nothing listened on port 3000; button landed on a blank
  page.
- **Tauri `InstanceInfo` was dropping `dashboard_port`** — CLI
  `InstanceSummary` had the field but the IPC bridge map discarded it,
  so the frontend fell back to `gateway_port` and reproduced the blank
  page even after all the other fixes landed. Fixed by threading the
  field through.
- **`instance_health` / tray / start-readiness probe now consistent** —
  all three probe `dashboard_port` when set, `gateway_port` otherwise.
  Previously tray said "Stopped" while ClawPage said "Running" for
  Hermes.
- **`upgrade_instance` didn't relaunch dashboard** — step 1 killed
  both gateway + dashboard via `process_names()` but step 4 only
  restarted gateway. Hermes stayed dead after every upgrade.
- **ConfigModal port-conflict check ignored dashboard_port** — picking
  a `gateway_port` that collided with a sibling instance's dashboard
  would silently accept.
- **`wrap_with_inner_tar` was blocking on GB payloads** — `std::fs::
  rename/copy` in an async function stalled the tokio runtime for the
  duration of a WSL export (routinely 2–4 GB). Now uses `tokio::fs`.
- **`extract_inner_payload` buffered entire payload into RAM** —
  `tar -O ...` stdout → `std::fs::write` round-tripped GB through
  process memory. Now extracts to a scratch dir + `tokio::fs::rename`.
- **Peek/extract error messages stopped leaking tar stderr** — raw
  "tar: Not found in archive" noise is tracing::debug now, user sees a
  clean "bundles produced by pre-v0.2.6 clawenv can't be imported".
- **Dead code & platform-gate cleanup** — removed
  `check_upgrade_available` (never called), `is_win_native` (unused
  shadow), `install_dir` shadow in `NativeBackend::new`; gated
  `GitRelease` + its impl + its test module to `macos/linux` only so
  Windows `-D warnings` builds clean. `kill_by_name_cmd` /
  `check_process_cmd` pattern-escape vars moved into their cfg branches.
- **Hermes dashboard binds 127.0.0.1** — `0.0.0.0` was refused by
  Hermes without `--insecure` ("exposes API keys without robust
  auth"). Lima/WSL/Podman all port-forward guest 127.0.0.1 to host
  127.0.0.1 anyway, so loopback-only is correct + safer.

### CI hygiene

- `cargo clippy --workspace --tests -- -D warnings` is now a hard gate
  (was `continue-on-error: true` — dead code had been silently
  accumulating for weeks).
- Version SSOT guard runs on every CI build.
- New steps: "Bundle manifest unit tests", "Bundle offline e2e",
  "Install podman (Linux)" + "Podman wrap roundtrip (Linux)".
- Pre-existing Windows-specific clippy warnings fixed: dead
  `CommandExt` imports (tokio provides `creation_flags` directly),
  unused `esc` variables in non-unix cfg branches, `name_esc` in
  install_native, needless `return` in `bridge::permissions`.

## v0.2.5 — 2026-04-18

Big session focused on (1) privatising all sandbox/toolchain data under
`~/.clawenv/`, (2) tightening front-end ↔ back-end state sync, and
(3) making the Windows Native install flow actually work end-to-end.

### Private-data unification

- **Lima (macOS)** pinned to `~/.clawenv/lima/` via `LIMA_HOME`; private
  `limactl` downloaded + sha256-verified from GitHub to `~/.clawenv/bin/`
  (`assets/lima/lima-release.toml` — bump the toml to roll Lima forward,
  no code change).
- **Git (all platforms)** always privately installed:
  Windows → MinGit (git-for-windows), macOS/Linux → dugite-native
  (`assets/git/git-release.toml`); refusal to use system git, sha256 pinned.
- **Podman (Linux)** now in `~/.clawenv/podman-data/` + `~/.clawenv/podman-run/`
  via `XDG_DATA_HOME` + `XDG_RUNTIME_DIR` injection at process start — parity
  with Lima and WSL.
- **Node.js (all platforms)** private install at `~/.clawenv/node/`.
- Native bundle export now refuses to run if `node/`, `git/`, or `native/`
  is missing — old "silent half-bundle" path is gone.

### State-sync rewrite

- Single canonical `instance-changed` event now emitted on every mutating
  backend IPC (`start`, `stop`, `delete`, `rename`, `edit_ports`,
  `edit_resources`, `install_chromium`, `upgrade`, and `install`). Retired
  the legacy `instances-changed` duplicate.
- Front-end `MainLayout` is the single subscriber; drives list refresh,
  health refetch, `activeTab` fixup on rename/delete, gateway-token epoch
  bump on start/restart, and a "needs restart" toast after port/resource
  edits.
- `sandbox_vm_action("delete")` on a managed VM now cascades to
  `delete_instance_with_progress` so config.toml stays consistent (no more
  ghost instances in the Home / ClawPage lists after a VM-page delete).

### Windows-native install & export fixes

- `managed_shell::spawn_detached` rewritten for Windows: one-shot `.bat`
  wrapper + `powershell Start-Process -WindowStyle Hidden`, with stdin/out
  /err redirection done inside the bat so the hidden-console scenario
  doesn't silently kill the openclaw gateway mid-boot.
- `start_instance` health-check window extended from 13 s → 25 s to match
  real Windows ARM64 openclaw startup times (loading config →
  resolving auth → plugins ready → HTTP listener).
- `clawcli export` now branches on `sandbox_type`: Native mode uses
  Windows' built-in `tar.exe` to pack `~/.clawenv/{node,git,native}`,
  sandbox mode keeps the old `package-alpine.sh` path. No more falling
  through to `bash.exe` → WSL on Windows.
- Bundle export refuses `--output <existing dir>` (used to nest the
  tarball silently one level deeper than requested).

### Install-flow polish

- **VM name ↔ instance name mapping fixed**: `vm.name == instance.sandbox_id`
  (an auto-generated hash) is the correct mapping, not `"clawenv-" +
  instance.name`. Terminal / Chromium / Export / Delete buttons on the VM
  page now work for real. Backend adds `instance_name` to `SandboxVmInfo`;
  front-end VmCard uses it.
- Alpine mirror fallback chain inside the provision script: try user-
  configured, then dl-cdn, Tsinghua, Aliyun, SJTU. Both `apk update` and
  `apk add` get their own timeouts so a slow/flaky CDN mirror can't hang
  the whole install.
- Background script adds a 30 s heartbeat line to the log so a silent
  npm phase doesn't trip the poller's idle kill.
- `exec_helper` idle ceiling 10 min → 20 min; front-end idle timer 5 min
  → 22 min (just the safety net; heartbeat is the real fix).
- `StepProgress` extracts "current package" from npm output (shows
  `postinstall: @matrix/…`, `fetch: lodash`, `apk: busybox`) instead of a
  generic spinner message.
- Retry button reworked: `createEffect(on(retryTrigger))` replaces the
  `<Show keyed>` remount pattern that silently swallowed the listener
  registration under certain Tauri+Solid timings.
- Terminal WebSocket reconnect: 4 retries with 0/0.8/2/4 s backoff so a
  just-restarted ttyd gets a chance to come up before the UI gives up.
- `INSTALL_RUNNING` now guarded by an RAII `Drop` — a panic in the spawn
  task no longer leaves the lock set forever.

### Nanoclaw out, OpenClaw pinned to first

- Removed nanoclaw from `assets/claw-registry.toml`.
- `ClawRegistry::list_all` orders OpenClaw first; everything else follows
  alphabetically.

### CI

- Tight matrix (3-OS Test job) on every push/PR; the heavy matrix
  (CLI smoke, Tauri check, Linux cross-compile) only runs on tag pushes
  or manual `workflow_dispatch` — cuts email noise for day-to-day commits.
- `concurrency: cancel-in-progress` so a rapid re-push cancels the old run
  instead of queueing.
- CLI-smoke `sandbox` probes now `|| true` — hosted runners lack Lima/
  Podman/WSL, so their failure is expected noise, not a real error.
- Unix-only `exec_helper` unit tests gated with `#[cfg(all(test, unix))]`
  so Windows CI no longer fails on `echo`/`sh` not being PATH-resolvable
  executables.

### Tests

- 133 total; new coverage:
  - `managed_instance_for_vm` mapping (4 cases, regression guard for the
    "strip `clawenv-` prefix" bug).
  - `GitRelease::render_urls` (regression guard for dugite's `tag` vs
    `upstream_version` distinction that previously produced a 404 URL).
  - `ClawRegistry::list_all` puts openclaw first.

### Known limitations

- macOS .app and Windows MSI/NSIS installers are unsigned — users will
  see Gatekeeper / SmartScreen warnings on first launch.
- Proxy-related env vars (`HTTP_PROXY`, `HTTPS_PROXY`) are inherited into
  the Lima VM and Podman containers; if the proxy is down, apk/npm will
  timeout through our mirror fallback before failing with a clear error.
- macOS 26.4 + Tauri 2.10 have a known-upstream WebKit crash
  (`dispatchSetObscuredContentInsets`) when opening external URLs from the
  webview. Waiting on Apple's fix; the crash isn't reproducible from
  typical navigation within the app.
