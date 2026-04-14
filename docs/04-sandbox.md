# 4. Sandbox Implementation

## 4.1 Common Interface

All backends implement the `SandboxBackend` trait:

```rust
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn is_available(&self) -> Result<bool>;
    async fn ensure_prerequisites(&self) -> Result<()>;
    async fn create(&self, opts: &SandboxOpts) -> Result<()>;
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn destroy(&self) -> Result<()>;
    async fn exec(&self, cmd: &str) -> Result<String>;
    async fn exec_with_progress(&self, cmd: &str, tx: &Sender<String>) -> Result<String>;
    async fn edit_resources(&self, cpus: Option<u32>, memory_mb: Option<u32>, disk_gb: Option<u32>) -> Result<()>;
    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> Result<()>;
    fn supports_rename(&self) -> bool;
    fn supports_resource_edit(&self) -> bool;
    fn supports_port_edit(&self) -> bool;
}
```

Factory: `detect_backend_for(instance_name)` returns the platform-appropriate backend.

## 4.2 macOS: Lima VZ

- **VM engine**: Apple Virtualization.framework (VZ mode, not QEMU)
- **Template**: `assets/lima/clawenv-alpine.yaml`
- **Port forwarding**: Lima guestagent auto-detects ports bound to 0.0.0.0
  - Gateway uses `--bind lan` to bind 0.0.0.0
  - ttyd binds 0.0.0.0 by default
- **VM naming**: `clawenv-{instance_name}`
- **Provision**: Alpine 3.23 + git, curl, bash, nodejs, npm, ttyd, openssh, procps

Key: Lima VZ guestagent only forwards ports listening on `0.0.0.0`, not `127.0.0.1`.
This is why gateway config is set to `gateway.bind=lan` before start.

## 4.3 Windows: WSL2

- **VM engine**: Hyper-V lightweight VM
- **Port forwarding**: `netsh interface portproxy` (requires Administrator)
- **Distro naming**: `clawenv-{instance_name}`
- **Provision**: Same Alpine packages as Lima

WSL2 port forwarding via netsh binds explicitly to `127.0.0.1` on the host.
No `gateway.bind=lan` needed (netsh handles the translation).

## 4.4 Linux: Podman

- **Engine**: Rootless Podman containers
- **Flags**: `--userns=keep-id`, volumes with `:Z` (SELinux)
- **Port forwarding**: Direct `-p host:guest` mapping
- **Container naming**: `clawenv-{instance_name}`

Podman uses `host.containers.internal` for host access from inside containers.

## 4.5 Native Mode

No sandbox. OpenClaw installed directly on host via npm:
- Node.js managed by ClawEnv (`~/.clawenv/node/`)
- Gateway runs as detached host process
- No ttyd (host already has terminal)
- Only one native instance allowed per machine

## 4.6 Browser Integration (Chromium in Sandbox)

Chromium + noVNC chain for browser automation:

```
Headless mode:  chromium --headless --remote-debugging-port={cdp_port}
Interactive:    Xvfb :99 → x11vnc → websockify:{vnc_ws_port} → noVNC iframe

Installed via:  sudo apk add chromium xvfb-run x11vnc novnc websockify ttf-freefont
```

HIL (Human-in-the-Loop): agent calls `hil_request` MCP tool → ClawEnv switches
from headless to interactive → user operates browser via noVNC → clicks
"Continue Auto" → switches back to headless.

## 4.7 Per-Instance Services

Each sandbox instance runs:

| Service | Port | Binding | Started by |
|---------|------|---------|-----------|
| OpenClaw gateway | base+0 | 0.0.0.0 (sandbox) / 127.0.0.1 (native) | start_instance() |
| ttyd terminal | base+1 | 0.0.0.0 | start_instance() (sandbox only) |
| Chromium CDP | base+3 | 0.0.0.0 | browser_start_headless() |
| noVNC websockify | base+4 | 0.0.0.0 | browser_start_interactive() |

Bridge server runs on the **host** (not in sandbox), port base+2.
