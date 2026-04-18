//! Unified "instance state changed" event.
//!
//! Every IPC that mutates an instance's runtime or config state emits
//! `instance-changed` at the end of its side-effect window. The frontend
//! subscribes once (MainLayout) and drives all refreshes from there —
//! list reload, health refetch, activeTab fixup, gateway-token invalidation,
//! needs-restart toast, etc.
//!
//! Keep the payload small and additive: new fields must be optional so older
//! frontend builds keep working.

use serde::Serialize;
use tauri::Emitter;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum InstanceAction {
    /// A brand-new instance was created (online install or bundle import).
    /// Triggers a list refresh in MainLayout so the new entry appears in
    /// Home / ClawPage tabs. The install runs in a separate WebviewWindow,
    /// so this event is how the main window learns about the addition.
    Install,
    Start,
    Stop,
    Delete,
    Rename,
    EditPorts,
    EditResources,
    InstallChromium,
    Upgrade,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct InstanceChanged {
    pub action: String,              // snake_case action name
    pub instance: Option<String>,    // primary instance (old name for rename, victim for delete)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_restart: Option<bool>,
}

impl InstanceChanged {
    pub fn simple(action: InstanceAction, instance: impl Into<String>) -> Self {
        Self {
            action: action_name(action),
            instance: Some(instance.into()),
            ..Default::default()
        }
    }

    pub fn with_needs_restart(mut self, v: bool) -> Self {
        self.needs_restart = Some(v);
        self
    }

    pub fn deleted(instance: impl Into<String>) -> Self {
        Self {
            action: action_name(InstanceAction::Delete),
            instance: Some(instance.into()),
            removed: Some(true),
            ..Default::default()
        }
    }

    pub fn renamed(old: impl Into<String>, new: impl Into<String>) -> Self {
        let old = old.into();
        let new = new.into();
        Self {
            action: action_name(InstanceAction::Rename),
            instance: Some(old.clone()),
            old_name: Some(old),
            new_name: Some(new),
            ..Default::default()
        }
    }
}

fn action_name(a: InstanceAction) -> String {
    match a {
        InstanceAction::Install => "install",
        InstanceAction::Start => "start",
        InstanceAction::Stop => "stop",
        InstanceAction::Delete => "delete",
        InstanceAction::Rename => "rename",
        InstanceAction::EditPorts => "edit_ports",
        InstanceAction::EditResources => "edit_resources",
        InstanceAction::InstallChromium => "install_chromium",
        InstanceAction::Upgrade => "upgrade",
    }.to_string()
}

/// Emit the canonical `instance-changed` event. Swallow errors — the event is
/// best-effort; a missing listener is never a reason to fail the action itself.
pub fn emit_instance_changed(app: &tauri::AppHandle, payload: InstanceChanged) {
    let _ = app.emit("instance-changed", payload);
}
