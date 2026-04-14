# 5. Packaging & Distribution

## 5.1 Sandbox Image Packaging

Export a running sandbox instance as a distributable image for offline installation.

### Usage

```bash
# Basic (no chromium, default instance)
bash tools/package-alpine.sh

# With Chromium + noVNC (~630MB extra)
bash tools/package-alpine.sh --chromium

# Custom instance and output dir
bash tools/package-alpine.sh --chromium my-instance ./dist
```

### Platform Output

| Platform | Format | Tool |
|----------|--------|------|
| macOS | Lima VM tar.gz (qcow2/vz disk) | `limactl` |
| Linux | Podman OCI image tar.gz | `podman commit/save` |
| Windows | WSL2 rootfs tar.gz | `wsl --export` |

### What's Included

- Alpine Linux base system
- Node.js + npm
- OpenClaw (installed version)
- MCP Bridge + HIL Skill plugins
- Chromium + noVNC (if `--chromium` flag used)

### Install from Image

```bash
clawenv install --image clawenv-default-20260414-macos-arm64.tar.gz
```

---

## 5.2 Native Bundle Packaging

Create a self-contained Node.js + OpenClaw bundle for native (no-sandbox) install.

### Usage

```bash
# Latest version
bash tools/package-native.sh

# Specific version
bash tools/package-native.sh 2026.4.12 ./dist

# With China mirrors
NODEJS_DIST_MIRROR=https://npmmirror.com/mirrors/node \
NPM_REGISTRY_MIRROR=https://registry.npmmirror.com \
bash tools/package-native.sh
```

Note: `--chromium` is ignored for native mode (uses system browser).

### Output

`clawenv-native-{version}-{platform}-{arch}.tar.gz` containing:
- `node/` — Node.js runtime
- `node_modules/` — OpenClaw + dependencies
- `manifest.toml` — Bundle metadata

### Install from Bundle

```bash
clawenv install --native-bundle clawenv-native-2026.4.12-macos-arm64.tar.gz
```

---

## 5.3 App Bundle (GUI Distribution)

Build the Tauri desktop application for distribution.

### macOS

```bash
# Prerequisites: Rust 1.88+, Node.js, npm
export PATH="$HOME/.cargo/bin:$PATH"
npm install
cargo tauri build
```

Output:
- `target/release/bundle/macos/ClawEnv.app`
- `target/release/bundle/dmg/ClawEnv_0.1.0_aarch64.dmg`

### Windows

```powershell
# Prerequisites: Rust, Node.js, LLVM (for ARM64), Visual Studio Build Tools
npm install
cargo tauri build
```

Output:
- `target\release\bundle\msi\ClawEnv_0.1.0_arm64_en-US.msi`
- `target\release\bundle\nsis\ClawEnv_0.1.0_arm64-setup.exe`

### Linux

```bash
# Prerequisites: Rust, Node.js, webkit2gtk-4.1-dev, libappindicator3-dev
npm install
cargo tauri build
```

Output:
- `target/release/bundle/deb/clawenv_0.1.0_amd64.deb`
- `target/release/bundle/appimage/ClawEnv_0.1.0_amd64.AppImage`

### Build Notes

- `beforeBuildCommand` in `tauri.conf.json` automatically:
  1. Builds `clawenv-cli` (release)
  2. Copies CLI as Tauri sidecar (`scripts/copy-cli-sidecar.cjs`)
  3. Builds frontend (`npm run build`)
- Rust version: tauri crate needs 1.88+ (`~/.cargo/bin/rustc`)
- On Windows ARM64: needs LLVM for C++ compilation

### Remote Windows Build (via SSH)

```bash
# From macOS, build on Windows VM
ssh clawenv@192.168.64.7 \
  "set \"PATH=C:\Program Files\Git\cmd;C:\Program Files\nodejs;C:\Users\clawenv\.cargo\bin;%PATH%\" && \
   cd C:\Users\clawenv\ClawEnv && git pull && npm install && cargo tauri build"
```

---

## 5.4 CI/CD

GitHub Actions workflow at `.github/workflows/ci.yml`:
- Runs `cargo check` for core, cli, and gui crates
- Runs `cargo test --workspace`
- Checks frontend TypeScript compilation
