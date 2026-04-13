# ClawEnv — Scripts

## Test Scripts

| Script | Purpose |
|--------|---------|
| `test-cli.sh` | CLI end-to-end test (system check → install → lifecycle → cleanup) |

### Three-Layer Test Architecture

| Layer | Command | Duration | Scope |
|-------|---------|----------|-------|
| L1 Unit | `cargo test -p clawenv-core` | <1s | Config, registry, version parsing |
| L2 CLI E2E | `cargo test -p clawenv-cli` | <2s | CLI binary output format, commands |
| L3 Real Install | `bash scripts/test-cli.sh` | 5-30min | Full install/lifecycle in real environment |

### L3 Test Usage

```bash
# Quick check — system exploration only (no install, ~5s)
bash scripts/test-cli.sh --skip-install --skip-cleanup

# Full native install test (~10min)
bash scripts/test-cli.sh --mode native

# Full sandbox install test (~20min)
bash scripts/test-cli.sh --mode sandbox

# JSON mode test
bash scripts/test-cli.sh --mode native --json

# Test specific claw
bash scripts/test-cli.sh --mode native --claw-type nanoclaw

# Keep instance after test (for debugging)
bash scripts/test-cli.sh --mode native --skip-cleanup --verbose
```

### Developer Mode: Step-by-Step Install

The test script exercises the `--step` flag for developer mode:

```bash
clawenv install --mode native --step prereq   # Check/install Node.js
clawenv install --mode native --step create    # Create directory + ensure Node
clawenv install --mode native --step claw      # npm install openclaw
clawenv install --mode native --step config    # Save instance config
clawenv install --mode native --step gateway   # Start gateway service
```

For sandbox mode:

```bash
clawenv install --mode sandbox --step prereq   # Check/install Lima/WSL2/Podman
clawenv install --mode sandbox --step create   # Create Alpine VM
clawenv install --mode sandbox --step claw     # Install claw inside VM
clawenv install --mode sandbox --step config   # Save instance config
clawenv install --mode sandbox --step gateway  # Start gateway + ttyd
```

## Package Scripts

| Script | Purpose |
|--------|---------|
| `package-alpine.sh` | Export sandbox VM as distributable tar.gz |
| `package-native.sh` | Create native offline install bundle |
| `win-remote.sh` | SSH remote build helper for Windows ARM64 |
