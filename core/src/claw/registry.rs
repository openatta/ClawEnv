//! Claw product registry — lookup descriptors by claw_type ID.
//!
//! Sources (in priority order):
//!   1. User-defined descriptors in config.toml `[[custom_claws]]`
//!   2. Built-in registry from assets/claw-registry.toml

use anyhow::Result;
use std::collections::HashMap;

use super::descriptor::{ClawDescriptor, openclaw_descriptor};

/// In-memory registry of known claw descriptors.
pub struct ClawRegistry {
    descriptors: HashMap<String, ClawDescriptor>,
}

impl ClawRegistry {
    /// Load registry: built-in defaults + optional TOML overrides.
    pub fn load() -> Self {
        let mut descriptors = HashMap::new();

        // 1. Load built-in registry from embedded TOML
        let builtin_toml = include_str!("../../../assets/claw-registry.toml");
        if let Ok(table) = builtin_toml.parse::<toml::Table>() {
            if let Some(claws) = table.get("claw").and_then(|v| v.as_array()) {
                for entry in claws {
                    if let Ok(desc) = entry.clone().try_into::<ClawDescriptor>() {
                        descriptors.insert(desc.id.clone(), desc);
                    }
                }
            }
        }

        // 2. Ensure OpenClaw is always present (even if TOML is broken)
        descriptors
            .entry("openclaw".into())
            .or_insert_with(openclaw_descriptor);

        Self { descriptors }
    }

    /// Look up a descriptor by claw_type ID.
    /// Falls back to OpenClaw if the ID is not found, or panics if registry is empty.
    pub fn get(&self, claw_type: &str) -> &ClawDescriptor {
        self.descriptors
            .get(claw_type)
            .or_else(|| self.descriptors.get("openclaw"))
            .expect("ClawRegistry is empty — claw-registry.toml is missing or invalid")
    }

    /// Look up a descriptor, returning an error if not found.
    pub fn get_strict(&self, claw_type: &str) -> Result<&ClawDescriptor> {
        self.descriptors
            .get(claw_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown claw type: '{}'. Use 'clawenv list-claws' to see available types.", claw_type))
    }

    /// List all registered claw type IDs.
    pub fn list_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.descriptors.keys().map(|s| s.as_str()).collect();
        ids.sort();
        ids
    }

    /// List all descriptors. OpenClaw is pinned to the front as the reference
    /// implementation; all others follow in alphabetical order.
    pub fn list_all(&self) -> Vec<&ClawDescriptor> {
        let mut descs: Vec<&ClawDescriptor> = self.descriptors.values().collect();
        descs.sort_by(|a, b| match (a.id.as_str(), b.id.as_str()) {
            ("openclaw", "openclaw") => std::cmp::Ordering::Equal,
            ("openclaw", _) => std::cmp::Ordering::Less,
            (_, "openclaw") => std::cmp::Ordering::Greater,
            _ => a.id.cmp(&b.id),
        });
        descs
    }

    /// Register a custom descriptor (e.g., from user config).
    pub fn register(&mut self, desc: ClawDescriptor) {
        self.descriptors.insert(desc.id.clone(), desc);
    }
}
