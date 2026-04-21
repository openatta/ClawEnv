//! In-VM mirror application: writes `/etc/apk/repositories` and
//! `npm config set registry`.
//!
//! v0.3.0: the URL list collapsed to one upstream URL per asset (see
//! `assets/mirrors.toml` and `mirrors_asset.rs`). The prior multi-tier
//! fallback machinery — apk's multi-line repositories file, npm
//! reachability preflight across candidate registries — is gone. Now
//! we write a single repo base and a single registry, and fail clean
//! if the user can't reach them.

use anyhow::Result;

use super::models::MirrorsConfig;
use crate::sandbox::SandboxBackend;

/// Apply mirror configuration inside a sandbox environment.
/// Writes `/etc/apk/repositories` and (optionally) `npm config set registry`.
pub async fn apply_mirrors(
    backend: &dyn SandboxBackend,
    mirrors: &MirrorsConfig,
) -> Result<()> {
    // ---- Alpine APK repositories ----
    let alpine_base = mirrors.alpine_repo_url();

    // Detect Alpine version inside the sandbox
    let version_id = backend
        .exec("grep -oP 'VERSION_ID=\\K.*' /etc/os-release 2>/dev/null || echo '3.23'")
        .await
        .unwrap_or_else(|_| "3.23".into());
    let version_id = version_id.trim();
    // Extract major.minor (e.g., "3.23.2" → "3.23")
    let parts: Vec<&str> = version_id.split('.').collect();
    let v_short = if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version_id.to_string()
    };

    // Single-line repositories file (main + community). Previous versions
    // emitted one pair of lines per mirror tier because apk reads the file
    // top-down and retries the next line on failure — useful when a CN
    // mirror was paired with dl-cdn upstream. With the tier collapse in
    // v0.3.0 this is always one base, so two lines total.
    let repos = format!(
        "{alpine_base}/v{v_short}/main\n{alpine_base}/v{v_short}/community\n"
    );
    // /etc/apk/repositories is root-owned; `limactl shell` default user is
    // clawenv (NOPASSWD sudo available). Stream through `sudo tee` rather
    // than `cat > /etc/...` to avoid Permission denied.
    backend
        .exec(&format!(
            "sudo tee /etc/apk/repositories > /dev/null << 'REPOEOF'\n{repos}REPOEOF"
        ))
        .await?;
    tracing::info!("Alpine APK repositories written: {alpine_base} v{v_short}");

    // ---- npm registry ----
    // Only touch npm config when the user overrode the registry. Default
    // upstream (registry.npmjs.org) is npm's bundled value — setting it
    // explicitly is a no-op. When a user override is set we trust the
    // user; we don't preflight-probe the URL (contract: network is the
    // user's problem — if their custom registry is down, `npm install`
    // will surface the real error immediately).
    let npm = mirrors.npm_registry_url();
    if npm != "https://registry.npmjs.org" {
        backend
            .exec(&format!("npm config set registry '{npm}'"))
            .await?;
        tracing::info!("npm registry set to {npm}");
    }

    Ok(())
}

/// Generate the Alpine repositories content for use in provision scripts
/// (before the sandbox is fully booted, e.g., Lima YAML or WSL provision
/// script). Single base → two lines (main + community).
pub fn alpine_repo_script(
    mirrors: &MirrorsConfig,
    _alpine_version: &str,
) -> String {
    let base = mirrors.alpine_repo_url();
    // Detect version at runtime inside the sandbox (works on all Alpine
    // images).
    format!(
        "ALPINE_VER=$(cat /etc/alpine-release 2>/dev/null | cut -d. -f1,2)\n\
         if [ -n \"$ALPINE_VER\" ]; then\n  \
           echo '{base}/v'\"$ALPINE_VER\"'/main' > /etc/apk/repositories\n  \
           echo '{base}/v'\"$ALPINE_VER\"'/community' >> /etc/apk/repositories\n\
         fi\n"
    )
}

/// Generate npm registry configuration command for provision scripts.
/// Returns an empty string when the effective registry is the upstream
/// default (no-op — npm already uses it). Returns a single
/// `npm config set` line when the user has configured an override.
pub fn npm_registry_script(mirrors: &MirrorsConfig) -> String {
    let reg = mirrors.npm_registry_url();
    if reg == "https://registry.npmjs.org" {
        return String::new();
    }
    format!("npm config set registry '{reg}'\n")
}
