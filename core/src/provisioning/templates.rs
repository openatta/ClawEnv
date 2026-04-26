//! Create-time template rendering for Lima / Podman / WSL.
//!
//! Three rendering paths, one data type ([`CreateOpts`]):
//!
//! - **Lima**: render `assets/lima/clawenv-alpine.yaml` with placeholders
//!   replaced; cloud-init runs the inlined provision script at first-boot.
//! - **Podman**: the `assets/podman/Containerfile` is used as-is —
//!   template substitution happens via `--build-arg` passed to
//!   `podman build`, not via file rewriting. So we compose a Vec of
//!   build-arg strings here and the Podman backend feeds them to the
//!   command.
//! - **WSL2**: there's no file asset — we compose an inline shell
//!   provision script in-memory and dispatch it via
//!   `run_background_script` (from R3-P1-b).
//!
//! Ported from v1 `core/src/sandbox/{lima,wsl,podman}.rs`'s create-time
//! template/script composition. Biggest difference: v2 derives
//! `PROXY_SCRIPT` from a structured [`ProxyTriple`] instead of accepting
//! a free-form string — fewer escape surprises.
//!
//! Design note: v1 uses literal `str.replace("{KEY}", value)`. We do
//! the same. Handlebars/Tera are overkill for 10 placeholders; a
//! templating library would also make golden-testing harder to read.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::proxy::ProxyTriple;

use super::mirrors::{provision_snippet, MirrorsConfig, DEFAULT_ALPINE_REPO};

/// Embedded Lima template (compile-time include_str! from v2/assets/).
pub const LIMA_TEMPLATE: &str = include_str!("../../../assets/v2/lima/clawenv-alpine.yaml");

/// Embedded Podman Containerfile — passed as-is to `podman build`.
pub const PODMAN_CONTAINERFILE: &str = include_str!("../../../assets/v2/podman/Containerfile");

/// Everything a backend needs to create a fresh sandbox.
///
/// Intentionally smaller than v1's `SandboxOpts` — v2 derives the
/// proxy shell script from a [`ProxyTriple`] and keeps
/// [`MirrorsConfig`] as its own type, rather than flattening
/// everything into string fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpts {
    /// VM / container instance name (user-chosen, e.g. "default").
    /// Used as -name arg for limactl/wsl, tag suffix for podman.
    pub instance_name: String,

    /// Host mount source for `/workspace` inside the VM. Typically
    /// `~/.clawenv/workspaces/<instance_name>`.
    pub workspace_dir: PathBuf,

    /// Primary host port; other service ports derive as offsets
    /// (ttyd = +1, bridge = +2, cdp = +3, vnc-ws = +4, dashboard = +5).
    pub gateway_port: u16,

    /// CPU cores to give the VM (Lima/WSL only; Podman uses host cores).
    pub cpu_cores: u32,

    /// Memory allocation (Lima/WSL only).
    pub memory_mb: u32,

    /// Proxy triple to inject at provision time. `None` = no proxy —
    /// emitted PROXY_SCRIPT is an empty-comment line so YAML stays valid.
    pub proxy: Option<ProxyTriple>,

    /// Mirror overrides (apk + npm). `MirrorsConfig::default()` = upstream.
    pub mirrors: MirrorsConfig,

    /// npm package name for the claw (e.g. "openclaw", "hermes").
    pub claw_package: String,

    /// npm version spec or "latest".
    pub claw_version: String,

    /// Install Chromium + VNC stack for browser automation.
    #[serde(default)]
    pub install_browser: bool,
}

impl CreateOpts {
    /// Default options for a test instance. Most fields are sensible
    /// but gateway_port defaults to 3000 which may conflict; callers
    /// should override for production use.
    pub fn minimal(name: impl Into<String>, claw_package: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            instance_name: name.clone(),
            workspace_dir: PathBuf::from(format!("/tmp/clawenv-workspaces/{name}")),
            gateway_port: 3000,
            cpu_cores: 2,
            memory_mb: 2048,
            proxy: None,
            mirrors: MirrorsConfig::default(),
            claw_package: claw_package.into(),
            claw_version: "latest".into(),
            install_browser: false,
        }
    }

    /// Computed service ports = gateway_port + offset.
    pub fn ttyd_port(&self) -> u16 { self.gateway_port + 1 }
    pub fn bridge_port(&self) -> u16 { self.gateway_port + 2 }
    pub fn cdp_port(&self) -> u16 { self.gateway_port + 3 }
    pub fn vnc_ws_port(&self) -> u16 { self.gateway_port + 4 }
    pub fn dashboard_port(&self) -> u16 { self.gateway_port + 5 }
}

// ——— Lima YAML rendering ———

/// Render the Lima template with placeholders filled in. Pure function
/// — no I/O. Golden-tested.
pub fn render_lima_yaml(opts: &CreateOpts) -> String {
    // The {PROXY_SCRIPT} / {ALPINE_MIRROR_FALLBACK_LINES} placeholders
    // sit inside a YAML `script: |` block scalar that's already
    // 4-space indented. We add the SAME 4 spaces to every line of the
    // substitution EXCEPT the first — the template line "    {…}"
    // already provides the first line's indent.
    // PROXY_SCRIPT placeholder is on a `    {PROXY_SCRIPT}` line —
    // template gives the first-line indent, we add it to continuations.
    let proxy_script = render_proxy_script(opts);
    let proxy_script_indented = indent_continuation(&proxy_script, "    ");

    // ALPINE_MIRROR_FALLBACK_LINES placeholder is at column 0 in the
    // template — every line of our substitution (including the first)
    // needs the 4-space prefix so the YAML block scalar boundary holds.
    let alpine_fallback = render_alpine_mirror_fallback(opts);
    let alpine_fallback_indented = indent_yaml_block(&alpine_fallback, "    ");

    LIMA_TEMPLATE
        .replace("{CPU_CORES}", &opts.cpu_cores.to_string())
        .replace("{MEMORY_MB}", &opts.memory_mb.to_string())
        .replace("{WORKSPACE_DIR}", &opts.workspace_dir.display().to_string())
        .replace("{GATEWAY_PORT}", &opts.gateway_port.to_string())
        .replace("{TTYD_PORT}", &opts.ttyd_port().to_string())
        .replace("{BRIDGE_PORT}", &opts.bridge_port().to_string())
        .replace("{CDP_PORT}", &opts.cdp_port().to_string())
        .replace("{VNC_WS_PORT}", &opts.vnc_ws_port().to_string())
        .replace("{DASHBOARD_PORT}", &opts.dashboard_port().to_string())
        .replace("{PROXY_SCRIPT}", &proxy_script_indented)
        .replace("{USER_ALPINE_MIRROR}", opts.mirrors.alpine_repo.as_str())
        .replace("{ALPINE_MIRROR_FALLBACK_LINES}", &alpine_fallback_indented)
}

/// Proxy + mirrors preamble for the cloud-init `provision:` block. Runs
/// BEFORE `apk update` so blocked networks can route through the proxy.
///
/// Composed from:
/// - `export http_proxy=…; export https_proxy=…; export no_proxy=…`
/// - The mirrors [provision snippet](super::mirrors::provision_snippet),
///   which detects Alpine version and sets `/etc/apk/repositories`.
fn render_proxy_script(opts: &CreateOpts) -> String {
    let mut s = String::new();
    if let Some(t) = &opts.proxy {
        if !t.http.is_empty() {
            s.push_str(&format!("export http_proxy='{}'\n", t.http));
            s.push_str(&format!("export HTTP_PROXY='{}'\n", t.http));
        }
        if !t.https.is_empty() {
            s.push_str(&format!("export https_proxy='{}'\n", t.https));
            s.push_str(&format!("export HTTPS_PROXY='{}'\n", t.https));
        }
        if !t.no_proxy.is_empty() {
            s.push_str(&format!("export no_proxy='{}'\n", t.no_proxy));
            s.push_str(&format!("export NO_PROXY='{}'\n", t.no_proxy));
        }
    }
    // Append mirrors snippet (apk repositories + npm registry), which
    // is also shell-idempotent and runs before apk update.
    s.push_str(&provision_snippet(&opts.mirrors));
    s
}

/// Prepend `indent` to every non-empty line. Required for YAML block
/// scalars — Lima's `provision.script:` is a `|`-scalar so every line
/// needs the same indent as the anchor. Use this when the template's
/// placeholder sits at column 0 (no template-provided first-line indent).
fn indent_yaml_block(s: &str, indent: &str) -> String {
    s.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{indent}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Indent every line EXCEPT the first. Use when the substitution
/// placeholder in the template already has its first-line indent
/// (e.g. `    {PLACEHOLDER}` puts 4 spaces before the first char of
/// our replacement). Continuation lines need the same indent so the
/// YAML block scalar boundary holds.
fn indent_continuation(s: &str, indent: &str) -> String {
    let mut out = String::new();
    let mut first = true;
    for l in s.lines() {
        if first {
            out.push_str(l);
            first = false;
        } else {
            out.push('\n');
            if !l.is_empty() {
                out.push_str(indent);
            }
            out.push_str(l);
        }
    }
    out
}

/// The `try_install` fallback line for the upstream Alpine mirror.
///
/// v1 generated N lines (one per tier); v0.3.0 collapsed to a single
/// upstream URL. v2 always emits this line so the provision script
/// has something to try even when the user hasn't configured an
/// `alpine_repo` override. When the user set `alpine_repo ==
/// DEFAULT_ALPINE_REPO` the fallback is a harmless double-try (same
/// URL again) — simpler than branching, and the shell short-circuits
/// on `SUCCESS=1` anyway.
fn render_alpine_mirror_fallback(_opts: &CreateOpts) -> String {
    let base = DEFAULT_ALPINE_REPO.trim_end_matches('/');
    format!("[ -z \"$SUCCESS\" ] && try_install '{base}' && SUCCESS=1")
}

// ——— Podman --build-arg rendering ———

/// Compose the argv list for `podman build`. Returns the complete arg
/// vector AFTER `podman build` and BEFORE the `-f <file> <context>`
/// tail. Caller appends those two and dispatches.
///
/// Also sets the image tag via `-t clawenv/<instance_name>:latest`.
pub fn render_podman_build_args(opts: &CreateOpts) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "build".into(),
        "-t".into(),
        format!("clawenv/{}:latest", opts.instance_name),
        "--build-arg".into(),
        format!("CLAW_PACKAGE={}", opts.claw_package),
        "--build-arg".into(),
        format!("CLAW_VERSION={}", opts.claw_version),
        "--build-arg".into(),
        format!(
            "INSTALL_BROWSER={}",
            if opts.install_browser { "true" } else { "false" }
        ),
    ];

    if !opts.mirrors.alpine_repo.is_empty() {
        args.push("--build-arg".into());
        args.push(format!("ALPINE_MIRROR={}", opts.mirrors.alpine_repo));
    }
    if !opts.mirrors.npm_registry.is_empty() {
        args.push("--build-arg".into());
        args.push(format!("NPM_REGISTRY={}", opts.mirrors.npm_registry));
    }

    if let Some(t) = &opts.proxy {
        if !t.http.is_empty() {
            args.push("--build-arg".into());
            args.push(format!("HTTP_PROXY={}", t.http));
        }
        if !t.https.is_empty() {
            args.push("--build-arg".into());
            args.push(format!("HTTPS_PROXY={}", t.https));
        }
        if !t.no_proxy.is_empty() {
            args.push("--build-arg".into());
            args.push(format!("NO_PROXY={}", t.no_proxy));
        }
    }
    args
}

// ——— WSL provision script rendering ———

/// Compose the inline shell script that runs after `wsl --import`.
/// WSL has no cloud-init equivalent; we just exec this script via
/// `run_background_script`.
///
/// Script stages:
/// 1. Proxy exports (before apk so blocked networks can route)
/// 2. apk mirrors configured
/// 3. apk update + apk add base packages
/// 4. npm install -g claw package (at provision time — cheap since
///    WSL has no separate "build" phase like Podman)
/// 5. Optional browser package bundle
/// 6. ssh-keygen + clawenv user
pub fn render_wsl_provision_script(opts: &CreateOpts) -> String {
    let mut s = String::from("#!/bin/sh\nset -e\n\n");
    // Proxy + mirrors preamble (reuses the same rendering as Lima).
    s.push_str(&render_proxy_script(opts));
    s.push('\n');
    // Base packages — same list as Lima/Podman for consistency.
    s.push_str(
        "apk update\n\
         apk add --no-cache git curl bash nodejs npm ttyd openssh build-base python3 procps\n",
    );
    s.push_str("ssh-keygen -A\n");
    // npm claw package
    s.push_str(&format!(
        "npm install -g {pkg}@{ver}\n",
        pkg = opts.claw_package,
        ver = opts.claw_version,
    ));
    if opts.install_browser {
        s.push_str(
            "apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont\n",
        );
    }
    // clawenv user (matches Lima's cloud-init semantics)
    s.push_str(
        "adduser -D -s /bin/bash clawenv || true\n\
         echo 'clawenv ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers\n\
         mkdir -p /usr/local/lib/node_modules /usr/local/bin\n\
         chown -R clawenv:clawenv /usr/local/lib/node_modules /usr/local/bin\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::ProxySource;

    fn opts_plain() -> CreateOpts { CreateOpts::minimal("default", "openclaw") }

    fn opts_with_proxy() -> CreateOpts {
        CreateOpts {
            proxy: Some(ProxyTriple {
                http: "http://host.lima.internal:7890".into(),
                https: "http://host.lima.internal:7890".into(),
                no_proxy: "localhost,127.0.0.1".into(),
                source: ProxySource::GlobalConfig,
            }),
            ..opts_plain()
        }
    }

    fn opts_with_mirrors() -> CreateOpts {
        CreateOpts {
            mirrors: MirrorsConfig {
                alpine_repo: "https://mirrors.aliyun.com/alpine".into(),
                npm_registry: "https://registry.npmmirror.com".into(),
            },
            ..opts_plain()
        }
    }

    // ——— Port derivation ———

    #[test]
    fn computed_ports_are_consecutive_offsets() {
        let o = CreateOpts::minimal("x", "y");
        assert_eq!(o.gateway_port, 3000);
        assert_eq!(o.ttyd_port(), 3001);
        assert_eq!(o.bridge_port(), 3002);
        assert_eq!(o.cdp_port(), 3003);
        assert_eq!(o.vnc_ws_port(), 3004);
        assert_eq!(o.dashboard_port(), 3005);
    }

    // ——— indent_yaml_block ———

    #[test]
    fn indent_adds_prefix_to_nonempty_lines() {
        // lines() consumes the trailing newline; the join re-emits
        // inter-line \n but not a final one. Matches how YAML block
        // scalars are laid out.
        let s = "foo\n\nbar\n";
        assert_eq!(indent_yaml_block(s, "    "), "    foo\n\n    bar");
    }

    #[test]
    fn indent_empty_input_yields_empty() {
        assert_eq!(indent_yaml_block("", "    "), "");
    }

    // ——— Lima YAML ———

    #[test]
    fn lima_yaml_replaces_all_port_placeholders() {
        let y = render_lima_yaml(&opts_plain());
        assert!(!y.contains("{GATEWAY_PORT}"), "unreplaced GATEWAY_PORT in:\n{y}");
        assert!(!y.contains("{TTYD_PORT}"));
        assert!(!y.contains("{BRIDGE_PORT}"));
        assert!(!y.contains("{CDP_PORT}"));
        assert!(!y.contains("{VNC_WS_PORT}"));
        assert!(!y.contains("{DASHBOARD_PORT}"));
        // Concrete values present.
        assert!(y.contains("hostPort: 3000"));
        assert!(y.contains("hostPort: 3005"));
    }

    #[test]
    fn lima_yaml_replaces_workspace_dir() {
        let y = render_lima_yaml(&opts_plain());
        assert!(!y.contains("{WORKSPACE_DIR}"), "unreplaced:\n{y}");
        assert!(y.contains("/tmp/clawenv-workspaces/default"));
    }

    #[test]
    fn lima_yaml_has_no_remaining_placeholders() {
        let y = render_lima_yaml(&opts_with_proxy());
        for tag in [
            "{CPU_CORES}", "{MEMORY_MB}",
            "{WORKSPACE_DIR}", "{GATEWAY_PORT}", "{TTYD_PORT}", "{BRIDGE_PORT}",
            "{CDP_PORT}", "{VNC_WS_PORT}", "{DASHBOARD_PORT}",
            "{PROXY_SCRIPT}", "{USER_ALPINE_MIRROR}", "{ALPINE_MIRROR_FALLBACK_LINES}",
        ] {
            assert!(!y.contains(tag), "unreplaced {tag} in:\n{y}");
        }
    }

    #[test]
    fn lima_yaml_carries_resource_opts() {
        let mut o = opts_plain();
        o.cpu_cores = 4;
        o.memory_mb = 8192;
        let y = render_lima_yaml(&o);
        // Lima 2.x accepts cpus as bare integer, memory as quoted string.
        assert!(y.contains("cpus: 4"), "missing cpus: 4\n{y}");
        assert!(y.contains(r#"memory: "8192MiB""#), "missing memory: \"8192MiB\"\n{y}");
    }

    #[test]
    fn lima_yaml_injects_proxy_exports() {
        let y = render_lima_yaml(&opts_with_proxy());
        assert!(y.contains("export http_proxy='http://host.lima.internal:7890'"));
        assert!(y.contains("export HTTP_PROXY='http://host.lima.internal:7890'"));
        assert!(y.contains("export no_proxy='localhost,127.0.0.1'"));
    }

    #[test]
    fn lima_yaml_emits_no_proxy_exports_without_proxy() {
        let y = render_lima_yaml(&opts_plain());
        assert!(!y.contains("export http_proxy="));
        assert!(!y.contains("export HTTP_PROXY="));
    }

    #[test]
    fn lima_yaml_proxy_block_indented_four_spaces() {
        let y = render_lima_yaml(&opts_with_proxy());
        // Every proxy export line should be indented by EXACTLY 4 spaces
        // so it lives inside the `script: |` block scalar without
        // accumulating extra leading whitespace.
        // Regression guard: the previous implementation double-indented
        // the first line (template provided 4 + we added 4 = 8) — Lima
        // tolerated it but cosmetic was off. Now indent_continuation
        // skips the first line.
        for line in y.lines() {
            if line.contains("export http_proxy=") {
                assert!(
                    line.starts_with("    export http_proxy="),
                    "expected exactly 4-space indent, got: {line:?}"
                );
            }
        }
    }

    // ——— indent_continuation ———

    #[test]
    fn indent_continuation_skips_first_line() {
        // First line keeps its original indent (template provides it);
        // subsequent lines get the prefix added.
        let s = "first\nsecond\nthird";
        assert_eq!(indent_continuation(s, "    "), "first\n    second\n    third");
    }

    #[test]
    fn indent_continuation_handles_empty_lines_in_body() {
        let s = "first\n\nsecond";
        assert_eq!(indent_continuation(s, "    "), "first\n\n    second");
    }

    #[test]
    fn indent_continuation_single_line_passthrough() {
        let s = "only";
        assert_eq!(indent_continuation(s, "    "), "only");
    }

    #[test]
    fn indent_continuation_empty_input_yields_empty() {
        assert_eq!(indent_continuation("", "    "), "");
    }

    #[test]
    fn lima_yaml_user_alpine_mirror_appears_in_script() {
        let y = render_lima_yaml(&opts_with_mirrors());
        // template has: USER_MIRROR="{USER_ALPINE_MIRROR}"
        assert!(y.contains("USER_MIRROR=\"https://mirrors.aliyun.com/alpine\""));
    }

    #[test]
    fn lima_yaml_always_has_upstream_fallback() {
        // Default mirrors: user line has empty USER_MIRROR (won't fire);
        // the fallback line MUST run so provision can install packages.
        let y = render_lima_yaml(&opts_plain());
        assert!(
            y.contains("try_install 'https://dl-cdn.alpinelinux.org/alpine'"),
            "missing upstream fallback line"
        );
    }

    #[test]
    fn lima_yaml_fallback_line_present_with_custom_user_mirror() {
        // User override AND upstream fallback coexist so a flaky user
        // mirror can fall through to dl-cdn.
        let y = render_lima_yaml(&opts_with_mirrors());
        assert!(
            y.contains("try_install 'https://dl-cdn.alpinelinux.org/alpine'"),
            "missing upstream fallback line with custom user mirror"
        );
        assert!(y.contains("USER_MIRROR=\"https://mirrors.aliyun.com/alpine\""));
    }

    // ——— Podman build args ———

    #[test]
    fn podman_args_start_with_build_and_tag() {
        let a = render_podman_build_args(&opts_plain());
        assert_eq!(a[0], "build");
        assert_eq!(a[1], "-t");
        assert_eq!(a[2], "clawenv/default:latest");
    }

    #[test]
    fn podman_args_always_include_claw_and_browser_flag() {
        let a = render_podman_build_args(&opts_plain());
        let joined = a.join(" ");
        assert!(joined.contains("CLAW_PACKAGE=openclaw"));
        assert!(joined.contains("CLAW_VERSION=latest"));
        assert!(joined.contains("INSTALL_BROWSER=false"));
    }

    #[test]
    fn podman_args_include_browser_true_when_flagged() {
        let o = CreateOpts { install_browser: true, ..opts_plain() };
        let a = render_podman_build_args(&o);
        assert!(a.iter().any(|s| s == "INSTALL_BROWSER=true"));
        assert!(!a.iter().any(|s| s == "INSTALL_BROWSER=false"));
    }

    #[test]
    fn podman_args_omit_proxy_when_absent() {
        let a = render_podman_build_args(&opts_plain());
        assert!(!a.iter().any(|s| s.starts_with("HTTP_PROXY=")));
        assert!(!a.iter().any(|s| s.starts_with("HTTPS_PROXY=")));
    }

    #[test]
    fn podman_args_include_proxy_build_args() {
        let a = render_podman_build_args(&opts_with_proxy());
        assert!(a.iter().any(|s| s == "HTTP_PROXY=http://host.lima.internal:7890"));
        assert!(a.iter().any(|s| s == "HTTPS_PROXY=http://host.lima.internal:7890"));
        assert!(a.iter().any(|s| s == "NO_PROXY=localhost,127.0.0.1"));
    }

    #[test]
    fn podman_args_include_mirrors_build_args() {
        let a = render_podman_build_args(&opts_with_mirrors());
        assert!(a.iter().any(|s| s == "ALPINE_MIRROR=https://mirrors.aliyun.com/alpine"));
        assert!(a.iter().any(|s| s == "NPM_REGISTRY=https://registry.npmmirror.com"));
    }

    // ——— WSL provision script ———

    #[test]
    fn wsl_script_starts_with_shebang() {
        let s = render_wsl_provision_script(&opts_plain());
        assert!(s.starts_with("#!/bin/sh\nset -e"));
    }

    #[test]
    fn wsl_script_contains_base_packages_and_ssh_keygen() {
        let s = render_wsl_provision_script(&opts_plain());
        assert!(s.contains("apk add --no-cache git curl bash nodejs npm ttyd openssh"));
        assert!(s.contains("ssh-keygen -A"));
    }

    #[test]
    fn wsl_script_embeds_claw_install() {
        let s = render_wsl_provision_script(&opts_plain());
        assert!(s.contains("npm install -g openclaw@latest"));
    }

    #[test]
    fn wsl_script_respects_version_override() {
        let o = CreateOpts { claw_version: "1.2.3".into(), ..opts_plain() };
        let s = render_wsl_provision_script(&o);
        assert!(s.contains("npm install -g openclaw@1.2.3"));
    }

    #[test]
    fn wsl_script_includes_proxy_exports() {
        let s = render_wsl_provision_script(&opts_with_proxy());
        assert!(s.contains("export http_proxy="));
        assert!(s.contains("export HTTPS_PROXY="));
    }

    #[test]
    fn wsl_script_includes_browser_bundle_when_flagged() {
        let o = CreateOpts { install_browser: true, ..opts_plain() };
        let s = render_wsl_provision_script(&o);
        assert!(s.contains("chromium"));
        assert!(s.contains("xvfb-run"));
    }

    #[test]
    fn wsl_script_skips_browser_when_unflagged() {
        let s = render_wsl_provision_script(&opts_plain());
        assert!(!s.contains("chromium"));
    }

    #[test]
    fn wsl_script_creates_clawenv_user() {
        let s = render_wsl_provision_script(&opts_plain());
        assert!(s.contains("adduser -D -s /bin/bash clawenv"));
        assert!(s.contains("NOPASSWD:ALL"));
    }
}
