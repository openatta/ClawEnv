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
    pub async fn check_health(backend: &dyn SandboxBackend) -> InstanceHealth {
        match backend.exec("pgrep -f openclaw").await {
            Ok(out) if !out.trim().is_empty() => InstanceHealth::Running,
            Ok(_) => InstanceHealth::Stopped,
            Err(_) => InstanceHealth::Unreachable,
        }
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
