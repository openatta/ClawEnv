//! Instance layer — v2's own lightweight instance registry.
//!
//! Tracks which (claw × backend × instance-name × ports) records v2 has
//! created. Persisted at `<clawenv_root>/v2/instances.toml`.
//!
//! This is deliberately separate from v1's `config.toml` — v2 manages its
//! own instances independently. Future stages can add a migration step
//! to bridge the two.

pub mod config;
pub mod orchestrator;

pub use config::{InstanceConfig, InstanceRegistry, PortBinding, SandboxKind};
pub use orchestrator::{InstanceOrchestrator, CreateOpts, CreateReport, DestroyReport};
