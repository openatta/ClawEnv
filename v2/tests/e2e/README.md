# ClawEnv v2 E2E smoke tests

Lifted from v1 `tests/e2e/`. Same harness shape (run.sh + lib/ + scenarios/), adapted for v2's CLI surface.

## Quick start

```bash
# Build the CLI (workspace lives at v2/Cargo.toml; binary is named
# clawcli but the cargo package is `clawops-cli`).
cd v2 && cargo build -p clawops-cli --release && cd ..

# Run one scenario
./v2/tests/e2e/run.sh smoke-mac-native-noproxy

# Run everything that fits the host (Mac scenarios skip on Linux, etc.)
./v2/tests/e2e/run.sh all

# Skip proxy variants (e.g. on a network without a configured proxy)
./v2/tests/e2e/run.sh --skip-proxy all

# Keep the isolated $HOME for post-mortem inspection
./v2/tests/e2e/run.sh --keep-home smoke-mac-sandbox-noproxy

# Override the binary (point at a specific build)
CLAWCLI_BIN=/path/to/clawcli ./v2/tests/e2e/run.sh smoke-mac-native-noproxy
```

## What's covered

| scenario                          | platform | wall   | notes |
|-----------------------------------|----------|--------|-------|
| smoke-mac-native-noproxy          | macOS    | ~30s   | host-mode net-check, no VM |
| smoke-mac-native-http-proxy       | macOS    | ~30s   | host-mode net-check via HTTP_PROXY |
| smoke-mac-sandbox-noproxy         | macOS    | ~8-10m | full Lima + openclaw install + in-VM probe |
| smoke-mac-sandbox-http-proxy      | macOS    | ~8-10m | same, via host's HTTP proxy |
| smoke-mac-upgrade                 | macOS    | ~10-15m | install old version → upgrade → assert version changed |
| smoke-mac-roundtrip               | macOS    | ~10-12m | install → export → destroy → import → start (P2-e) |
| smoke-mac-blocked-egress          | macOS    | ~90s   | negative test: install must fail fast under blocked proxy (P2-d) |
| smoke-linux-podman-noproxy        | Linux    | ~5-8m  | full rootless Podman + openclaw install |
| smoke-linux-podman-http-proxy     | Linux    | ~5-8m  | same, via $HTTP_PROXY |

Scenarios self-skip (exit 77) when the host doesn't match (uname / no proxy configured / no podman binary). The runner reports SKIPPED separately from PASS/FAIL.

## Deferred from v1

These v1 scenarios are **not** ported yet. They depend on infrastructure that v2 does not have and are tracked under Phase P3 / Phase M:

- `smoke-win-native-noproxy.sh` / `smoke-win-native-http-proxy.sh` — need the win-rsync + remote-build infra (`scripts/win-remote.sh`) which is v1-specific.

(The v1 `smoke-mac-import-export.sh` is now `smoke-mac-roundtrip.sh` and runs in v2 — done in P2-e.)

When those land, port the v1 scenario to use the new v2 verbs and remove the entry from this list.

## Key v1 → v2 syntax differences

| v1                                                     | v2                                          |
|--------------------------------------------------------|---------------------------------------------|
| `cli install --mode sandbox --claw-type openclaw …`    | `cli install openclaw --backend lima …`     |
| `cli install --mode native …`                          | `cli install <claw> --backend native …`     |
| `cli install … --step prereq` / `--step create`        | (full pipeline only — slower but real)      |
| `cli uninstall --name N`                               | `cli instance destroy N`                    |
| `cli net-check --probe apk,npm,git --proxy-url ""`     | `cli net-check --mode {host,sandbox} [N]`   |
| reads `config.toml` for instance entry                 | reads `instances.toml` (v2 source of truth) |

## Safety

- **HOME isolation**: every run creates `/tmp/clawenv-e2e-<suffix>/` and points `$HOME` there. Real `~/.clawenv` is untouched. `lib/isolate.sh` enforces a teardown-prefix whitelist; never edit that without a code review.
- **prewarm**: copies `~/.clawenv/{bin,share/lima,git,node}` into the isolated HOME so scenarios don't redownload limactl / dugite / mingit / node every run. `node_modules/openclaw` is excluded so npm installs are real.
- **preflight**: every scenario that needs upstream egress probes npm CDN + github + nodejs.org first. If any fail, the runner exits 2 (NOT skip) — you fix your network, then retry.
