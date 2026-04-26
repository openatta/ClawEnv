//! MCP plugin deployment + registration (P1-g).
//!
//! Lifted from v1 `core/src/manager/install.rs:649-734`. Three plugins
//! (mcp-bridge, hil-skill, hw-notify) get embedded into the binary
//! via `include_str!` and deployed into `/workspace/<plugin>/` inside
//! the VM at install time. Then registered with the claw via
//! `<bin> mcp set <name> <json>`.
//!
//! Runtime selection: `provisioning.mcp_runtime()` returns "node" or
//! "python", and we deploy the matching `.mjs` or `.py` files.

use std::sync::Arc;

use crate::claw_ops::ClawProvisioning;
use crate::common::{OpsError, ProgressSink};
use crate::sandbox_backend::SandboxBackend;

// Plugin assets — embedded at compile time. v2/assets/mcp/*.{mjs,py}
// were copied verbatim from v1 assets/mcp/.
const BRIDGE_MJS: &str = include_str!("../../../assets/v2/mcp/mcp-bridge.mjs");
const HIL_MJS: &str = include_str!("../../../assets/v2/mcp/hil-skill.mjs");
const HW_MJS: &str = include_str!("../../../assets/v2/mcp/hw-notify.mjs");
const BRIDGE_PY: &str = include_str!("../../../assets/v2/mcp/mcp-bridge.py");
const HIL_PY: &str = include_str!("../../../assets/v2/mcp/hil-skill.py");
const HW_PY: &str = include_str!("../../../assets/v2/mcp/hw-notify.py");

/// One plugin entry. `dir_name` is the directory under /workspace,
/// `reg_name` is what we pass to `mcp set <reg_name> <json>`,
/// `file_name` lands inside that dir, `content` is the embedded body.
struct PluginSpec {
    dir_name: &'static str,
    reg_name: &'static str,
    file_name: &'static str,
    content: &'static str,
}

fn plugins_for_runtime(runtime: &str) -> Vec<PluginSpec> {
    if runtime == "python" {
        vec![
            PluginSpec { dir_name: "mcp-bridge", reg_name: "clawenv",     file_name: "bridge.py", content: BRIDGE_PY },
            PluginSpec { dir_name: "hil-skill",  reg_name: "clawenv-hil", file_name: "skill.py",  content: HIL_PY },
            PluginSpec { dir_name: "hw-notify",  reg_name: "hw-notify",   file_name: "notify.py", content: HW_PY },
        ]
    } else {
        vec![
            PluginSpec { dir_name: "mcp-bridge", reg_name: "clawenv",     file_name: "index.mjs", content: BRIDGE_MJS },
            PluginSpec { dir_name: "hil-skill",  reg_name: "clawenv-hil", file_name: "index.mjs", content: HIL_MJS },
            PluginSpec { dir_name: "hw-notify",  reg_name: "hw-notify",   file_name: "notify.mjs", content: HW_MJS },
        ]
    }
}

/// Deploy plugin source files into /workspace/<dir>/<file>.
/// No-op for claws that don't support MCP.
pub async fn deploy_plugins(
    backend: &Arc<dyn SandboxBackend>,
    provisioning: &dyn ClawProvisioning,
    progress: &ProgressSink,
) -> Result<(), OpsError> {
    if !provisioning.supports_mcp() {
        return Ok(());
    }
    let plugins = plugins_for_runtime(provisioning.mcp_runtime());
    progress.info("mcp", format!("Deploying {} plugin(s)", plugins.len())).await;

    for p in &plugins {
        let dir = format!("/workspace/{}", p.dir_name);
        backend.exec_argv(&["sh", "-c", &format!("mkdir -p {dir}")])
            .await
            .map_err(OpsError::Other)?;

        // Heredoc with a stable per-plugin marker; plugin source can't
        // contain that marker without breaking the heredoc, but ours
        // are static and audit-able.
        let eof = format!("CLAWOPS_MCP_EOF_{}", p.dir_name.to_uppercase().replace('-', "_"));
        let script = format!(
            "cat > {dir}/{file} << '{eof}'\n{content}\n{eof}\n",
            file = p.file_name,
            content = p.content,
        );
        backend.exec_argv(&["sh", "-c", &script])
            .await
            .map_err(OpsError::Other)?;
    }

    // Python runtime: install MCP SDK on top of the base apk packages.
    if provisioning.mcp_runtime() == "python" {
        let _ = backend.exec_argv(&[
            "sh", "-c",
            "pip install --break-system-packages mcp httpx 2>&1 || true"
        ]).await;
    }

    Ok(())
}

/// Register the deployed plugins with the claw via `<bin> mcp set`.
/// Pre-condition: `deploy_plugins` already ran and the gateway is up
/// (or at least `<bin> mcp set` is callable — for some claws this
/// works via config edit without the gateway running).
pub async fn register_plugins(
    backend: &Arc<dyn SandboxBackend>,
    provisioning: &dyn ClawProvisioning,
    bridge_url: &str,
    gateway_token: &str,
    progress: &ProgressSink,
) -> Result<(), OpsError> {
    if !provisioning.supports_mcp() {
        return Ok(());
    }
    let mcp_set = match provisioning.mcp_set_cmd_template() {
        Some(t) => t,
        None => return Ok(()),
    };
    let runner = if provisioning.mcp_runtime() == "python" { "python3" } else { "node" };
    let plugins = plugins_for_runtime(provisioning.mcp_runtime());

    for p in &plugins {
        let json = serde_json::json!({
            "command": runner,
            "args": [format!("/workspace/{}/{}", p.dir_name, p.file_name)],
            "env": {
                "CLAWENV_BRIDGE_URL": bridge_url,
                "CLAWENV_GATEWAY_TOKEN": gateway_token,
            }
        }).to_string();

        // Substitute placeholders in the template; embed via shell quoting.
        let cmd = format!(
            "{} {}",
            provisioning.cli_binary(),
            mcp_set.replace("{name}", p.reg_name).replace("{json}", &json),
        );
        progress.info("mcp", format!("Registering `{}`", p.reg_name)).await;
        // Best-effort: a failure to register one plugin shouldn't kill install.
        if let Err(e) = backend.exec_argv(&["sh", "-c", &cmd]).await {
            progress.info("mcp",
                format!("`{}` register failed (may already exist): {e}", p.reg_name)).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claw_ops::{HermesProvisioning, OpenClawProvisioning};
    use crate::sandbox_ops::testing::MockBackend;

    #[tokio::test]
    async fn deploy_writes_three_node_plugins_for_openclaw() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let p = OpenClawProvisioning;
        deploy_plugins(&backend, &p, &ProgressSink::noop()).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        // 3 mkdir + 3 cat heredoc = 6 calls.
        assert_eq!(log.len(), 6, "expected 6 exec (mkdir+heredoc x3): {log:?}");
        // Content writes have node .mjs filenames.
        let contents: String = log.join("\n");
        assert!(contents.contains("index.mjs"), "no node files: {contents}");
        assert!(!contents.contains("bridge.py"), "wrong runtime: {contents}");
    }

    #[tokio::test]
    async fn deploy_writes_python_plugins_for_hermes_and_pip_installs_sdk() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let p = HermesProvisioning;
        deploy_plugins(&backend, &p, &ProgressSink::noop()).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        // 3 mkdir + 3 heredoc + 1 pip = 7.
        assert_eq!(log.len(), 7, "expected 7 calls for python runtime: {log:?}");
        let contents: String = log.join("\n");
        assert!(contents.contains("bridge.py"));
        assert!(contents.contains("skill.py"));
        assert!(contents.contains("notify.py"));
        assert!(contents.contains("pip install"));
        assert!(contents.contains("mcp httpx"));
    }

    #[tokio::test]
    async fn register_calls_mcp_set_for_each_plugin() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let p = OpenClawProvisioning;
        register_plugins(&backend, &p, "http://host:3002", "tok123", &ProgressSink::noop())
            .await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 3, "expected 3 mcp set calls: {log:?}");
        for line in log.iter() {
            assert!(line.contains("openclaw mcp set"), "bad cmd: {line}");
            assert!(line.contains("http://host:3002"), "bridge_url not embedded: {line}");
            assert!(line.contains("tok123"), "token not embedded: {line}");
        }
    }
}
