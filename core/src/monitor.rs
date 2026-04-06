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

    /// Check the health of a single instance
    /// Uses HTTP probe on the gateway port — most reliable method.
    pub async fn check_health_with_port(backend: &dyn SandboxBackend, port: u16) -> InstanceHealth {
        // Primary: HTTP probe the gateway inside the VM
        match backend.exec(&format!(
            "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
        )).await {
            Ok(out) => {
                let code = out.trim().trim_matches('\'');
                if code.starts_with('2') || code.starts_with('3') || code == "401" || code == "403" {
                    return InstanceHealth::Running; // HTTP response = gateway alive
                }
            }
            Err(_) => return InstanceHealth::Unreachable,
        }
        // Fallback: check if process exists via pidof (no self-match issue)
        match backend.exec("pidof node 2>/dev/null || echo ''").await {
            Ok(out) if !out.trim().is_empty() => InstanceHealth::Stopped, // process alive but not responding on port
            _ => InstanceHealth::Stopped,
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
