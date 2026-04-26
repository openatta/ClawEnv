//! Compose a proxy URL with embedded credentials when the user has
//! `auth_required` set.
//!
//! Pulls the password from the credentials vault on demand rather than
//! caching it — renewals and rotations take effect immediately.

use crate::credentials::{self, CredentialError};

use super::config::ProxyConfig;

/// Turn a [`ProxyConfig`] into the URL that network clients should use.
///
/// - If auth isn't required, returns the `http_proxy` string verbatim.
/// - If auth IS required, pulls the password from the credentials vault
///   and splices in `user:password@`. A missing password is tolerated
///   (returns the URL with an empty password) — same behaviour as v1
///   so callers aren't forced to handle the "user configured auth but
///   never stored a password" edge case at every call site.
///
/// Returns [`UrlCompositionError`] only for I/O errors on the vault
/// (Keychain locked, daemon unreachable, etc.). A `NotFound` from the
/// vault is *not* an error — it's the expected state before the user
/// has ever set a password.
pub fn proxy_url_with_auth(proxy: &ProxyConfig) -> Result<String, UrlCompositionError> {
    if !proxy.auth_required || proxy.auth_user.is_empty() {
        return Ok(proxy.http_proxy.clone());
    }
    let password = match credentials::get_proxy_password() {
        Ok(p) => p,
        Err(CredentialError::NotFound { .. }) => String::new(),
        Err(e) => return Err(UrlCompositionError::CredentialLookup(e)),
    };
    Ok(splice_credentials(&proxy.http_proxy, &proxy.auth_user, &password))
}

/// Pure string op — splice `user:password@` into the URL just after the
/// scheme. Factored out so it can be unit-tested without touching the
/// credentials vault.
fn splice_credentials(url: &str, user: &str, password: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        format!("http://{user}:{password}@{rest}")
    } else if let Some(rest) = url.strip_prefix("https://") {
        format!("https://{user}:{password}@{rest}")
    } else {
        // v1 quirk: if there's no scheme, assume http.
        format!("http://{user}:{password}@{url}")
    }
}

#[derive(thiserror::Error, Debug)]
pub enum UrlCompositionError {
    #[error("credential lookup failed: {0}")]
    CredentialLookup(#[source] CredentialError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{set_default_backend, MemoryBackend};
    use std::sync::{Arc, Mutex};

    // Tests touch the process-global credentials backend; serialize them.
    static LOCK: Mutex<()> = Mutex::new(());

    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_default_backend(Arc::new(MemoryBackend::default()));
        g
    }

    #[test]
    fn splice_into_http_scheme() {
        assert_eq!(
            splice_credentials("http://proxy.corp:3128", "alice", "s3cret"),
            "http://alice:s3cret@proxy.corp:3128"
        );
    }

    #[test]
    fn splice_into_https_scheme() {
        assert_eq!(
            splice_credentials("https://proxy.corp:3128", "alice", "s3cret"),
            "https://alice:s3cret@proxy.corp:3128"
        );
    }

    #[test]
    fn splice_without_scheme_assumes_http() {
        assert_eq!(
            splice_credentials("proxy.corp:3128", "alice", "s3cret"),
            "http://alice:s3cret@proxy.corp:3128"
        );
    }

    #[test]
    fn splice_with_empty_password() {
        // Not great URL hygiene but matches v1 behavior; the URL parser
        // on the other end accepts it.
        assert_eq!(
            splice_credentials("http://proxy:3128", "alice", ""),
            "http://alice:@proxy:3128"
        );
    }

    #[test]
    fn auth_not_required_returns_url_verbatim() {
        let _g = setup();
        let p = ProxyConfig {
            enabled: true,
            http_proxy: "http://proxy:3128".into(),
            auth_required: false,
            ..Default::default()
        };
        assert_eq!(proxy_url_with_auth(&p).unwrap(), "http://proxy:3128");
    }

    #[test]
    fn auth_required_but_no_user_returns_verbatim() {
        let _g = setup();
        let p = ProxyConfig {
            enabled: true,
            http_proxy: "http://proxy:3128".into(),
            auth_required: true,
            auth_user: String::new(),
            ..Default::default()
        };
        assert_eq!(proxy_url_with_auth(&p).unwrap(), "http://proxy:3128");
    }

    #[test]
    fn auth_required_with_user_but_no_stored_password_uses_empty() {
        let _g = setup();
        let p = ProxyConfig {
            enabled: true,
            http_proxy: "http://proxy:3128".into(),
            auth_required: true,
            auth_user: "alice".into(),
            ..Default::default()
        };
        // No password ever stored — still returns a URL, not an error.
        assert_eq!(
            proxy_url_with_auth(&p).unwrap(),
            "http://alice:@proxy:3128"
        );
    }

    #[test]
    fn auth_required_with_stored_password_splices_it_in() {
        let _g = setup();
        credentials::store_proxy_password("s3cret").unwrap();
        let p = ProxyConfig {
            enabled: true,
            http_proxy: "http://proxy:3128".into(),
            auth_required: true,
            auth_user: "alice".into(),
            ..Default::default()
        };
        assert_eq!(
            proxy_url_with_auth(&p).unwrap(),
            "http://alice:s3cret@proxy:3128"
        );
    }
}
