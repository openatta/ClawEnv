//! v2 proxy subsystem — ported from v1's `config/{proxy,proxy_resolver}.rs`.
//!
//! Responsibilities:
//! - Store the user's proxy preference ([`ProxyConfig`])
//! - Compose an authenticated proxy URL with credentials from the
//!   [`credentials`](crate::credentials) vault
//! - Resolve the right [`ProxyTriple`] for each execution scope
//!   (installer / native-runtime / sandbox-runtime)
//! - Rewrite loopback URLs so they work from inside a sandbox
//!
//! What this module deliberately does NOT do (yet):
//! - OS-level proxy detection (macOS `scutil`, Windows registry,
//!   GNOME `gsettings`). Deferred — it pulls in platform-specific
//!   crates and is better implemented once for all of v2.
//! - Writing `/etc/environment` / `/etc/profile.d/proxy.sh` inside
//!   the VM. That lives in [`apply`].

pub mod config;
pub mod resolver;
pub mod url;
pub mod apply;

pub use config::{InstanceProxyConfig, InstanceProxyMode, ProxyConfig, ProxySource, ProxyTriple};
pub use resolver::{rewrite_loopback, sandbox_host_address, Scope};
pub use url::proxy_url_with_auth;
