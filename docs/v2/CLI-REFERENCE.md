# clawcli — Command Reference

`clawcli` is the v2 unified command-line interface for ClawEnv: it owns the entire sandbox install / launch / inspect / destroy lifecycle and is the single business-logic surface (the Tauri GUI is a thin shell that spawns `clawcli` as a sidecar). The command set is layered: **top-level verbs** (`install`, `start`, `launch`, …) cover task-oriented workflows; **noun subcommand groups** (`sandbox`, `proxy`, `download`, …) expose layer-direct primitives for advanced use. Pass `--json` to flip every verb into a line-delimited `CliEvent` stream that any consumer can parse — the human renderer is just a courtesy on top of the same payload.

Source of truth: `cli/src/main.rs` (top-level enum), `cli/src/cmd/*.rs` (subcommand modules), wire payload schemas in `core/src/wire/mod.rs`, design contract in `docs/v2/CLI-DESIGN.md`.

## Global flags

Defined in `cli/src/main.rs` as `clap` global args — every verb accepts them.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit line-delimited JSON `CliEvent` stream on stdout instead of human text. |
| `--quiet` | bool | `false` | Suppress `Info` text events; `Data`/`Error`/`Complete` still emit. |
| `--no-progress` | bool | `false` | Suppress `Progress` events; `Data` and `Complete` still emit. |
| `--instance <name>` | string | `default` | Fallback instance for verbs that take `<name>` positionally. Positional arg always wins. |

In addition, the CLI reads `RUST_LOG` (via `tracing_subscriber::EnvFilter`) for tracing log filtering — defaults to `warn,clawops_cli=info,clawops_core=info`. Logs go to stderr; never collide with `--json` stdout protocol.

## Top-level verbs

### `clawcli install`
End-to-end install: provision backend + deploy claw + register instance.

**Synopsis**: `clawcli install <claw> [flags]`

**Description**: Composes the full install pipeline (preflight → create VM → boot-verify → install claw → register). Streams `Progress` events for every stage; emits a final `Data` event with `instance + version_output + install_elapsed_secs`. Respects `--proxy-url` overlay without persisting to `config.toml`.

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<claw>` (positional) | string | required | Claw product id (e.g. `openclaw`, `hermes`). |
| `--name <N>` | string | `default` (or `--instance`) | Instance name (must be unique). |
| `--backend <B>` | enum | host default (lima on macOS, wsl2 on Windows, podman on Linux) | One of `native` \| `lima` \| `wsl2` \| `podman`. |
| `--version <V>` | string | `latest` | Claw version (semver, git ref, or `latest`). |
| `--port <P>` | u16 | `3000` | Host port to expose the gateway on. |
| `--cpus <N>` | u32 | `2` | VM cores (sandbox only; ignored for native). |
| `--memory-mb <N>` | u32 | `2048` | VM RAM in MiB. |
| `--browser` | bool | `false` | Install Chromium + noVNC bundle. |
| `--workspace <PATH>` | path | none | Host workspace dir to mount/expose. |
| `--proxy-url <URL>` | string | none | Per-install proxy override (`http://[user:pass@]host:port`). |
| `--autoinstall-deps` | bool | `false` | `--backend native` only — auto-install missing node/git into `~/.clawenv/{node,git}`. |
| `--dry-run` | bool | `false` | Render templates + describe; no backend invocation. |

**Examples**:
```bash
clawcli install openclaw --port 3001
clawcli install hermes --name myhermes --backend lima --memory-mb 4096
clawcli install openclaw --backend native --autoinstall-deps --dry-run
```

**JSON output**: streaming `Progress` events (stages: `validate`, `create-vm`, `boot-verify`, `install-claw`, `verify`, `done`) followed by one `Data` event:
```json
{"type":"data","data":{"instance":{...InstanceConfig},"version_output":"openclaw 0.5.2","install_elapsed_secs":312}}
```

### `clawcli uninstall`
End-to-end teardown: stop daemons + destroy backend + drop registry record.

**Synopsis**: `clawcli uninstall <name> [--keep-bundle <PATH>]`

**Description**: Routes to `InstanceOrchestrator::destroy` and streams progress. With `--keep-bundle`, runs an `export` first (atomic backup) and aborts the destroy if export fails. The instance name is **required** (no fallback to `--instance`) — guards against destroying `default` by accident.

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional) | string | required | Instance to destroy. |
| `--keep-bundle <PATH>` | path | none | Run `export <name> --output PATH` before destroy; abort on failure. |

**Examples**:
```bash
clawcli uninstall myhermes
clawcli uninstall openclaw-old --keep-bundle ~/snapshots/openclaw-old.tar.gz
```

**Notes**: v1's `uninstall --name N` flag form is removed — name is now positional. The `--force` flag listed in `CLI-DESIGN.md` §7.2 is not implemented in v2 (no confirmation prompt to bypass).

### `clawcli upgrade`
Upgrade an instance's claw to a new version (reuses VM).

**Synopsis**: `clawcli upgrade [<name>] [--to <version>] [--check]`

**Description**: With `--check`, probes the upstream registry (npm/PyPI/GitHub depending on the claw's package manager) and emits an `UpdateCheckResponse` without touching the VM — replaces v1's `update-check` verb. Without `--check`, runs `InstanceOrchestrator::upgrade` and streams progress.

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional) | string | `--instance` / `default` | Instance to upgrade. |
| `--to <V>` | string | `latest` | Target version. |
| `--check` | bool | `false` | Probe latest only; no install. |

**Examples**:
```bash
clawcli upgrade default --to 0.6.0
clawcli upgrade myhermes --check
```

**JSON output**:
- Without `--check`: `Progress` events + final `Data` `{instance,previous_version,new_version,upgrade_elapsed_secs}`.
- With `--check`: one `Data` event matching `wire::UpdateCheckResponse` (`current`, `latest`, `has_upgrade`, `is_security_release`, `changelog`).

**Notes**: v1's top-level `update-check <name>` is replaced by `upgrade <name> --check`.

### `clawcli start`
Bring an instance's backend online (start VM; no daemon spawn).

**Synopsis**: `clawcli start [<name>]`

**Description**: Calls the backend's `start` op. For native instances bails with an explanatory error (no VM to start). Does **not** spawn the gateway daemons — use `launch` for that.

| Arg | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Instance to start. |

```bash
clawcli start
clawcli start myhermes
```

### `clawcli stop`
Take an instance's backend offline.

**Synopsis**: `clawcli stop [<name>] [--all] [--timeout-secs <N>]`

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Instance to stop. |
| `--all` | bool | `false` | Stop every registered instance (per-instance failures logged, loop continues). |
| `--timeout-secs <N>` | u64 | `60` | Wall-clock budget on the backend stop call. |

```bash
clawcli stop default
clawcli stop --all
clawcli stop wedged-vm --timeout-secs 10
```

**JSON output (`--all` mode)**: `Data` event with an array of `{instance, status, reason?}` records (status = `stopped` | `skipped` | `error`).

### `clawcli restart`
Restart an instance's backend.

**Synopsis**: `clawcli restart [<name>]`

```bash
clawcli restart default
```

### `clawcli launch`
Spawn the gateway/dashboard daemons inside the instance and probe `ready_port`.

**Synopsis**: `clawcli launch [<name>] [--probe-secs <N>] [--no-probe]`

**Description**: Calls `InstanceOrchestrator::launch_with_probe`. Emits a `LaunchReport` with `instance_name`, `started_processes` (e.g. `["gateway","dashboard"]`), and `ready_port`.

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Instance to launch. |
| `--probe-secs <N>` | u64 | `120` | Reserved for future use; orchestrator currently bakes its own 120s budget. |
| `--no-probe` | bool | `false` | Spawn daemons but skip the HTTP readiness probe. |

```bash
clawcli launch default
clawcli launch myhermes --no-probe
```

### `clawcli list`
List registered instances. See wire type `wire::ListResponse` in `core/src/wire/mod.rs`.

**Synopsis**: `clawcli list [--filter <BACKEND>] [--include-broken]`

| Flag | Type | Default | Description |
|---|---|---|---|
| `--filter <BACKEND>` | string | none | Match `backend` (`lima`/`wsl2`/`podman`/`native`). |
| `--include-broken` | bool | `false` | Keep instances whose probe surfaces `broken` or `missing`. |

```bash
clawcli list
clawcli list --filter lima
clawcli list --include-broken --json
```

**JSON output**: `Data` event = `wire::ListResponse { instances: [InstanceSummary] }`.

### `clawcli status`
Show one instance: VM state + claw + ports + capabilities. See `wire::StatusResponse`.

**Synopsis**: `clawcli status [<name>] [--no-probe]`

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Instance to inspect. |
| `--no-probe` | bool | `false` | Skip backend availability probe (registry-only, fast). |

```bash
clawcli status
clawcli status myhermes --no-probe --json
```

**JSON output**: `wire::StatusResponse` — flattens `InstanceSummary` fields inline plus optional `capabilities: { rename, resource_edit, port_edit, snapshot }`.

### `clawcli info`
Registry-only instance record (never probes the backend).

**Synopsis**: `clawcli info <name>`

**Description**: Emits the raw `InstanceConfig` so scripts can read the persisted record without race conditions on a live VM.

```bash
clawcli info default
clawcli info myhermes --json
```

**JSON output**: `Data` = `instance::InstanceConfig` (name, claw, claw_version, backend, sandbox_instance, ports[], browser, proxy, created_at, updated_at, note).

### `clawcli logs`
Tail or follow gateway/dashboard logs.

**Synopsis**: `clawcli logs <name> [--follow] [--tail <N>] [--since <DUR>]`

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` (positional) | string | required | Instance whose logs to read. |
| `--follow` | bool | `false` | Stream new lines (Ctrl-C to exit). |
| `--tail <N>` | u32 | `200` | Number of trailing lines per file. |
| `--since <DUR>` | string | none | Reserved (advisory; currently stripped). |

```bash
clawcli logs default
clawcli logs default --follow --tail 500
clawcli logs myhermes --json
```

**JSON output**: one `Data` event per emitted batch / line. Payload = `wire::LogResponse { content }`. With `--follow`, each new line is a separate Data event so the GUI tail panel can scroll without buffering.

### `clawcli exec`
Run a non-interactive command inside an instance.

**Synopsis**: `clawcli exec [<name>] -- <cmd> [args...]`

**Description**: Routes through `ExecutionContext` — works for both sandboxed and native instances. For an attached TTY use `clawcli shell`. The `--` delimiter separates the verb's flags from the inner argv.

| Arg | Type | Description |
|---|---|---|
| `<name>` | string | Instance name (positional); falls back to `--instance`. |
| `-- <cmd> [args...]` | argv | Command to run inside the instance. |

```bash
clawcli exec default -- ls /workspace
clawcli exec myhermes -- node --version
clawcli exec default --json -- cat /etc/os-release
```

**JSON output**: `Data` = `wire::ExecResult { stdout, stderr, exit_code }`. In non-JSON mode, raw stdout is printed unformatted (so piping works).

**Notes**: The `--no-tty` flag listed in `CLI-DESIGN.md` §7.9 is not implemented in v2.

### `clawcli shell`
Open an interactive shell inside an instance.

**Synopsis**: `clawcli shell [<name>]`

**Description**: TTY passthrough — does **not** emit JSON events, even with `--json`. Inherits stdin/stdout/stderr. Bails on native instances. Per backend: `limactl shell`, `wsl -d <name>`, `podman exec -it <name> /bin/sh`.

```bash
clawcli shell
clawcli shell myhermes
```

**Exit codes**: forwards the shell's exit status; reports failure when the shell exits non-zero.

### `clawcli doctor`
Aggregate diagnostics across native, sandbox, and download layers.

**Synopsis**: `clawcli doctor [<name>] [--all] [--fix]`

| Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Scope to one instance. |
| `--all` | bool | `false` | Iterate every registered instance. |
| `--fix` | bool | `false` | Apply repair recipes for fixable issues; re-doctor afterward. |

```bash
clawcli doctor
clawcli doctor myhermes --fix
clawcli doctor --all --json
```

**JSON output**: `Data` = `CompositeDoctor { name, native, sandbox?, download, overall_healthy }` (or array thereof with `--all`). Layer reports (`NativeDoctorReport`, `SandboxDoctorReport`, `DownloadDoctorReport`) live in `core/src/native_ops`, `core/src/sandbox_ops`, `core/src/download_ops`.

### `clawcli net-check`
Connectivity probes from host or from inside a sandbox VM.

**Synopsis**: `clawcli net-check [--mode host|sandbox] [<name>] [--proxy-url <URL>]`

| Flag | Type | Default | Description |
|---|---|---|---|
| `--mode <M>` | enum | `host` | `host` (probe from host process) or `sandbox` (probe from inside VM via in-VM curl). |
| `<name>` | string | `--instance` / `default` | Required for `--mode sandbox`. |
| `--proxy-url <URL>` | string | none | Override `HTTP(S)_PROXY` env vars for the probe; restored on exit. |

```bash
clawcli net-check
clawcli net-check --mode sandbox default
clawcli net-check --proxy-url http://proxy.corp:3128 --json
```

**JSON output**: `Data` = `wire::NetCheckReport { origin, all_reachable, hosts: [NetCheckHostResult], suggestion? }`.

**Exit codes**: exits **1** when `all_reachable=false` so install orchestration / CI can gate on it.

### `clawcli token`
Read the gateway auth token from inside an instance.

**Synopsis**: `clawcli token [<name>]`

```bash
clawcli token default
clawcli token myhermes --json
```

**JSON output**: `Data` = `{ "instance": <name>, "token": <string> }`. In non-JSON mode the token is plain stdout (no decoration), suitable for command substitution.

### `clawcli export`
Export an instance to a portable bundle.

**Synopsis**: `clawcli export [<name>] --output <PATH>`

**Description**: Stops the VM (export of a running VM risks a torn snapshot), shells the backend's `export_image`, builds `clawenv-bundle.toml`, then `tar czf <output>` wraps the manifest + payload. Native instances are not exportable (no VM image).

| Arg / Flag | Type | Default | Description |
|---|---|---|---|
| `<name>` | string | `--instance` / `default` | Instance to export. |
| `--output <PATH>` | path | required | Output file (`.tar.gz` conventional, not enforced). |

```bash
clawcli export default --output ~/backups/default.tar.gz
clawcli export myhermes --output bundle.tar.gz --json
```

**JSON output**: streaming `Info` events for each stage; final `Data`:
```json
{"instance":"default","claw":"openclaw","claw_version":"0.5.2","backend":"lima","output":"..."}
```

**Notes**: The `--include-workspace` and `--scrub-credentials` flags listed in `CLI-DESIGN.md` §7.15 are not implemented in v2.

### `clawcli import`
Import a bundle as a new instance.

**Synopsis**: `clawcli import --file <PATH> --name <NEW> [--port <P>]`

**Description**: Validates the manifest, restores the backend image via `backend.import_image`, inserts a registry record. Native bundles are not importable.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--file <PATH>` | path | required | Bundle from `clawcli export`. |
| `--name <NEW>` | string | required | Instance name to register under. |
| `--port <P>` | u16 | `3000` | Host gateway port. |

```bash
clawcli import --file ~/backups/default.tar.gz --name restored
clawcli import --file bundle.tar.gz --name newvm --port 3010
```

**JSON output**: `Data` = `{ instance: InstanceConfig, manifest: { schema_version, clawenv_version, claw_type, claw_version, sandbox_type, source_platform, created_at } }`.

## Noun subcommand groups

### `clawcli config`
Read/write `~/.clawenv/config.toml`.

Sub-verbs: `show`, `get`, `set`, `unset`, `validate`. Keys use dot notation.

#### `clawcli config show`
**Synopsis**: `clawcli config show`
Dump effective config (resolved global + env overlay) as flat `key = value` pairs. The keychain-backed `proxy.auth_password` always renders as `(in keychain)`.

```bash
clawcli config show
clawcli config show --json
```

#### `clawcli config get <key>`
Read one key. Errors when the key is unknown.
```bash
clawcli config get proxy.http
```

#### `clawcli config set <key> <value>`
Write one key. The value is parsed per the field's schema (string / bool / u16). Booleans accept `true|false|yes|no|1|0|on|off`. `proxy.auth_password` writes to the OS keychain (never to TOML).

Known keys: `proxy.enabled`, `proxy.http`, `proxy.https`, `proxy.no_proxy`, `proxy.auth_required`, `proxy.auth_user`, `proxy.auth_password`, `mirrors.alpine_repo`, `mirrors.npm_registry`, `bridge.enabled`, `bridge.port`, plus scalar `[clawenv]` fields `language` and `theme`.

```bash
clawcli config set proxy.http http://proxy.corp:3128
clawcli config set bridge.port 8080
echo -n SECRET | clawcli proxy set-password --stdin    # password path
```

#### `clawcli config unset <key>`
Only `proxy.auth_password`, `language`, and `theme` are unsettable. For other keys, use `set <key> <default>` to revert.

#### `clawcli config validate`
Parse + schema-check; emits `config: ok` on success or the first error.

### `clawcli system`
Host introspection.

#### `clawcli system info`
**Synopsis**: `clawcli system info`
Probes OS, arch, memory, disk free, sandbox availability. Emits `wire::SystemInfo { os, arch, memory_gb, disk_free_gb, sandbox_backend, sandbox_available, checks: [SystemCheckItem] }`.

```bash
clawcli system info --json
```
**Notes**: replaces v1's `system-check`.

#### `clawcli system version`
**Synopsis**: `clawcli system version`
Emits `wire::VersionInfo { clawcli_version, commit, build_date, capabilities }`. `commit` and `build_date` come from build-time env (`CLAWCLI_GIT_COMMIT`, `CLAWCLI_BUILD_DATE`); both default to `"unknown"` for dev builds.

#### `clawcli system state`
**Synopsis**: `clawcli system state`
Emits `LaunchState` (`FirstRun` / `NotInstalled` / `Ready`) for the GUI's startup routing. Replaces v1's `launcher-state`.

### `clawcli claw`
Invoke a claw product's own CLI (passthrough).

#### `clawcli claw list`
Emits `wire::ClawTypesResponse { claw_types: [ClawTypeInfo] }`. Each entry: `id, display_name, package_manager, package_id, default_port, supports_mcp, supports_browser, supports_native, has_gateway_ui`.

```bash
clawcli claw list --json
```
**Notes**: replaces v1's `claw-types`.

#### `clawcli claw update <claw>`
**Synopsis**: `clawcli claw update <claw> [--yes] [--json] [--dry-run] [--channel <C>] [--tag <T>] [--no-restart] [--execute] [--backend <B>]`

| Flag | Type | Description |
|---|---|---|
| `--yes` | bool | non-interactive |
| `--json` | bool | claw's own JSON output |
| `--dry-run` | bool | claw-side dry run |
| `--channel <C>` | string | release channel |
| `--tag <T>` | string | version tag |
| `--no-restart` | bool | skip post-update restart |
| `--execute` | bool | run the command (default: preview only) |
| `--backend <B>` | enum (`lima`/`wsl2`/`podman`) | runner backend; absent = host-native |

Without `--execute`, emits a `CommandPreview { claw, binary, args, timeout_secs, output_format }`. With `--execute`, runs the spec and emits `ExecutionReport { claw, runner, exit_code, duration_ms, was_cancelled, was_timed_out, stdout, stderr, structured? }`.

```bash
clawcli claw update openclaw
clawcli claw update openclaw --execute --backend lima
```

#### `clawcli claw doctor <claw> [--fix] [--json] [--execute] [--backend <B>]`
Same preview-vs-execute semantics. `--fix` triggers repair, `--json` asks the claw for structured output.

#### `clawcli claw status <claw> [--execute] [--backend <B>]`
Preview or execute the claw's `status` subcommand.

#### `clawcli claw version <claw> [--execute] [--backend <B>]`
Preview or execute the claw's `version` subcommand.

#### `clawcli claw logs <claw> [--tail <N>] [--follow] [--level <L>] [--execute] [--backend <B>]`
Preview or execute the claw's `logs` subcommand.

#### `clawcli claw config <claw> <op> [--execute] [--backend <B>]`
Sub-ops:
- `get <key>`
- `set <key> <value>`
- `list`

```bash
clawcli claw config openclaw get model.default
clawcli claw config openclaw set model.default claude-sonnet-4-7 --execute --backend lima
```

### `clawcli sandbox`
Sandbox VM ops not part of the lifecycle.

Sub-verbs: `status`, `start`, `stop`, `restart`, `port`, `doctor`, `repair`, `stats`, `list`, `rename`, `edit`, `prereqs`, `disk-usage`. Every sub-verb takes `--backend <BackendSel>` (`lima` | `wsl2` | `podman`); when omitted, the host default is used. The instance name comes from the global `--instance` flag.

#### `clawcli sandbox status [--backend <B>]`
VM status + capabilities (CPU cores, memory, disk, IP, state).
```bash
clawcli sandbox status --backend lima
```

#### `clawcli sandbox start | stop | restart [--backend <B>]`
Backend lifecycle ops independent of registry.
```bash
clawcli sandbox stop --backend podman
```

#### `clawcli sandbox port <op>`
Port forward management. Sub-ops:
- `list [--backend <B>]` — emits `Vec<PortForward { host, guest, native_id }>`.
- `add <host> <guest> [--backend <B>]`
- `remove <host> [--backend <B>]`

```bash
clawcli sandbox port list --backend lima
clawcli sandbox port add 8080 80 --backend lima
clawcli sandbox port remove 8080 --backend lima
```

#### `clawcli sandbox doctor [--backend <B>]`
Per-backend diagnostic. Emits `SandboxDoctorReport { backend, instance_name, issues: [{id, severity, message, repair_hint?}], checked_at }`.

#### `clawcli sandbox repair <issue_ids...> [--backend <B>]`
Apply repair recipes for given issue IDs (e.g. `vm-not-running`).

#### `clawcli sandbox stats [--backend <B>]`
Resource usage. Emits `wire::SandboxStats { backend, instance, cpu_percent, memory_used_mb, memory_limit_mb, disk_used_gb, disk_total_gb }`.

#### `clawcli sandbox list [--backend <B>]`
Inventory ALL VMs/containers known to the host backend (managed by v2 or not). Emits `wire::SandboxListResponse { backend, vms: [SandboxVmInfo { name, status, managed, instance_name }] }`. Useful for finding orphan VMs.

```bash
clawcli sandbox list --backend lima
```

#### `clawcli sandbox rename --from <A> --to <B> [--backend <B>]`
Rename a sandbox VM. Lima supports it via `limactl`; WSL/Podman bail with guidance to recreate.

#### `clawcli sandbox edit [--cpus <N>] [--memory-mb <N>] [--disk-gb <N>] [--backend <B>]`
Edit resource allocation. Lima only for in-place edits — rewrites top-level scalars in `lima.yaml` and instructs the user to `clawcli restart <inst>`. WSL/Podman bail with an explanation; `--disk-gb` is attempted via `resize_disk` first when supported.

```bash
clawcli sandbox edit --cpus 4 --memory-mb 8192 --backend lima
```

#### `clawcli sandbox prereqs [--backend <B>]`
Idempotent install of host-side prerequisites (limactl on macOS, dism/WSL on Windows, podman+uidmap on Linux).

#### `clawcli sandbox disk-usage [--backend <B>]`
Reports disk usage of the backend's data dir (`~/.lima`, `~/.clawenv/podman`, `~/.clawenv/wsl`). Output: `{ backend, path, size }`. Uses `du -sh` on Unix, PowerShell on Windows.

### `clawcli native`
Host-runtime ops for native (no-VM) installs.

#### `clawcli native status`
Emits `NativeStatus { clawenv_home, home_exists, total_disk_bytes, node?: { version, path, healthy }, git?: { version, path, healthy } }`.

#### `clawcli native components`
Tabular list of components (name, version, path, healthy, size).

#### `clawcli native doctor`
Emits `NativeDoctorReport { issues: [{id, severity, message, repair_hint?}], checked_at }`.

#### `clawcli native repair <issue_ids...>`
Stage A: returns `Unsupported` for most issues today. Reserved.

#### `clawcli native upgrade <what>`
Sub-ops:
- `node [--to <version>]`
- `git [--to <version>]`

`--to` defaults to `Latest`. Stage A: returns `Unsupported`. Reserved.

```bash
clawcli native status
clawcli native upgrade node --to 20
```

### `clawcli download`
Artifact catalog: list, fetch, doctor.

#### `clawcli download list [--os <OS>] [--arch <ARCH>]`
Tabular list of catalog entries with `name, version, os, arch, kind, size`.

#### `clawcli download cache <op>`
Sub-ops:
- `list` — currently cached items.
- `verify` — verify cached items against catalog sha256; emits per-item `{name,version,verified}`.
- `prune [--keep <N>]` — prune old versions, keeping N most recent per artifact (default `--keep 2`).

#### `clawcli download fetch <name> [--version <V>] [--to <PATH>]`
Fetch an artifact into the cache, or to `--to` if provided. Emits `{ "path": <resolved path> }`.

```bash
clawcli download fetch lima-vm --version 1.0.0
clawcli download fetch openclaw-image --to ./vendored.qcow2
```

#### `clawcli download check-connectivity`
Probe each host in the catalog.

#### `clawcli download preflight`
Fast go/no-go probe against the 3 load-bearing hosts (Alpine CDN, npm, GitHub). Cheaper than `check-connectivity`. Emits `wire::NetCheckReport`. **Exit 1** when any host is unreachable.

#### `clawcli download doctor`
Emits `DownloadDoctorReport`.

### `clawcli instance`
Registry-only operations (no backend touch).

#### `clawcli instance list`
List registered instances (raw orchestrator output, distinct from top-level `list`).

#### `clawcli instance info <name>`
Single registry record.

#### `clawcli instance create`
**Synopsis**:
```
clawcli instance create --name <N> --claw <C> --backend <B>
                        [--sandbox-instance <S>] [--port <HOST:GUEST[:LABEL]>]…
                        [--note <TEXT>] [--autoinstall-deps]
```

| Flag | Type | Default | Description |
|---|---|---|---|
| `--name <N>` | string | required | Registry name. |
| `--claw <C>` | string | required | Claw id. |
| `--backend <B>` | enum (`native`/`lima`/`wsl2`/`podman`) | required | Backend kind. |
| `--sandbox-instance <S>` | string | `default` | VM/container name (sandboxed only). |
| `--port <SPEC>` (repeatable) | `host:guest[:label]` | — | Port forwards. |
| `--note <TEXT>` | string | `""` | Free-text note. |
| `--autoinstall-deps` | bool | `false` | Native-only auto-install of node/git. |

```bash
clawcli instance create --name myinst --claw openclaw --backend lima \
  --port 3000:3000:gateway --port 7681:7681:ttyd
```

#### `clawcli instance destroy <name>`
Streams progress and emits `DestroyReport`. Same pipeline as `uninstall` minus the bundle option.

#### `clawcli instance health`
Cross-layer composed health check for the current `--instance`. Emits `CompositeHealth { instance, native, sandbox, download, overall_healthy }`.

### `clawcli proxy`
Proxy config + apply.

#### `clawcli proxy resolve [--scope installer|native|sandbox] [--backend <B>]`
Show the `ProxyTriple` that would be applied for a given scope. `--scope sandbox` honours `--backend`; defaults to host backend.

```bash
clawcli proxy resolve --scope sandbox --backend lima
```

#### `clawcli proxy set-password --stdin`
Store the global proxy password in the keychain. Reads from stdin (the trailing `\n`/`\r` is stripped).

```bash
echo -n SECRET | clawcli proxy set-password --stdin
```

Without `--stdin`, the command bails with usage hints (no interactive prompt today).

#### `clawcli proxy clear-password`
Delete the global proxy password from the keychain.

#### `clawcli proxy apply [--backend <B>]`
Resolve `Scope::RuntimeSandbox` and write `/etc/environment` + `/etc/profile.d/proxy.sh` inside the sandbox. Bails when no proxy is configured.

#### `clawcli proxy clear [--backend <B>]`
Remove the proxy files written by `apply`.

#### `clawcli proxy get <name>`
Read effective proxy for an instance (resolves global config + env vars to a backend-specific triple). Emits `{ instance, backend, configured: bool, effective: ProxyTriple? }`.

```bash
clawcli proxy get default
```

#### `clawcli proxy set [<name>] --url <URL> [--no-proxy <LIST>] [--no-apply]`
Persist `[clawenv.proxy]` in `config.toml` and (unless `--no-apply`) push to a running instance's VM. `--url ""` disables.

```bash
clawcli proxy set default --url http://user:pass@proxy.corp:3128
clawcli proxy set --url http://proxy:3128 --no-apply
```

#### `clawcli proxy check <name>`
Probe an instance VM for `/etc/profile.d/proxy.sh`. Emits `{ instance, backend, present, contents? }`.

### `clawcli bridge`
AttaRun bridge daemon configuration.

#### `clawcli bridge config`
Read `[clawenv.bridge]` section from `~/.clawenv/config.toml`. Emits the BridgeConfig shape; missing file/section returns the default config.

#### `clawcli bridge save-config [--stdin] [<json>]`
Replace `[clawenv.bridge]`. With `--stdin`, reads JSON from stdin; otherwise expects JSON as the positional arg. Other config sections are preserved verbatim.

```bash
clawcli bridge config --json
clawcli bridge save-config --stdin <<< '{"enabled":true,"port":8080}'
```

**Notes**: The `start`/`stop`/`status` sub-verbs listed in `CLI-DESIGN.md` §2.6 are not implemented in v2 — bridge daemon lifecycle is owned by the host service manager (launchd / systemd / Task Scheduler), per the architecture rule that the bridge is an independent daemon (CLAUDE.md rule 10).

### `clawcli browser`
Chromium HIL state machine inside a sandbox instance. Native instances are not supported.

#### `clawcli browser status [<name>]`
Reports `BrowserStatus`:
- `Stopped`
- `Headless { cdp_port }`
- `Interactive { novnc_url }`

#### `clawcli browser hil-start [<name>]`
Switch from headless → noVNC HIL mode for human takeover. Emits the noVNC URL the user should open. The VNC websocket port defaults to `6080`.

#### `clawcli browser hil-resume [<name>]`
Switch back from noVNC → headless after the user finishes HIL.

```bash
clawcli browser status default --json
clawcli browser hil-start default
clawcli browser hil-resume default
```

## Wire protocol (`--json` mode)

Defined in `cli/src/output.rs` as `enum CliEvent`. When `--json` is set, every line on stdout is one `CliEvent` JSON object — a tagged union with snake_case `type`:

```json
{"type":"progress","stage":"install-claw","percent":80,"message":"npm install …"}
{"type":"info","message":"stopping VM (export of running VM risks torn snapshot)"}
{"type":"data","data":{ /* arbitrary verb-specific payload */ }}
{"type":"complete","message":"ok"}
{"type":"error","message":"instance `foo` not found","code":"instance-not-found"}
```

- **progress** — stage milestone. `stage` is a stable lowercase-kebab id. `percent` is monotonic non-decreasing within a verb.
- **info** — informational message.
- **data** — structured result. Conforms to the verb's documented response type (see schemas in `core/src/wire/mod.rs`: `ListResponse`, `StatusResponse`, `LogResponse`, `ExecResult`, `NetCheckReport`, `ExportReport`, `ImportReport`, `UpdateCheckResponse`, `SystemInfo`, `VersionInfo`, `ClawTypesResponse`, `SandboxListResponse`, `SandboxStats`, …).
- **complete** — normal exit signal. Always emitted on success.
- **error** — failure signal; `code` may be omitted (`#[serde(skip_serializing_if = "Option::is_none")]`).

Contract: every `--json` invocation emits **at most one** `complete` event, **at most one** `error` event, **never both**. Process exit code mirrors the event (`complete` → 0, `error` → 1). Streaming verbs (`install`, `upgrade`, `export`, `logs --follow`) emit many `progress`/`data` events; one-shot verbs emit a single `data` plus `complete`.

Idle timeout: GUI's `run_cli_streaming` SIGTERMs the child after 240s of no `progress` events. Long-running steps emit heartbeat progress every ≤120s.

`shell` is the sole exception — it's a TTY passthrough and emits no JSON events even with `--json`.

## Exit codes

| Code | When |
|---|---|
| `0` | Normal success (`Complete` event emitted in JSON mode). |
| `1` | Any unrecoverable error. Used by every verb that bails via `anyhow::Error`. Also: `net-check` and `download preflight` exit `1` when reachability fails; `claw` execute path exits `1` on non-zero child exit. |
| `2` (effective) | `claw` execute reports `exit_code: -2` in the structured payload to signal a runner-layer error (vs a legitimate child non-zero); the process still exits `1`. |

`shell` forwards the inner shell's exit status (best-effort; reports failure when non-zero).

## Environment variables

CLI- and core-read environment variables:

| Variable | Read by | Effect |
|---|---|---|
| `CLAWENV_HOME` | `core/src/paths/mod.rs` | Override the `~/.clawenv` root (used by tests for isolation). |
| `LIMA_HOME` | `core/src/paths/mod.rs` | Override the Lima home directory. |
| `RUST_LOG` | `tracing_subscriber::EnvFilter` (main.rs) | Tracing log filter. Default: `warn,clawops_cli=info,clawops_core=info`. |
| `HTTP_PROXY` / `HTTPS_PROXY` / `http_proxy` / `https_proxy` | preflight, install, `net-check --proxy-url` | Standard proxy env vars; `net-check --proxy-url` overlays them and restores on exit. |
| `NO_PROXY` / `no_proxy` | proxy resolver | Standard noproxy list. |
| `HOME` (Unix) / `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH` (Windows) | `core/src/paths/mod.rs` | Resolve user home when `CLAWENV_HOME` is unset. |
| `CLAWCLI_GIT_COMMIT` | build-time, `system version` | Embedded in `VersionInfo.commit`. Defaults to `"unknown"`. |
| `CLAWCLI_BUILD_DATE` | build-time, `system version` | Embedded in `VersionInfo.build_date`. Defaults to `"unknown"`. |
| `CARGO_PKG_VERSION` | build-time, `system version` | Embedded in `VersionInfo.clawcli_version`. |
