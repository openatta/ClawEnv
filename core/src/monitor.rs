use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::InstanceConfig;
use crate::manager::instance::backend_for_instance;
use crate::sandbox::SandboxBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceHealth {
    Running,
    Stopped,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvent {
    pub instance_name: String,
    pub health: InstanceHealth,
}

pub struct InstanceMonitor {
    pub interval: Duration,
}

impl Default for InstanceMonitor {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
        }
    }
}

impl InstanceMonitor {
    pub fn with_interval(secs: u32) -> Self {
        Self {
            interval: Duration::from_secs(secs as u64),
        }
    }

    /// Check the health of a single instance.
    /// For native mode: pure Rust HTTP request (no shell, works on all platforms).
    /// For sandbox mode: curl inside the VM via backend.exec().
    pub async fn check_health_with_port(backend: &dyn SandboxBackend, port: u16) -> InstanceHealth {
        Self::check_health_with_port_platform(backend, port, false).await
    }

    pub async fn check_health_with_port_platform(_backend: &dyn SandboxBackend, port: u16, is_native: bool) -> InstanceHealth {
        if is_native {
            // Native: direct Rust HTTP — no shell, no PowerShell, no quoting issues
            Self::check_health_native(port).await
        } else {
            // Sandbox: curl inside VM
            let probe_cmd = format!(
                "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
            );
            let result = _backend.exec(&probe_cmd).await;
            match &result {
                Ok(out) => {
                    let code = out.trim().trim_matches('\'').trim();
                    if code != "000" && !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) {
                        return InstanceHealth::Running;
                    }
                }
                Err(_) => return InstanceHealth::Unreachable,
            }
            // Fallback: pgrep inside VM
            match _backend.exec("pgrep -f 'gateway' 2>/dev/null || echo ''").await {
                Ok(out) if !out.trim().is_empty() => InstanceHealth::Running,
                _ => InstanceHealth::Stopped,
            }
        }
    }

    /// Pure Rust HTTP health check for native instances — no shell dependency.
    pub async fn check_health_native(port: u16) -> InstanceHealth {
        let url = format!("http://127.0.0.1:{port}/");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build();
        match client {
            Ok(c) => match c.get(&url).send().await {
                Ok(resp) if resp.status().is_success() || resp.status().is_redirection() || resp.status().as_u16() == 401 => {
                    InstanceHealth::Running
                }
                Ok(_) => InstanceHealth::Running, // Any HTTP response = process alive
                Err(_) => InstanceHealth::Stopped,
            },
            Err(_) => InstanceHealth::Unreachable,
        }
    }

    /// Legacy check without port (uses default 3000)
    pub async fn check_health(backend: &dyn SandboxBackend) -> InstanceHealth {
        Self::check_health_with_port(backend, 3000).await
    }

    /// Run the monitoring loop, sending health events for all instances
    pub async fn run(
        &self,
        instances: Vec<InstanceConfig>,
        tx: mpsc::Sender<HealthEvent>,
    ) {
        loop {
            for inst in &instances {
                let health = match backend_for_instance(inst) {
                    Ok(backend) => Self::check_health(backend.as_ref()).await,
                    Err(_) => InstanceHealth::Unreachable,
                };

                let event = HealthEvent {
                    instance_name: inst.name.clone(),
                    health,
                };

                if tx.send(event).await.is_err() {
                    // Receiver dropped, stop monitoring
                    tracing::info!("Monitor channel closed, stopping");
                    return;
                }
            }
            tokio::time::sleep(self.interval).await;
        }
    }
}
