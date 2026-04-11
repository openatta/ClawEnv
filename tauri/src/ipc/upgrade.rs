use clawenv_core::config::{ConfigManager, UserMode};
use clawenv_core::manager::{instance, upgrade};
use tauri::Emitter;

#[tauri::command]
pub async fn check_instance_update(name: String) -> Result<clawenv_core::update::checker::VersionInfo, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let npm_registry = config.config().clawenv.mirrors.npm_registry_url().to_string();
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let registry = clawenv_core::claw::ClawRegistry::load();
    let desc = registry.get(&inst.claw_type);
    upgrade::check_upgrade(inst, &npm_registry, &desc.npm_package).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn upgrade_instance(app: tauri::AppHandle, name: String, target_version: Option<String>) -> Result<(), String> {
    let mut config = ConfigManager::load()
        .or_else(|_| ConfigManager::create_default(UserMode::General))
        .map_err(|e| e.to_string())?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = app_handle.emit("upgrade-progress", &progress);
        }
    });

    let app_done = app.clone();
    let instance_name = name.clone();
    tokio::spawn(async move {
        let target = target_version.as_deref();
        match upgrade::upgrade_instance(&mut config, &instance_name, target, &tx).await {
            Ok(new_ver) => {
                let _ = app_done.emit("upgrade-complete", &new_ver);
            }
            Err(e) => {
                let _ = app_done.emit("upgrade-failed", &e.to_string());
            }
        }
    });

    Ok(())
}
