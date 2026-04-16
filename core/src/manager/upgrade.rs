use anyhow::Result;
use tokio::sync::mpsc;

use crate::claw::ClawRegistry;
use crate::config::{ConfigManager, InstanceConfig};
use crate::manager::instance::backend_for_instance;
use crate::sandbox::SandboxType;
use crate::update::checker::{self, VersionInfo};

/// Check if an upgrade is available for an instance.
/// Dispatches to npm registry or PyPI based on the claw's package manager.
pub async fn check_upgrade(instance: &InstanceConfig, registry_url: &str) -> Result<VersionInfo> {
    let claw_registry = ClawRegistry::load();
    let desc = claw_registry.get(&instance.claw_type);
    match desc.package_manager {
        crate::claw::descriptor::PackageManager::Npm => {
            checker::check_latest_npm(&instance.version, registry_url, &desc.npm_package).await
        }
        crate::claw::descriptor::PackageManager::Pip => {
            checker::check_latest_pypi(&instance.version, &desc.pip_package).await
        }
        crate::claw::descriptor::PackageManager::GitPip => {
            // Git-based packages: check GitHub releases for latest version
            if !desc.git_repo.is_empty() {
                checker::check_latest_github(&instance.version, &desc.git_repo).await
            } else if !desc.pip_package.is_empty() {
                checker::check_latest_pypi(&instance.version, &desc.pip_package).await
            } else {
                Ok(checker::VersionInfo {
                    current: instance.version.clone(),
                    latest: instance.version.clone(),
                    has_upgrade: false,
                    is_security_release: false,
                    changelog: String::new(),
                })
            }
        }
    }
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

    // 2. Run package upgrade (npm or pip)
    let install_cmd = desc.sandbox_install_cmd(version);
    let pm_label = match desc.package_manager {
        crate::claw::descriptor::PackageManager::Npm => "npm",
        crate::claw::descriptor::PackageManager::Pip | crate::claw::descriptor::PackageManager::GitPip => "pip",
    };
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
        // Sandbox: background script + polling (shared implementation)
        use super::background::{run_background_script, BackgroundScriptOpts};
        let tx_bg = tx.clone();
        run_background_script(backend.as_ref(), &BackgroundScriptOpts {
            cmd: &install_cmd,
            label: &format!("{pm_label} upgrade"),
            sudo: true,
            log_file: "/tmp/clawenv-upgrade.log",
            done_file: "/tmp/clawenv-upgrade.done",
            script_file: "/tmp/clawenv-upgrade.sh",
            pct_range: (25, 80),
            ..Default::default()
        }, move |msg, pct| {
            let tx_bg = tx_bg.clone();
            tokio::spawn(async move {
                send(&tx_bg, &msg, pct, "install").await;
            });
        }).await?;
    }

    // 3. Verify new version
    send(tx, "Verifying upgrade...", 85, "verify").await;
    let new_ver = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await?;
    let new_ver = new_ver.trim().to_string();

    // 4. Restart gateway
    send(tx, "Restarting gateway...", 90, "restart").await;
    let port = instance.gateway.gateway_port;
    if let Some(gateway_cmd) = desc.gateway_start_cmd(port) {
        backend.exec(&format!(
            "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
        )).await?;
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 5. Update config
    send(tx, "Saving configuration...", 95, "config").await;
    let ver = new_ver.clone();
    config.update_instance(instance_name, move |inst| {
        inst.version = ver;
        inst.last_upgraded_at = chrono::Utc::now().to_rfc3339();
    })?;

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
