# clawcli v2 — Command-Line Design

Status: design doc, ground truth for implementation. Replaces ad-hoc
verb evolution.

This document defines what `clawcli` looks like in v2. It does **not**
preserve v1 backward compatibility — the `--mode` / `--claw-type` /
`--browser` flags, top-level `uninstall` / `rename` / `edit` / `logs` /
`claw-types` / `update-check` aliases were Phase-M-bridge debt and are
removed. Tauri's GUI layer adapts to this design (see `Phase M-thin`),
not the other way around.

---

## 1 Design principles

1. **One concept, one verb.** No `start <name>` AND `sandbox start
   <name>` for the same operation. Pick the most natural surface and
   route advanced layer-direct calls through noun groups.
2. **Verbs are task-oriented, nouns are layer-direct.** `install`
   creates an instance; `instance create` creates a registry entry
   without a backend. Two different intents, two different surfaces.
3. **JSON is the wire contract.** `--json` flips every verb into a
   line-delimited `CliEvent` stream. Human output is a courtesy
   rendering of the same data.
4. **Sandbox and Native are peers.** No verb is "sandbox-only" or
   "native-only" without a clear reason. Where the two diverge, the
   verb tells you upfront with a structured error, not by silently
   doing the wrong thing.
5. **No defaults that surprise.** Backend, version, name, port — all
   either explicit or fall back to a documented "host default" that
   the user can override globally. Magic values (e.g. `default` as
   instance name) are spelled out.
6. **Errors are recoverable, not opaque.** Every failure includes a
   `hint` (what the user can do next), and progress events keep
   firing so the GUI / user sees the state machine even on the sad
   path.

---

## 2 Verb taxonomy

```
clawcli <verb> [args]               # task-oriented, primary surface
clawcli <noun> <subverb> [args]     # layer-direct, advanced surface
```

### 2.1 Lifecycle (task-oriented top-level)

| verb | summary |
|---|---|
| `install <claw>` | end-to-end install: create instance + provision backend + deploy claw + register |
| `uninstall <name>` | end-to-end teardown: stop daemons + destroy backend + remove from registry |
| `upgrade <name>` | bump claw to a new version (reuses VM) |
| `start <name>` | bring instance's backend online (start VM, no daemon spawn) |
| `stop <name>` | take instance's backend offline |
| `restart <name>` | stop + start |
| `launch <name>` | spawn the gateway/dashboard daemons + probe ready_port |

### 2.2 Inspection (read-only)

| verb | summary |
|---|---|
| `list` | every registered instance, summary table or JSON |
| `status <name>` | one instance: VM state + claw version + ports + caps |
| `info <name>` | full registry record (registry-only, no probe) |
| `logs <name>` | tail recent gateway/dashboard logs from inside the instance |
| `exec <name> -- <cmd>` | one-shot command execution inside the instance |
| `shell <name>` | interactive shell inside the instance |
| `doctor [<name>]` | aggregate diagnostics (cross-layer); `--all` iterates every instance |
| `net-check` | connectivity probes from host or from inside an instance |
| `token <name>` | gateway auth token from inside the instance |

### 2.3 Distribution

| verb | summary |
|---|---|
| `export <name> --output <path>` | tar.gz bundle with `clawenv-bundle.toml` manifest |
| `import --file <path> --name <new>` | restore a bundle as a new instance |

### 2.4 Configuration (top-level group)

| verb | summary |
|---|---|
| `config show` | dump the effective `~/.clawenv/config.toml` (resolved with env overlay) |
| `config get <key>` | read one key (dot notation, e.g. `proxy.http`) |
| `config set <key> <value>` | write one key |
| `config unset <key>` | delete one key |
| `config validate` | parse + schema-check, no write |

Replaces v1's `settings save <json-blob>` / `settings diagnose`.
Cleaner: one canonical write surface, no JSON-blob form.

### 2.5 Self-introspection

| verb | summary |
|---|---|
| `system info` | host OS + arch + memory + disk + sandbox availability |
| `version` | clawcli version + build commit + capabilities |
| `state` | runtime state (FirstRun / NotInstalled / Ready) for GUI startup |

`system info` replaces v1's `system-check` (verb name didn't make
sense as it's pure inspection, no checks/asserts). The resulting
JSON shape is `SystemInfo` — see §5.

### 2.6 Layer-direct (noun groups)

For advanced/scripted use. Each group exposes operations that don't
fit naturally as a top-level verb.

| group | when to use |
|---|---|
| `claw <sub>` | invoke a claw product's own CLI (passthrough): `claw version <id>`, `claw doctor <id>`, `claw config get/set <id>` |
| `sandbox <sub>` | sandbox VM ops that aren't part of the lifecycle: `sandbox edit/rename/port/prereqs/stats/disk-usage` |
| `native <sub>` | host-runtime ops: `native components`, `native upgrade node`, `native repair` |
| `download <sub>` | artifact catalog: `download list/fetch/doctor` |
| `instance <sub>` | registry-only operations: `instance info/create/destroy/health` (no backend touch) |
| `proxy <sub>` | proxy config + apply: `proxy resolve/get/set/check/apply/clear/set-password` |
| `bridge <sub>` | AttaRun bridge daemon: `bridge config/start/stop/status` |
| `browser <sub>` | Chromium HIL state machine: `browser status/hil-start/hil-resume` |

---

## 3 Argv conventions

### 3.1 Shape

```
clawcli [GLOBAL OPTS] <verb> [<positional>...] [--flag VALUE...]
```

- Positional args are required arguments specific to the verb.
- Flags are optional or have host-default fallbacks.
- Global flags work across every verb:
  - `--json` — emit line-delimited CliEvents instead of human text
  - `--quiet` — suppress non-error output
  - `--instance <name>` — fallback for verbs that take `<name>` positionally
  - `--no-progress` — suppress Progress events (still emits Data + Complete)

### 3.2 Naming rules

- Flag names use `kebab-case`: `--gateway-port`, `--memory-mb`,
  `--install-browser`, `--no-proxy`. No underscore variants.
- Boolean flags are bare presence: `--browser`, `--all`, `--check`.
  No `--no-foo` opposites unless there's a real default-on case.
- Repeatable args use `--port HOST:GUEST[:LABEL]` (one flag, structured value).
- Time durations always have explicit units in the flag name:
  `--timeout-secs`, `--retry-budget-secs`. No bare `--timeout`.

### 3.3 Positional vs flag

A field is positional iff:
1. It's required (no host default), AND
2. It's the natural subject of the verb ("install **what**?").

Examples:
- `install <claw>` — claw id is positional (required, the subject)
- `--name <N>` — flag (optional, defaults to `default` or `--instance` global)
- `status <name>` — positional (required, the subject)
- `--backend <B>` — flag (host-default fallback)

### 3.4 No silent argv translation

v1 had `--mode sandbox|native` mapping to a backend. v2 doesn't:
backends are explicit (`--backend lima|wsl2|podman|native`). If the
user wants "host default sandbox", the host-default fallback fires
when `--backend` is omitted entirely. No magic strings.

---

## 4 JSON event protocol (`--json` mode)

### 4.1 Event types

Line-delimited JSON. One event per line. Spec:

```json
{"type": "progress", "stage": "install-claw", "percent": 80, "message": "..."}
{"type": "info",     "message": "..."}
{"type": "data",     "data": {...}}
{"type": "complete", "message": "ok"}
{"type": "error",    "message": "...", "code": "...", "hint": "..."}
```

- **progress** — stage milestone. `stage` is a stable, lowercase-kebab id
  (e.g. `validate`, `create-vm`, `boot-verify`, `install-claw`,
  `verify`, `done`). `percent` is monotonic 0-100.
- **info** — informational message, no progress meaning.
- **data** — structured result. The verb's return shape goes here.
  Most verbs emit exactly one Data event; long-running streaming
  verbs (e.g. `logs --follow`) emit many.
- **complete** — normal exit signal. Always emitted on success.
- **error** — failure signal. Always emitted before non-zero exit.
  - `code` is a stable kebab id (e.g. `instance-not-found`,
    `network-blocked`, `vm-busy`).
  - `hint` is a short user-actionable next step.

### 4.2 Wire contract

- Every `--json` invocation emits **at most one** `complete` event,
  **at most one** `error` event, **never both**.
- Process exit code mirrors event: `complete` → exit 0, `error` →
  exit 1.
- `data` events MUST conform to the verb's documented response type
  (see §5).
- `progress` percent is monotonic non-decreasing within a verb.

### 4.3 Cancellation

- The CLI honours SIGTERM (Unix) / Ctrl-C (anywhere) by:
  1. Setting an internal cancel token.
  2. Emitting `error` with `code: "cancelled"`.
  3. Cleaning up child processes via RAII (kill_on_drop).
- Long-running operations (install, export) check the token at every
  stage boundary and bail before the next stage.

### 4.4 Idle timeout

`run_cli_streaming` (GUI's spawn helper) considers the child idle if
no `progress` event in 240s. It SIGTERMs the child. CLI verbs that
do legitimately long-running work (bulk apk add, Lima boot) emit
heartbeat progress every ≤120s.

---

## 5 Response types (Data event payloads)

All verbs that emit Data MUST conform to a documented type. Type
definitions live in `v2/core/src/wire/`. Names match the verb.

| verb | Data type |
|---|---|
| `list` | `ListResponse { instances: [InstanceSummary] }` |
| `status` | `StatusResponse` |
| `info` | `InstanceConfig` |
| `logs` | `LogResponse { content: string }` |
| `exec` | `ExecResult { stdout, stderr, exit_code }` |
| `doctor` | `DoctorReport` (or `[DoctorReport]` with `--all`) |
| `net-check` | `NetCheckReport` |
| `token` | `{token: string}` |
| `export` | `ExportReport` |
| `import` | `ImportReport` |
| `upgrade --check` | `UpdateCheckResponse` |
| `system info` | `SystemInfo` |
| `version` | `VersionInfo` |
| `state` | `LaunchState` |
| `claw list` | `ClawTypesResponse { claw_types: [ClawTypeInfo] }` |
| `sandbox list` | `SandboxListResponse` |
| `sandbox stats` | `SandboxStats` |

Full schemas in `v2/core/src/wire/mod.rs` — single source of truth.

---

## 6 ExecutionContext abstraction

The biggest cleanup over v1 is unifying sandbox-exec and native-exec
under one trait. v1 had `SandboxBackend` for VMs and a tangled set
of helpers for native. v2 says: every execution target implements
`ExecutionContext`, and verbs that don't care WHERE they run accept
`Arc<dyn ExecutionContext>`.

### 6.1 The trait

```rust
#[async_trait]
pub trait ExecutionContext: Send + Sync {
    /// Identifier (e.g. "lima:default", "podman:abc123", "native").
    fn id(&self) -> &str;

    /// Where this context maps in the abstract space.
    fn kind(&self) -> ContextKind;

    /// Run a command, returning stdout. Errors carry a structured cause.
    async fn exec(&self, argv: &[&str]) -> Result<String, ExecError>;

    /// Same with retry on transient errors (SSH master not warm,
    /// CONNECTION_RESET, etc.). Backoff schedule defined by impl.
    async fn exec_with_retry(&self, argv: &[&str]) -> Result<String, ExecError>;

    /// Resolve an in-context path to a host path if possible.
    /// Native: returns the same path. Sandbox: returns None unless the
    /// VM has a host mount that covers the path.
    fn resolve_to_host(&self, ctx_path: &Path) -> Option<PathBuf>;

    /// Check whether this context is currently usable (VM running,
    /// host has the binary, etc.). Used by health checks.
    async fn is_alive(&self) -> bool;

    /// Long-running streaming exec. The caller drives the iterator.
    /// Used for install pipelines (apk update / npm install) where
    /// progress feedback matters.
    async fn exec_streaming(
        &self,
        argv: &[&str],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ExecError>;
}

pub enum ContextKind {
    Sandbox { backend: BackendKind, instance: String },
    Native  { prefix: PathBuf },  // e.g. ~/.clawenv/native/<inst>
}
```

### 6.2 Implementations

- `LimaContext`, `PodmanContext`, `WslContext` — wrap the existing
  `SandboxBackend` impls. `exec` shells through `limactl shell` /
  `podman exec` / `wsl --exec`. `exec_with_retry` honours v1's SSH
  master warmup pattern set (CHANGELOG v0.2.10).
- `NativeContext` — wraps `tokio::process::Command` rooted at the
  instance's native prefix dir. `exec` cd's into that dir; `is_alive`
  checks for the claw binary.
- `MockContext` — for unit tests.

### 6.3 What v1 got right (kept)

- **3-probe post-boot verify** (alive / dns / fs) — `LimaContext`'s
  `is_alive` does this on demand.
- **Triple-deadline downloads** (`CONNECT / CHUNK_STALL / MIN_THROUGHPUT`)
  — lives in `download_ops`, used regardless of context.
- **Background script pattern** (`nohup ... > log; tail; done file`)
  — `ExecutionContext::exec_long_running` helper, wraps the v1
  `run_background_script`.
- **`exec_with_retry` budget** (5 attempts, 0/1/3/9/30s) — lives in
  the trait's default impl.
- **kill_on_drop** for child processes — every spawn uses it.
- **Wire-protocol streaming** (line-delimited JSON) — described in §4.

### 6.4 What changes vs v1

- **No more "is this sandbox or native?" branching in business logic**.
  `install_claw_in_context(ctx, opts)` works for both. The pipeline
  has the same stages; only the `ExecutionContext` impl differs.
- **One retry budget definition**. v1 had three different retry helpers
  in three different modules. v2 has one trait method.
- **Lifecycle is explicit**. `start` / `stop` / `restart` operate on
  the context, not "the VM". `NativeContext::start` is a no-op (host
  is always "started"); `LimaContext::start` invokes `limactl start`.
  The verb is the same.

---

## 7 Concrete verbs — full argv

Every verb listed in §2 with its complete argv. All flags shown.

### 7.1 `install <claw>`

```
clawcli install <claw> --name <N> --backend <B> --version <V> --port <P>
                       [--cpus <N>] [--memory-mb <N>] [--browser]
                       [--workspace <PATH>] [--proxy-url <URL>]
                       [--autoinstall-deps] [--dry-run]
```

- `<claw>`: `openclaw` | `hermes` | … (positional, required)
- `--name`: instance name (default: `default`)
- `--backend`: `lima` | `wsl2` | `podman` | `native` (default: host's)
- `--version`: `latest` | `<semver>` | `<git-ref>` (default: `latest`)
- `--port`: gateway host port (default: 3000)
- `--cpus`, `--memory-mb`: VM resources (sandbox only; ignored for native)
- `--browser`: install Chromium + noVNC (sandbox only)
- `--workspace`: bind-mount path (sandbox only); host base for native
- `--proxy-url`: ad-hoc proxy for this install (env-overlay, not persisted)
- `--autoinstall-deps`: native-only — install node/git into ~/.clawenv if missing
- `--dry-run`: render templates only, no backend invocation

Emits: progress (every stage), data (`InstallReport`), complete.

### 7.2 `uninstall <name>`

```
clawcli uninstall <name> [--keep-bundle <PATH>] [--force]
```

- `--keep-bundle`: export to PATH before destroying (atomic backup)
- `--force`: bypass "are you sure" check (cli-only; GUI always force)

Emits: progress (lookup → stop → destroy-vm → remove-registry), complete.

### 7.3 `upgrade <name>`

```
clawcli upgrade <name> [--to <version>] [--check] [--keep-config]
```

- `--to`: target version (default: `latest`)
- `--check`: registry probe only, no install
- `--keep-config`: preserve user config file (default: yes)

Emits: progress (or just data when `--check`), data (`UpgradeReport` or
`UpdateCheckResponse`), complete.

### 7.4 `start` / `stop` / `restart <name>`

```
clawcli start <name>
clawcli stop  <name> [--all] [--timeout-secs <N>]
clawcli restart <name>
```

- `--all` (stop only): stop every registered instance
- `--timeout-secs`: hard limit on backend stop call (default 60)

Emits: progress, complete. No data.

### 7.5 `launch <name>`

```
clawcli launch <name> [--probe-secs <N>] [--no-probe]
```

- `--probe-secs`: how long to wait for the gateway port to open (default 120)
- `--no-probe`: spawn daemons but don't wait for ready port

Emits: progress, data (`LaunchReport {ready_port, started_processes}`), complete.

### 7.6 `list`

```
clawcli list [--filter <kind>] [--include-broken]
```

- `--filter`: `native` | `lima` | `wsl2` | `podman` | `running` | `stopped`
- `--include-broken`: include instances with broken backend state

Emits: data (`ListResponse`), complete.

### 7.7 `status <name>`

```
clawcli status <name> [--no-probe]
```

- `--no-probe`: skip backend availability check (registry only)

Emits: data (`StatusResponse`), complete.

### 7.8 `info <name>`

```
clawcli info <name>
```

Emits: data (`InstanceConfig`), complete. Always registry-only — no
backend probing, fast.

### 7.9 `exec <name> -- <cmd> [args...]`

```
clawcli exec <name> [--no-tty] -- <cmd> [args...]
```

- `--no-tty`: never allocate a tty (default: auto)
- `--`: end-of-flag delimiter (rest is the command)

Emits: data (`ExecResult`), complete.

### 7.10 `shell <name>`

```
clawcli shell <name>
```

Spawns interactive shell. Inherits stdin/stdout/stderr. Does NOT
emit JSON events even with `--json` (it's a tty-passthrough).

### 7.11 `logs <name>`

```
clawcli logs <name> [--follow] [--tail <N>] [--since <duration>]
```

- `--follow`: stream new lines (Ctrl-C to exit)
- `--tail`: starting offset in lines (default: 200)
- `--since`: e.g. `5m`, `1h` (default: unlimited from `--tail` start)

Emits: data per line (when --json), complete on stream end / Ctrl-C.

### 7.12 `doctor`

```
clawcli doctor [<name>] [--all] [--fix]
```

- `<name>`: scope to one instance (default: composite host doctor)
- `--all`: iterate every registered instance
- `--fix`: apply repair recipes for fixable issues

Emits: data (`DoctorReport` or `[DoctorReport]`), complete.

### 7.13 `net-check`

```
clawcli net-check [--mode host|sandbox] [<name>] [--proxy-url <URL>]
```

- `--mode`: `host` (default) or `sandbox` (requires `<name>`)
- `--proxy-url`: ad-hoc proxy override

Emits: data (`NetCheckReport`), complete.

### 7.14 `token <name>`

```
clawcli token <name>
```

Emits: data (`{token: string}`), complete. Plain stdout in non-JSON mode.

### 7.15 `export <name>`

```
clawcli export <name> --output <path>
                     [--include-workspace] [--scrub-credentials]
```

- `--include-workspace`: include the host workspace dir (default: yes)
- `--scrub-credentials`: strip API keys / tokens from snapshot (default: yes)

Emits: progress, data (`ExportReport`), complete.

### 7.16 `import`

```
clawcli import --file <path> --name <new> [--port <P>]
```

Emits: progress, data (`ImportReport`), complete.

### 7.17 `config <sub>`

```
clawcli config show
clawcli config get <key>
clawcli config set <key> <value>
clawcli config unset <key>
clawcli config validate
```

Keys use dot notation: `proxy.http`, `bridge.port`,
`mirrors.alpine_repo`. Values are strings (parsed per schema).

### 7.18 `system <sub>`

```
clawcli system info
clawcli system version
clawcli system state
```

- `info`: SystemInfo (OS, arch, memory, disk, sandbox availability)
- `version`: clawcli version + commit + capabilities
- `state`: LaunchState (FirstRun / NotInstalled / Ready)

### 7.19 Noun groups

```
clawcli claw    {list, version, doctor, status, logs, config}
clawcli sandbox {edit, rename, port, prereqs, stats, disk-usage}
clawcli native  {components, upgrade, repair}
clawcli download{list, fetch, check-connectivity, doctor}
clawcli instance{create, destroy, info, health}
clawcli proxy   {resolve, get, set, check, apply, clear, set-password}
clawcli bridge  {config, start, stop, status}
clawcli browser {status, hil-start, hil-resume}
```

Each subcommand is documented in `clawcli <noun> <sub> --help`.

---

## 8 Removed from v2

These v1 verbs/flags don't survive into v2. Each has a documented
replacement.

| v1 | replacement |
|---|---|
| `--mode sandbox|native` | `--backend lima|wsl2|podman|native` (explicit) |
| `--claw-type X` (flag) | positional `<claw>` |
| `--browser` | `--browser` (kept; same meaning, but bare presence) |
| `--image PATH` | dropped — offline install path is `import` |
| top-level `system-check` | `system info` |
| top-level `claw-types` | `claw list` |
| top-level `update-check <name>` | `upgrade <name> --check` |
| top-level `uninstall --name N` | `uninstall <N>` (positional) |
| `settings save <json-blob>` | `config set <key> <value>` (one key at a time) |
| `settings diagnose` | `doctor --all` |

---

## 9 Migration plan

1. **Stage 1 — design (this doc).** Lock the verb set + argv shapes.
2. **Stage 2 — `ExecutionContext` trait.** Refactor v2 core to expose
   the trait; impl `Lima/Wsl/Podman/Native`. Move `SandboxBackend`
   methods that are context-y onto the new trait. Keep backend trait
   for backend-specific ops (image export/import, ensure_prereqs).
3. **Stage 3 — wire types regenerated.** Drop v1-mirror types from
   `wire::*`; redefine clean v2 shapes per §5. Update verbs to emit them.
4. **Stage 4 — refactor verbs.** Each verb in `cli/src/cmd/` rewritten
   to match §7. Remove all v1-compat aliases.
5. **Stage 5 — Tauri thin shell.** GUI's IPC handlers rewritten to
   call the v2 verb names + parse v2 wire types. Frontend may need
   field renames (claw_type → claw, sandbox_type → backend, etc.) —
   that's a frontend change but contained in TypeScript types.
6. **Stage 6 — feature gaps documented.** GUI features that depended
   on v1-only behaviour (per-instance proxy override, OS-proxy auto-
   refresh, etc.) get listed in `v0.5.x-features.md` and ship in
   subsequent point releases.

This document is the contract. Implementation drift FROM it is a bug
in the implementation, not a feature.
