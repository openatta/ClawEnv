# Changelog

Notable changes per release. This project loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); dates are the tag
date. Entries group by area so users can skim the bits that matter to them.

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
