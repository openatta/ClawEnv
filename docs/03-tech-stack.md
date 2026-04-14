# 3. Tech Stack

## 3.1 Core Stack

```
Backend:     Rust 2021 edition
GUI:         Tauri v2 (native WebView)
Frontend:    SolidJS + TailwindCSS v4 + TypeScript
CLI:         clap v4 (derive mode)
Config:      TOML (~/.clawenv/config.toml)
Sandbox:     Alpine Linux (unified base)
             WSL2 (Windows) / Lima VZ (macOS) / Podman (Linux)
```

## 3.2 Rust Dependencies

### core/Cargo.toml
```
tokio, async-trait     # Async runtime
serde, toml, serde_json # Serialization
reqwest (rustls-tls)   # HTTP client (update checks, bridge)
axum, tower, tower-http # Bridge HTTP server
semver                 # Version comparison
keyring                # System keychain
chrono                 # Timestamps
anyhow, thiserror      # Error handling
tracing                # Logging
dirs                   # System paths
sha2, hex              # Checksums
tempfile, fs2          # Atomic file operations
mockall                # Testing mocks
```

### tauri/Cargo.toml
```
tauri v2 (tray-icon)         # GUI framework
tauri-plugin-shell           # Shell operations
tauri-plugin-notification    # System notifications
tauri-plugin-autostart       # OS-level autostart
clawenv-core                 # Shared core logic
reqwest                      # Bridge API calls
```

### cli/Cargo.toml
```
clawenv-core    # Shared core logic
clap v4         # CLI argument parsing
indicatif       # Progress bars
console         # Terminal colors
dialoguer       # Interactive prompts
```

## 3.3 Frontend Dependencies

```json
{
  "@tauri-apps/api": "^2",
  "@tauri-apps/plugin-notification": "^2",
  "@tauri-apps/plugin-shell": "^2",
  "@xterm/addon-fit": "^0.11",
  "solid-js": "^1.9",
  "xterm": "^5.3"
}
```

Dev: `vite`, `vite-plugin-solid`, `@tailwindcss/vite`, `typescript`

## 3.4 Rust Versions

- **core + cli**: rustc 1.87+ (Homebrew)
- **tauri**: rustc 1.88+ (`~/.cargo/bin/rustc`, Tauri deps require newer darling/time)

## 3.5 Build Commands

```bash
cargo tauri dev          # Dev mode (hot reload)
cargo tauri build        # Production build
cargo test --workspace   # All tests (83 tests)
cargo clippy --workspace # Lint
npm install              # Frontend deps
```
