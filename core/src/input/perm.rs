//! Platform permission probing + first-run guidance.
//!
//! macOS:
//!  - Accessibility is required by `enigo` for synthetic keyboard/mouse
//!    events. There is no side-effect-free probe short of
//!    `AXIsProcessTrusted*`; rdev also trips AX when it spins up. We use
//!    a fast "try to create an Enigo and immediately drop it" probe.
//!  - Screen Recording is required by `xcap::Monitor::capture_image`.
//!    Listing monitors works without screen-recording, but capturing
//!    does not. `capture_image` is the only reliable probe.
//!
//! Windows:
//!  - SendInput / GDI capture work for normal user processes; return
//!    `Granted` unconditionally.
//!
//! Linux:
//!  - Out of scope per CLAUDE.md; return `Unsupported` so the caller
//!    can short-circuit with a clean message.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermState {
    Granted,
    Denied,
    NotDetermined,
    Unsupported,
}

impl PermState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::NotDetermined => "not_determined",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PermReport {
    pub accessibility: PermState,
    pub screen_capture: PermState,
}

impl PermReport {
    pub fn all_granted(&self) -> bool {
        self.accessibility == PermState::Granted
            && self.screen_capture == PermState::Granted
    }
}

// ---------- Probe ----------

#[cfg(target_os = "macos")]
pub fn probe() -> PermReport {
    let accessibility = probe_macos_accessibility();
    let screen_capture = probe_macos_screen_capture();
    PermReport { accessibility, screen_capture }
}

#[cfg(target_os = "macos")]
fn probe_macos_accessibility() -> PermState {
    // enigo::Enigo::new() fails with NoPermission when AX is denied.
    // Construction is cheap; it doesn't post any events.
    match enigo::Enigo::new(&enigo::Settings::default()) {
        Ok(_) => PermState::Granted,
        Err(_) => PermState::Denied,
    }
}

#[cfg(target_os = "macos")]
fn probe_macos_screen_capture() -> PermState {
    // Listing monitors is NOT a reliable probe on macOS — it returns
    // entries even without screen-recording permission, just with
    // blanked-out fields. A 1×1 capture_image is the shortest path to
    // truth. We capture a tiny region of the primary monitor.
    match xcap::Monitor::all() {
        Ok(monitors) => {
            let Some(primary) = monitors.into_iter().next() else {
                return PermState::NotDetermined;
            };
            match primary.capture_image() {
                Ok(_) => PermState::Granted,
                Err(_) => PermState::Denied,
            }
        }
        Err(_) => PermState::Denied,
    }
}

#[cfg(target_os = "windows")]
pub fn probe() -> PermReport {
    PermReport {
        accessibility: PermState::Granted,
        screen_capture: PermState::Granted,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn probe() -> PermReport {
    PermReport {
        accessibility: PermState::Unsupported,
        screen_capture: PermState::Unsupported,
    }
}

// ---------- First-run guidance ----------

pub struct GuidanceMessage {
    /// Human-readable one-line summary.
    pub headline: String,
    /// Step-by-step instructions printed to the user.
    pub steps: Vec<String>,
    /// macOS-only: `x-apple.systempreferences:...` URLs that open the
    /// right privacy pane. Empty on other platforms.
    pub open_urls: Vec<String>,
}

pub fn guidance_for(report: &PermReport) -> Option<GuidanceMessage> {
    if report.all_granted() {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let mut steps: Vec<String> = Vec::new();
        let mut urls: Vec<String> = Vec::new();
        if report.accessibility != PermState::Granted {
            steps.push("Open System Settings → Privacy & Security → Accessibility, then add and enable this binary.".into());
            urls.push("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility".into());
        }
        if report.screen_capture != PermState::Granted {
            steps.push("Open System Settings → Privacy & Security → Screen & System Audio Recording, then add and enable this binary.".into());
            urls.push("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture".into());
        }
        steps.push("After granting, fully quit and relaunch the bridge (macOS caches the decision per launch).".into());
        return Some(GuidanceMessage {
            headline: "ClawEnv remote-control needs macOS privacy permissions.".into(),
            steps,
            open_urls: urls,
        });
    }
    #[cfg(target_os = "windows")]
    {
        let _ = report;
        return None;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = report;
        Some(GuidanceMessage {
            headline: "Remote input/screen tools are not supported on this OS.".into(),
            steps: vec!["This platform has no input/screen MCP tools; the remote channel still works but input_* calls will fail.".into()],
            open_urls: vec![],
        })
    }
}

/// Best-effort: open the system-settings panes the user needs, on macOS.
/// No-op elsewhere. Does NOT block; returns once the `open` calls are
/// spawned. Errors are logged but not returned — a failed `open` should
/// not stop the daemon from starting.
pub fn open_guidance(msg: &GuidanceMessage) {
    #[cfg(target_os = "macos")]
    {
        for url in &msg.open_urls {
            let status = std::process::Command::new("open").arg(url).status();
            if let Err(e) = status {
                tracing::warn!(target: "clawenv::remote", "failed to open '{url}': {e}");
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = msg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_empty_when_all_granted() {
        let r = PermReport {
            accessibility: PermState::Granted,
            screen_capture: PermState::Granted,
        };
        assert!(guidance_for(&r).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn guidance_describes_both_missing_perms_on_macos() {
        let r = PermReport {
            accessibility: PermState::Denied,
            screen_capture: PermState::Denied,
        };
        let g = guidance_for(&r).expect("guidance");
        assert!(g.steps.iter().any(|s| s.contains("Accessibility")));
        assert!(g.steps.iter().any(|s| s.contains("Screen")));
        assert_eq!(g.open_urls.len(), 2);
    }
}
