# ClawEnv v2 — release notes

## v2-feature-complete — 2026-04-25 (pending)

Adds the post-rc1 hardening pass: real-machine matrix proven, two
post-install bugs caught & fixed, perf baselines captured.

### What landed since rc1

- **Bug #1 fixed (P3-machine):** `clawcli install --backend native`
  now self-bootstraps node when missing under `--autoinstall-deps`
  instead of bailing with a hint pointing at a `Unsupported` verb.
  `Native` variant was also missing from `InstallBackendSel` (was
  marked "deferred"); now wired through.
- **Bug #2 fixed (P3-machine):** e2e harness `expect_config_entry`
  asserted on `~/.clawenv/instances.toml` but v2 actually writes to
  `~/.clawenv/v2/instances.toml`. Fixed in `tests/e2e/lib/assert.sh`.
- **Bug #3 fixed (P3-roundtrip):** `clawcli instance destroy` only
  removed the registry entry + port forwards but left the Lima/Podman/
  WSL VM on disk. A subsequent `clawcli import --name <same>` then
  bailed with "already present". `InstanceOrchestrator::destroy()`
  now calls `backend.destroy()` between port-forward cleanup and
  registry removal; backend errors are tolerated (logged, not fatal)
  so a half-broken VM doesn't block registry cleanup.

### Perf baselines (macOS arm64, system HTTP proxy)

Captured 2026-04-25 from `smoke-mac-install-matrix.sh` and
`smoke-mac-launch.sh`. All times include proxy overhead. Hardware-
dependent; treat as upper-bound on similar Macs.

**Install pipeline (cold cache, fresh isolated $HOME):**

| pipeline                   | install_elapsed | notes |
|----------------------------|-----------------|-------|
| native + hermes (bail)     | <2s             | validation rejects pre-pipeline |
| native + openclaw          | 197s (3:17)     | node bootstrap (47MB) + npm install |
| lima + openclaw            | 632s (10:32)    | Lima boot + apk + npm + 3 MCP plugins |
| lima + hermes              | 671s (11:11)    | Lima boot + uv + python + git+pip + dashboard pre-build + 3 MCP plugins |

**Post-install runtime:**

| operation                      | wall   | notes |
|--------------------------------|--------|-------|
| `clawcli launch` (openclaw)    | ~90s   | Node import (~10s) + plugin runtime-deps install (acpx ~50s, browser ~30s) → HTTP listener up at 89.4s |
| `clawcli launch` (warm cache)  | <10s   | once plugin caches are warm subsequent launches skip the runtime-deps install |
| `clawcli stop` (Lima)          | ~5s    | qemu shutdown returns; hostagent SIGTERM'd at destroy time |
| `clawcli instance destroy`     | ~10s   | hostagent kill + 300ms grace + `limactl delete --force` + template cleanup |

### Scenarios green (real machine, 2026-04-25)

- `smoke-mac-install-matrix` — 4/4 combos PASS (~27min wall)
- `smoke-mac-install-quick` — 6/6 assertions PASS (170s)
- `smoke-mac-blocked-egress` — bails clean in 2s (negative test)
- `smoke-mac-roundtrip` — phases 1-5 PASS (install + export + destroy + import + start, covered P3-a)
- `smoke-mac-launch` — 7/7 assertions PASS (974s wall, covered P3-c)

### Counts

- 318 unit tests passing (`cargo test --workspace --lib`)
- `cargo clippy --workspace --tests -- -D warnings` clean
- 11 e2e smoke scenarios (3 quick + 8 long-form)

---

## v2-rc1 — 2026-04-25

First release candidate of the v2 codebase. v2 is a top-to-bottom rewrite of
ClawEnv's core Ops layer + CLI surface. It lives under `v2/` until the
final consolidation (Phase M); v1 in the root tree continues to ship.

What this RC says is true: the v2 CLI and core layer reach **functional
parity with v1's CLI** for install / upgrade / start / stop / launch /
exec / shell / doctor / net-check / export / import / instance ops, plus
new ergonomics that v1 lacks. The Tauri GUI **does not** consume v2 yet —
that's Phase M.

### Highlights

- **One binary, one workspace.** `v2/cli/` produces a `clawcli` binary
  that exposes a unified verb + noun grammar. The verb layer
  (`install`, `upgrade`, `launch`, `start`, `stop`, `exec`, `shell`,
  `status`, `list`, `net-check`, `export`, `import`, `doctor`) covers
  what users want to *do*; the noun layer (`claw`, `sandbox`, `native`,
  `download`, `instance`, `proxy`) is the layer-direct surface for
  scripts and power users.

- **Three sandbox backends, peer impls.** `LimaBackend`, `WslBackend`,
  `PodmanBackend` all implement `SandboxBackend` with the same trait
  surface (`create / destroy / start / stop / exec / exec_argv / exec_argv_with_retry / ensure_prerequisites / export_image / import_image / stats / edit_port_forwards`).
  `detect_backend()` returns one — no nesting, no composition, no
  hybrids. Mirrors v1's "三平台对等" architectural rule.

- **Wire protocol parity with v1's GUI bridge.** `clawcli --json` emits
  line-delimited `CliEvent` JSON: `{type:"progress",stage,percent,message}` /
  `{type:"info",message}` / `{type:"data",data}` / `{type:"complete",message}` /
  `{type:"error",message,code?}`. The same protocol v1's Tauri
  `cli_bridge` already speaks. Phase M's Tauri swap will be
  deps-only, no wire changes.

- **Post-boot 3-probe verify gate (P2-b, v0.2.12 lesson).** Every
  install runs `provisioning::post_boot::verify_post_boot` between VM
  boot and provisioning: `echo` → `getent hosts localhost` → tmp file
  touch. Each probe gets its own 4-attempt retry budget via
  `exec_argv_with_retry`. Catches the SSH-master / resolv.conf empty /
  rootfs-still-ro race conditions that v1 v0.2.x silently corrupted
  apk installs over.

- **Triple-deadline download gate (lifted from v1).** Slow / black-hole
  downloads bail in seconds (CONNECT_TIMEOUT) / minutes (CHUNK_STALL) /
  by throughput floor (MIN_THROUGHPUT). Negative test:
  `smoke-mac-blocked-egress.sh` regression-tests this — pointing
  HTTPS_PROXY at a black hole must surface a structured error within
  120s, not stall for 30 minutes.

- **Bundle export / import (P2-e).** `clawcli export <name> --output X.tar.gz`
  produces a v1-compatible bundle (`clawenv-bundle.toml` + `payload.tar`
  at archive root). `clawcli import --file X.tar.gz --name N --port P`
  validates the manifest schema, restores the VM/container via
  `backend.import_image`, and registers a fresh `InstanceConfig`. Schema
  is shared with v1 — bundles produced by v2 carry kebab-case backend
  names ("lima"/"wsl2"/"podman") which v1 readers will see as a v2
  origin marker.

- **HIL browser stack (P1-e).** `BrowserBackend` + `ChromiumBackend`
  cover headless ↔ noVNC switching for HIL CAPTCHA / 2FA flows. Same
  `Xvfb → x11vnc → websockify → chromium` chain as v1, lifted verbatim.

### What's NOT in this RC

- **No Tauri GUI integration.** Phase M deps-swap, IPC rewrite, and
  schema-drift fixes all happen *after* v2-rc1 stabilises in the field
  via clawcli usage.
- **No Windows e2e scenarios.** They need the win-rsync + remote-build
  infra (`scripts/win-remote.sh` from v1). Tracked as P3 work.
- **No `clawcli sandbox rename` for WSL/Podman.** Lima rename works via
  `limactl rename`; the other two need destroy+recreate which isn't
  yet wrapped. Bails clean with a clear hint.
- **No native-mode export / import.** Sandboxed only.

### How to try it

```bash
cd v2
cargo build -p clawops-cli --release
./target/release/clawcli --help

# Run the cheap smoke scenarios (~30s each)
cd ..
./v2/tests/e2e/run.sh smoke-mac-native-noproxy
./v2/tests/e2e/run.sh smoke-mac-native-http-proxy

# Run a full install + verify roundtrip (~10-12 min, needs proxy on
# GFW networks; see scenario header for E2E_FORCE_NOPROXY override)
./v2/tests/e2e/run.sh smoke-mac-roundtrip

# Or run everything that fits the host
./v2/tests/e2e/run.sh all
```

CI: `.github/workflows/v2-e2e.yml` runs the unit suite + host-mode smokes
nightly on macOS / Linux / Windows. Sandbox scenarios run on manual
dispatch (`run_sandbox_scenarios=true`) or v2-rc / v0.4 tag pushes.

### Path to v2-feature-complete and v0.4.0

- **P3** (1-2 weeks): iterate on bugs surfaced by real-machine smoke
  runs; add post-rc1 hardening; tag `v2-feature-complete`.
- **Phase M** (1-2 weeks): swap Tauri's Cargo deps to `clawops-core`,
  rewrite the 28 Type-B IPC handlers, fix frontend schema drift; tag
  `v0.4.0-rc1`.
- **Phase C** (3-5 days): delete v1, move v2 to root; tag `v0.4.0`.

### Counts

- 318 unit tests passing (`cargo test --workspace --lib`)
- 14/14 integration tests in `runner_local.rs` pass when run serially
  (parallel default may flake under contention — `--test-threads=1`
  is the safe knob for now; tracked as a P3 follow-up)
- `cargo clippy --workspace --tests -- -D warnings` clean
- 9 e2e smoke scenarios written; 2 quick ones (host mode) verified
  PASS in this RC build (sandbox + roundtrip need real VM provisioning,
  ~10-15min each — unrun in CI yet)
- Commits f0a439a → HEAD (the v2 lineage on `main`)
