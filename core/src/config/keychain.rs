use anyhow::{anyhow, Result};

const SERVICE_NAME: &str = "clawenv";

/// Store a secret in the system keychain
pub fn store(key: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)?;
    entry.set_password(value).map_err(|e| anyhow!("Keychain store failed: {e}"))?;
    tracing::debug!("Stored secret for key '{key}' in keychain");
    Ok(())
}

/// Retrieve a secret from the system keychain
pub fn retrieve(key: &str) -> Result<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)?;
    entry.get_password().map_err(|e| anyhow!("Keychain retrieve failed for '{key}': {e}"))
}

/// Delete a secret from the system keychain
pub fn delete(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)?;
    entry.delete_credential().map_err(|e| anyhow!("Keychain delete failed for '{key}': {e}"))?;
    Ok(())
}

/// Store API key for an instance
pub fn store_api_key(instance_name: &str, api_key: &str) -> Result<()> {
    store(&format!("apikey-{instance_name}"), api_key)
}

/// Retrieve API key for an instance
pub fn get_api_key(instance_name: &str) -> Result<String> {
    retrieve(&format!("apikey-{instance_name}"))
}

/// Store proxy password (global / Installer scope)
pub fn store_proxy_password(password: &str) -> Result<()> {
    store("proxy-password", password)
}

/// Retrieve proxy password (global / Installer scope)
pub fn get_proxy_password() -> Result<String> {
    retrieve("proxy-password")
}

/// Store proxy password for a specific VM instance. Separate namespace
/// from the global password (`proxy-password`) so per-VM credentials
/// don't leak across instances. Deleted automatically when the instance
/// is removed.
pub fn store_instance_proxy_password(instance: &str, password: &str) -> Result<()> {
    store(&format!("proxy-password-{instance}"), password)
}

pub fn get_instance_proxy_password(instance: &str) -> Result<String> {
    retrieve(&format!("proxy-password-{instance}"))
}

pub fn delete_instance_proxy_password(instance: &str) -> Result<()> {
    delete(&format!("proxy-password-{instance}"))
}
