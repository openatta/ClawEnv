//! Runtime lookup of available Claw CLIs.

use super::{ClawCli, HermesCli, OpenClawCli};

pub struct ClawRegistry;

impl ClawRegistry {
    /// Return a boxed `ClawCli` for the given id, or `None` if unknown.
    pub fn cli_for(id: &str) -> Option<Box<dyn ClawCli>> {
        match id {
            "hermes" => Some(Box::new(HermesCli::new())),
            "openclaw" => Some(Box::new(OpenClawCli::new())),
            _ => None,
        }
    }

    /// All known Claw CLIs.
    pub fn all() -> Vec<Box<dyn ClawCli>> {
        vec![
            Box::new(HermesCli::new()),
            Box::new(OpenClawCli::new()),
        ]
    }

    pub fn ids() -> Vec<&'static str> {
        vec!["hermes", "openclaw"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_returns_both() {
        let all = ClawRegistry::all();
        let ids: Vec<&str> = all.iter().map(|c| c.id()).collect();
        assert!(ids.contains(&"hermes"));
        assert!(ids.contains(&"openclaw"));
    }

    #[test]
    fn cli_for_known_returns_some() {
        assert!(ClawRegistry::cli_for("hermes").is_some());
        assert!(ClawRegistry::cli_for("openclaw").is_some());
    }

    #[test]
    fn cli_for_unknown_returns_none() {
        assert!(ClawRegistry::cli_for("nope").is_none());
    }
}
