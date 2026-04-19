use anyhow::Result;

use super::models::MirrorsConfig;
use crate::sandbox::SandboxBackend;

/// Apply mirror configuration inside a sandbox environment.
/// Replaces Alpine APK repositories and npm registry.
pub async fn apply_mirrors(backend: &dyn SandboxBackend, mirrors: &MirrorsConfig) -> Result<()> {
    if mirrors.is_default() {
        return Ok(());
    }

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
    tracing::info!("Alpine APK repositories set to {alpine_base}/v{v_short}");

    // ---- npm registry ----
    let npm_registry = mirrors.npm_registry_url();
    if npm_registry != "https://registry.npmjs.org" {
        backend
            .exec(&format!("npm config set registry '{npm_registry}'"))
            .await?;
        tracing::info!("npm registry set to {npm_registry}");
    }

    Ok(())
}

/// Generate the Alpine repositories content for use in provision scripts
/// (before the sandbox is fully booted, e.g., Lima YAML or WSL provision script).
///
/// Note: mirrors like Aliyun don't have a "latest-stable" path, so when the
/// exact version isn't known at provision time, we detect it from /etc/os-release.
pub fn alpine_repo_script(mirrors: &MirrorsConfig, _alpine_version: &str) -> String {
    let base = mirrors.alpine_repo_url();

    if base == "https://dl-cdn.alpinelinux.org/alpine" {
        // Default CDN — no need to override, Alpine images ship with correct repos
        return String::new();
    }

    // Detect version at runtime inside the sandbox (works on all Alpine images)
    format!(
        r#"ALPINE_VER=$(cat /etc/alpine-release 2>/dev/null | cut -d. -f1,2)
if [ -n "$ALPINE_VER" ]; then
  echo '{base}/v'"$ALPINE_VER"'/main' > /etc/apk/repositories
  echo '{base}/v'"$ALPINE_VER"'/community' >> /etc/apk/repositories
fi
"#
    )
}

/// Generate npm registry configuration command for provision scripts.
pub fn npm_registry_script(mirrors: &MirrorsConfig) -> String {
    let registry = mirrors.npm_registry_url();
    if registry == "https://registry.npmjs.org" {
        String::new()
    } else {
        format!("npm config set registry '{registry}'\n")
    }
}
