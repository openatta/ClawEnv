//! OS-keychain-backed credential storage. Thin wrapper over the
//! `keyring` crate with a swappable backend for tests.
//!
//! **Compatibility**: we use the same `SERVICE_NAME = "clawenv"` and
//! key-naming convention as v1 (`core/src/config/keychain.rs`) so a
//! password stored by v1 is readable by v2 and vice versa. Renaming
//! the service name would orphan every stored password on users'
//! machines.
//!
//! Password keys:
//!
//! - `"proxy-password"` — the global proxy password
//! - `"proxy-password-<instance>"` — per-instance override
//!
//! Arbitrary `store/retrieve/delete` with a caller-chosen key is also
//! exposed so future features (API keys, bearer tokens) can share the
//! same vault.
//!
//! ## Architecture
//!
//! The module has a `CredentialBackend` trait with two impls:
//! - [`KeyringBackend`] — production; wraps `keyring::Entry`.
//! - [`MemoryBackend`] — in-memory HashMap for tests.
//!
//! A process-global `DEFAULT_BACKEND` is seeded with `KeyringBackend` on
//! first use. Tests override via [`set_default_backend`].

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};

use thiserror::Error;

/// macOS Keychain service name. MUST remain "clawenv" for v1 compat.
pub const SERVICE_NAME: &str = "clawenv";

/// Key under which the global proxy password is stored.
pub const PROXY_PASSWORD_KEY: &str = "proxy-password";

/// Per-instance proxy password key (e.g. `proxy-password-my-vm`).
pub fn instance_proxy_password_key(instance: &str) -> String {
    format!("{PROXY_PASSWORD_KEY}-{instance}")
}

#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("keychain entry not found: {key}")]
    NotFound { key: String },

    #[error("keychain I/O error on key `{key}`: {message}")]
    Io { key: String, message: String },
}

// ——— Backend trait + two impls ———

pub trait CredentialBackend: Send + Sync {
    fn store(&self, key: &str, value: &str) -> Result<(), CredentialError>;
    fn retrieve(&self, key: &str) -> Result<String, CredentialError>;
    fn delete(&self, key: &str) -> Result<(), CredentialError>;
}

pub struct KeyringBackend;

impl CredentialBackend for KeyringBackend {
    fn store(&self, key: &str, value: &str) -> Result<(), CredentialError> {
        let e = entry(key)?;
        e.set_password(value).map_err(|source| CredentialError::Io {
            key: key.into(),
            message: source.to_string(),
        })
    }

    fn retrieve(&self, key: &str) -> Result<String, CredentialError> {
        let e = entry(key)?;
        match e.get_password() {
            Ok(v) => Ok(v),
            Err(keyring::Error::NoEntry) => Err(CredentialError::NotFound { key: key.into() }),
            Err(source) => Err(CredentialError::Io {
                key: key.into(),
                message: source.to_string(),
            }),
        }
    }

    fn delete(&self, key: &str) -> Result<(), CredentialError> {
        let e = entry(key)?;
        match e.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(source) => Err(CredentialError::Io {
                key: key.into(),
                message: source.to_string(),
            }),
        }
    }
}

fn entry(key: &str) -> Result<keyring::Entry, CredentialError> {
    keyring::Entry::new(SERVICE_NAME, key).map_err(|e| CredentialError::Io {
        key: key.into(),
        message: e.to_string(),
    })
}

/// In-memory credential store. Per-instance state — pass an
/// `Arc<MemoryBackend>` into `set_default_backend` if you need
/// multiple handles pointing at the same vault.
pub struct MemoryBackend {
    inner: Mutex<HashMap<String, String>>,
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl CredentialBackend for MemoryBackend {
    fn store(&self, key: &str, value: &str) -> Result<(), CredentialError> {
        self.inner.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }

    fn retrieve(&self, key: &str) -> Result<String, CredentialError> {
        self.inner
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| CredentialError::NotFound { key: key.into() })
    }

    fn delete(&self, key: &str) -> Result<(), CredentialError> {
        self.inner.lock().unwrap().remove(key);
        Ok(())
    }
}

// ——— Process-global default ———

type Backend = std::sync::Arc<dyn CredentialBackend>;

fn default_slot() -> &'static RwLock<Backend> {
    static SLOT: OnceLock<RwLock<Backend>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(std::sync::Arc::new(KeyringBackend)))
}

/// Override the process-global backend. Intended for tests.
pub fn set_default_backend(backend: Backend) {
    *default_slot().write().unwrap() = backend;
}

fn backend() -> Backend {
    default_slot().read().unwrap().clone()
}

// ——— Free-function API; delegates to default backend ———

pub fn store(key: &str, value: &str) -> Result<(), CredentialError> {
    backend().store(key, value)
}

pub fn retrieve(key: &str) -> Result<String, CredentialError> {
    backend().retrieve(key)
}

pub fn delete(key: &str) -> Result<(), CredentialError> {
    backend().delete(key)
}

pub fn has(key: &str) -> bool {
    retrieve(key).is_ok()
}

// ——— High-level proxy password helpers ———

pub fn store_proxy_password(password: &str) -> Result<(), CredentialError> {
    store(PROXY_PASSWORD_KEY, password)
}

pub fn get_proxy_password() -> Result<String, CredentialError> {
    retrieve(PROXY_PASSWORD_KEY)
}

pub fn delete_proxy_password() -> Result<(), CredentialError> {
    delete(PROXY_PASSWORD_KEY)
}

pub fn store_instance_proxy_password(
    instance: &str,
    password: &str,
) -> Result<(), CredentialError> {
    store(&instance_proxy_password_key(instance), password)
}

pub fn get_instance_proxy_password(instance: &str) -> Result<String, CredentialError> {
    retrieve(&instance_proxy_password_key(instance))
}

pub fn delete_instance_proxy_password(instance: &str) -> Result<(), CredentialError> {
    delete(&instance_proxy_password_key(instance))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Every test in this module mutates the process-global backend;
    // serialize them. The fresh MemoryBackend per test gives isolation.
    static LOCK: Mutex<()> = Mutex::new(());

    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_default_backend(Arc::new(MemoryBackend::default()));
        g
    }

    #[test]
    fn instance_key_matches_v1_format() {
        assert_eq!(instance_proxy_password_key("my-vm"), "proxy-password-my-vm");
    }

    #[test]
    fn store_then_retrieve_roundtrip() {
        let _g = setup();
        store("test-k1", "hello-secret").unwrap();
        assert_eq!(retrieve("test-k1").unwrap(), "hello-secret");
    }

    #[test]
    fn overwrite_replaces_value() {
        let _g = setup();
        store("test-k2", "v1").unwrap();
        store("test-k2", "v2").unwrap();
        assert_eq!(retrieve("test-k2").unwrap(), "v2");
    }

    #[test]
    fn retrieve_missing_returns_not_found() {
        let _g = setup();
        let err = retrieve("test-nonexistent-zzz").unwrap_err();
        assert!(matches!(err, CredentialError::NotFound { .. }));
    }

    #[test]
    fn delete_is_idempotent() {
        let _g = setup();
        // Delete on missing must not fail.
        delete("test-never-existed").unwrap();
        store("test-k3", "v").unwrap();
        delete("test-k3").unwrap();
        assert!(!has("test-k3"));
        // Double-delete is also fine.
        delete("test-k3").unwrap();
    }

    #[test]
    fn has_reflects_presence() {
        let _g = setup();
        assert!(!has("test-k4"));
        store("test-k4", "v").unwrap();
        assert!(has("test-k4"));
    }

    #[test]
    fn proxy_password_helpers_use_canonical_key() {
        let _g = setup();
        store_proxy_password("pw").unwrap();
        assert_eq!(get_proxy_password().unwrap(), "pw");
        assert!(has(PROXY_PASSWORD_KEY));
        delete_proxy_password().unwrap();
        assert!(!has(PROXY_PASSWORD_KEY));
    }

    #[test]
    fn per_instance_password_keyed_by_instance_name() {
        let _g = setup();
        store_instance_proxy_password("alpha", "pw-a").unwrap();
        store_instance_proxy_password("beta", "pw-b").unwrap();
        assert_eq!(get_instance_proxy_password("alpha").unwrap(), "pw-a");
        assert_eq!(get_instance_proxy_password("beta").unwrap(), "pw-b");
        // Deleting one doesn't touch the other.
        delete_instance_proxy_password("alpha").unwrap();
        assert!(!has(&instance_proxy_password_key("alpha")));
        assert_eq!(get_instance_proxy_password("beta").unwrap(), "pw-b");
    }

    #[test]
    fn empty_string_value_roundtrips() {
        let _g = setup();
        store("test-empty", "").unwrap();
        assert_eq!(retrieve("test-empty").unwrap(), "");
    }

    #[test]
    fn memory_backend_direct_ops() {
        let b = MemoryBackend::default();
        assert!(matches!(
            b.retrieve("k").unwrap_err(),
            CredentialError::NotFound { .. }
        ));
        b.store("k", "v").unwrap();
        assert_eq!(b.retrieve("k").unwrap(), "v");
        b.delete("k").unwrap();
        assert!(matches!(
            b.retrieve("k").unwrap_err(),
            CredentialError::NotFound { .. }
        ));
    }
}
