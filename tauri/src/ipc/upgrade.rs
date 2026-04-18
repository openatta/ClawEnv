use clawenv_core::api::UpdateCheckResponse;
use tauri::Emitter;

use crate::cli_bridge::{self, CliEvent};
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

#[tauri::command]
pub async fn check_instance_update(name: String) -> Result<UpdateCheckResponse, String> {
    let data = cli_bridge::run_cli(&["update-check", &name]).await.map_err(|e| e.to_string())?;
    let resp: UpdateCheckResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp)
}

#[tauri::command]
pub async fn upgrade_instance(app: tauri::AppHandle, name: String, target_version: Option<String>) -> Result<(), String> {
    let mut args = vec!["upgrade".to_string(), name.clone()];
    if let Some(ver) = target_version {
        args.push("--version".to_string());
        args.push(ver);
    }

    let app_handle = app.clone();
    let instance_name = name.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CliEvent>(32);

        let app_fwd = app_handle.clone();
        let fwd_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Forward all event types to frontend
                let _ = app_fwd.emit("upgrade-progress", &event);
            }
        });

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let result = cli_bridge::run_cli_streaming(&args_ref, tx).await;
        fwd_task.await.ok();

        match result {
            Ok(v) => {
                let ver = v.as_str().unwrap_or("unknown");
                let _ = app_handle.emit("upgrade-complete", ver);
                emit_instance_changed(
                    &app_handle,
                    InstanceChanged::simple(InstanceAction::Upgrade, &instance_name),
                );
            }
            Err(e) => {
                let _ = app_handle.emit("upgrade-failed", &e.to_string());
            }
        }
    });

    Ok(())
}
