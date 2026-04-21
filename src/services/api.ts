import { invoke } from "@tauri-apps/api/core";
import type { Instance, ClawType } from "../types";

// ---------------------------------------------------------------------------
// Inline types (not in types.ts)
// ---------------------------------------------------------------------------

export type SystemCheckInfo = {
  os: string;
  arch: string;
  memory_gb: number;
  disk_free_gb: number;
  sandbox_backend: string;
  sandbox_available: boolean;
  checks: Array<{ name: string; ok: boolean; detail: string; info_only?: boolean }>;
};

export type SystemProxy = {
  detected: boolean;
  source: string;
  http_proxy: string;
  https_proxy: string;
  no_proxy: string;
};

export type ConnTestResult = {
  endpoint: string;
  ok: boolean;
  message: string;
};

export type ValidateImportResult = {
  valid: boolean;
  error: string;
  is_native: boolean;
};

export type SandboxVm = {
  name: string;
  status: string;
  cpus: string;
  memory: string;
  disk: string;
  dir_size: string;
  managed: boolean;
  ttyd_port?: number;
};

export type UpdateCheckResult = {
  current: string;
  latest: string;
  has_upgrade: boolean;
  is_security_release: boolean;
};

export type LaunchState = string;

// ---------------------------------------------------------------------------
// Instance operations
// ---------------------------------------------------------------------------

export async function listInstances(): Promise<Instance[]> {
  return invoke<Instance[]>("list_instances");
}

export async function startInstance(name: string): Promise<void> {
  return invoke<void>("start_instance", { name });
}

export async function stopInstance(name: string): Promise<void> {
  return invoke<void>("stop_instance", { name });
}

export async function getInstanceHealth(name: string): Promise<string> {
  return invoke<string>("get_instance_health", { name });
}

export async function getInstanceLogs(name: string): Promise<string> {
  return invoke<string>("get_instance_logs", { name });
}

export async function getGatewayToken(name: string): Promise<string> {
  return invoke<string>("get_gateway_token", { name });
}

export async function deleteInstance(name: string): Promise<void> {
  return invoke<void>("delete_instance", { name });
}

export async function deleteInstanceWithProgress(name: string): Promise<void> {
  return invoke<void>("delete_instance_with_progress", { name });
}

export async function editInstancePorts(
  name: string,
  gatewayPort: number,
  ttydPort: number,
): Promise<void> {
  return invoke<void>("edit_instance_ports", { name, gatewayPort, ttydPort });
}

export async function getInstanceCapabilities(
  name: string,
): Promise<Record<string, boolean>> {
  return invoke<Record<string, boolean>>("get_instance_capabilities", { name });
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

export async function installOpenclaw(params: {
  instanceName: string;
  clawType: string;
  clawVersion: string;
  useNative: boolean;
  installBrowser: boolean;
  installMcpBridge: boolean;
  gatewayPort: number;
  image: string;
  proxyJson?: string | null;
}): Promise<void> {
  return invoke<void>("install_openclaw", params);
}

export async function systemCheck(): Promise<SystemCheckInfo> {
  return invoke<SystemCheckInfo>("system_check");
}

export async function detectSystemProxy(): Promise<SystemProxy> {
  return invoke<SystemProxy>("detect_system_proxy");
}

export async function testConnectivity(
  proxyJson: string,
): Promise<ConnTestResult[]> {
  return invoke<ConnTestResult[]>("test_connectivity", { proxyJson });
}

export async function validateImportFile(
  filePath: string,
): Promise<ValidateImportResult> {
  return invoke<ValidateImportResult>("validate_import_file", { filePath });
}

export async function hasNativeInstance(): Promise<boolean> {
  return invoke<boolean>("has_native_instance");
}

export async function pickImportFile(): Promise<string> {
  return invoke<string>("pick_import_file");
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

export async function listSandboxVms(): Promise<SandboxVm[]> {
  return invoke<SandboxVm[]>("list_sandbox_vms");
}

export async function getSandboxDiskUsage(): Promise<string> {
  return invoke<string>("get_sandbox_disk_usage");
}

export async function sandboxVmAction(
  vmName: string,
  action: string,
): Promise<void> {
  return invoke<void>("sandbox_vm_action", { vmName, action });
}

export async function editInstanceResources(
  name: string,
  cpus: number,
  memoryMb: number,
  diskGb: number,
): Promise<void> {
  return invoke<void>("edit_instance_resources", { name, cpus, memoryMb, diskGb });
}

export async function checkChromiumInstalled(name: string): Promise<boolean> {
  return invoke<boolean>("check_chromium_installed", { name });
}

export async function installChromium(name: string): Promise<void> {
  return invoke<void>("install_chromium", { name });
}

export async function browserStartInteractive(name: string): Promise<string> {
  return invoke<string>("browser_start_interactive", { name });
}

export async function browserResumeHeadless(name: string): Promise<void> {
  return invoke<void>("browser_resume_headless", { name });
}

export async function hilComplete(): Promise<void> {
  return invoke<void>("hil_complete");
}

// ---------------------------------------------------------------------------
// Upgrade
// ---------------------------------------------------------------------------

export async function checkInstanceUpdate(
  name: string,
): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_instance_update", { name });
}

export async function upgradeInstance(
  name: string,
  targetVersion: string,
): Promise<void> {
  return invoke<void>("upgrade_instance", { name, targetVersion });
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export async function exportNativeBundle(name: string): Promise<string> {
  return invoke<string>("export_native_bundle", { name });
}

export async function exportSandbox(name: string): Promise<string> {
  return invoke<string>("export_sandbox", { name });
}

export async function exportCancel(): Promise<void> {
  return invoke<void>("export_cancel");
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export async function saveSettings(settingsJson: string): Promise<void> {
  return invoke<void>("save_settings", { settingsJson });
}

export async function getBridgeConfig(): Promise<any> {
  return invoke<any>("get_bridge_config");
}

export async function saveBridgeConfig(bridgeJson: string): Promise<void> {
  return invoke<void>("save_bridge_config", { bridgeJson });
}

export async function diagnoseInstances(): Promise<any> {
  return invoke<any>("diagnose_instances");
}

export async function fixDiagnosticIssue(
  instanceName: string,
  issueType: string,
): Promise<void> {
  return invoke<void>("fix_diagnostic_issue", { instanceName, issueType });
}

export async function autostartIsEnabled(): Promise<boolean> {
  return invoke<boolean>("autostart_is_enabled");
}

export async function autostartSet(enabled: boolean): Promise<void> {
  return invoke<void>("autostart_set", { enabled });
}

// ---------------------------------------------------------------------------
// Claw types
// ---------------------------------------------------------------------------

export async function listClawTypes(): Promise<ClawType[]> {
  return invoke<ClawType[]>("list_claw_types");
}

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------

export async function detectLaunchState(): Promise<LaunchState> {
  return invoke<LaunchState>("detect_launch_state");
}

export async function openInstallWindow(
  instanceName: string,
  clawType: string,
): Promise<void> {
  return invoke<void>("open_install_window", { instanceName, clawType });
}

export async function openUrlInBrowser(url: string): Promise<void> {
  return invoke<void>("open_url_in_browser", { url });
}

export async function stopAllInstances(): Promise<void> {
  return invoke<void>("stop_all_instances");
}

export async function exitApp(): Promise<void> {
  return invoke<void>("exit_app");
}

export async function execApprove(): Promise<void> {
  return invoke<void>("exec_approve");
}

export async function execDeny(): Promise<void> {
  return invoke<void>("exec_deny");
}
