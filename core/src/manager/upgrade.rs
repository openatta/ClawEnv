use anyhow::Result;
use tokio::sync::mpsc;

use crate::claw::ClawRegistry;
use crate::config::{ConfigManager, InstanceConfig};
use crate::manager::instance::backend_for_instance;
use crate::sandbox::SandboxType;
use crate::update::checker::{self, VersionInfo};

/// Check if an upgrade is available for an instance.
/// `npm_registry` can be empty to use the default registry.
pub async fn check_upgrade(instance: &InstanceConfig, npm_registry: &str, npm_package: &str) -> Result<VersionInfo> {
    checker::check_latest_version(&instance.version, npm_registry, npm_package).await
}

/// Progress event for upgrade UI
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpgradeProgress {
    pub message: String,
    pub percent: u8,
    pub stage: String,
}

/// Upgrade an instance to target version (or latest).
/// Uses background script + polling for sandbox, direct exec for native.
pub async fn upgrade_instance(
    config: &mut ConfigManager,
    instance_name: &str,
    target_version: Option<&str>,
    tx: &mpsc::Sender<UpgradeProgress>,
) -> Result<String> {
    let instance = config
        .instances()
        .iter()
        .find(|i| i.name == instance_name)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance_name))?
        .clone();

    let registry = ClawRegistry::load();
    let desc = registry.get(&instance.claw_type);
    let backend = backend_for_instance(&instance)?;
    let version = target_version.unwrap_or("latest");

    // 1. Stop gateway before upgrade
    send(tx, "Stopping gateway...", 20, "prepare").await;
    for pn in &desc.process_names() {
        backend.exec(&crate::platform::process::kill_by_name_cmd(pn)).await.ok();
    }

    // 2. Run npm upgrade
    let install_cmd = desc.npm_install_verbose_cmd(version);
    send(tx, &format!("Upgrading {} to {version}...", desc.display_name), 25, "install").await;

    if instance.sandbox_type == SandboxType::Native {
        // Native: direct exec
        let (progress_tx, mut progress_rx) = mpsc::channel::<String>(64);
        let tx_ui = tx.clone();
        let ui_task = tokio::spawn(async move {
            let start = std::time::Instant::now();
            while let Some(line) = progress_rx.recv().await {
                let t = line.trim();
                if !t.is_empty() {
                    let elapsed = start.elapsed().as_secs();
                    let short = if t.len() > 80 { &t[..80] } else { t };
                    let pct = std::cmp::min(25 + (elapsed / 8) as u8, 80);
                    send(&tx_ui, &format!("[{elapsed}s] {short}"), pct, "install").await;
                }
            }
        });
        backend.exec_with_progress(&install_cmd, &progress_tx).await?;
        drop(progress_tx);
        ui_task.await.ok();
    } else {
        // Sandbox: background script + polling
        let log = "/tmp/clawenv-upgrade.log";
        let done = "/tmp/clawenv-upgrade.done";
        backend.exec(&format!("rm -f {log} {done}")).await?;
        backend.exec(&format!(
            r#"cat > /tmp/clawenv-upgrade.sh << 'UPGEOF'
#!/bin/sh
sudo {install_cmd} > {log} 2>&1
echo $? > {done}
UPGEOF
chmod +x /tmp/clawenv-upgrade.sh"#
        )).await?;
        backend.exec("nohup sh /tmp/clawenv-upgrade.sh > /dev/null 2>&1 &").await?;

        let mut elapsed = 0u64;
        let mut last_lines = 0usize;
        let mut idle = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            elapsed += 5;

            let done_val = backend.exec(&format!("cat {done} 2>/dev/null || echo ''")).await.unwrap_or_default();
            let new_output = backend.exec(&format!(
                "tail -n +{} {log} 2>/dev/null | head -30 || echo ''", last_lines + 1
            )).await.unwrap_or_default();

            let new_lines: Vec<&str> = new_output.lines().filter(|l| !l.trim().is_empty()).collect();
            if !new_lines.is_empty() {
                idle = 0;
                last_lines += new_lines.len();
                let last = new_lines.last().unwrap_or(&"");
                let short = if last.len() > 80 { &last[..80] } else { last };
                let pct = std::cmp::min(25 + (elapsed / 8) as u8, 80);
                send(tx, &format!("[{elapsed}s] {short}"), pct, "install").await;
            } else {
                idle += 5;
                let pct = std::cmp::min(25 + (elapsed / 8) as u8, 80);
                send(tx, &format!("Upgrading... ({elapsed}s)"), pct, "install").await;
            }

            if !done_val.trim().is_empty() {
                let rc: i32 = done_val.trim().parse().unwrap_or(-1);
                backend.exec("rm -f /tmp/clawenv-upgrade.sh /tmp/clawenv-upgrade.log /tmp/clawenv-upgrade.done").await.ok();
                if rc != 0 {
                    let tail = backend.exec(&format!("tail -5 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
                    anyhow::bail!("npm upgrade failed (exit {rc}):\n{tail}");
                }
                break;
            }

            if idle >= 600 {
                anyhow::bail!("Upgrade stalled — no output for 10 min");
            }
        }
    }

    // 3. Verify new version
    send(tx, "Verifying upgrade...", 85, "verify").await;
    let new_ver = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await?;
    let new_ver = new_ver.trim().to_string();

    // 4. Restart gateway
    send(tx, "Restarting gateway...", 90, "restart").await;
    let port = instance.gateway.gateway_port;
    let gateway_cmd = desc.gateway_start_cmd(port);
    backend.exec(&format!(
        "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
    )).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 5. Update config
    send(tx, "Saving configuration...", 95, "config").await;
    for inst in config.config_mut().instances.iter_mut() {
        if inst.name == instance_name {
            inst.version = new_ver.clone();
            inst.last_upgraded_at = chrono::Utc::now().to_rfc3339();
        }
    }
    config.save()?;

    send(tx, &format!("Upgraded to {new_ver}"), 100, "done").await;
    Ok(new_ver)
}

async fn send(tx: &mpsc::Sender<UpgradeProgress>, message: &str, percent: u8, stage: &str) {
    let _ = tx.send(UpgradeProgress {
        message: message.to_string(),
        percent,
        stage: stage.to_string(),
    }).await;
}
