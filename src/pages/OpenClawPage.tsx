import { createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Instance, UpgradeInfo, UpgradeProgress } from "../types";

export default function OpenClawPage(props: {
  instances: Instance[];
  healths: Record<string, string>;
  onInstancesChanged?: () => void;
  onAddInstance?: () => void;
}) {
  const [activeTab, setActiveTab] = createSignal(props.instances[0]?.name ?? "");

  const activeInstance = () => props.instances.find((i) => i.name === activeTab());
  const activeHealth = () => props.healths[activeTab()] || "unreachable";
  const isRunning = () => activeHealth() === "running";

  function healthColor(name: string): string {
    const h = props.healths[name] ?? "unknown";
    if (h === "running") return "bg-green-500";
    if (h === "stopped") return "bg-gray-500";
    return "bg-red-500";
  }

  // actionLoading format: "action:instanceName" e.g. "start:default"
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal("");
  const [showDeleteConfirm, setShowDeleteConfirm] = createSignal(false);
  const [gatewayToken, setGatewayToken] = createSignal("");

  // Upgrade state
  const [updateInfo, setUpdateInfo] = createSignal<UpgradeInfo | null>(null);
  const [showUpgrade, setShowUpgrade] = createSignal(false);
  const [upgrading, setUpgrading] = createSignal(false);
  const [upgradeProgress, setUpgradeProgress] = createSignal(0);
  const [upgradeMessage, setUpgradeMessage] = createSignal("");
  const [upgradeError, setUpgradeError] = createSignal("");

  // Listen for update-available events from background checker
  onMount(async () => {
    const unUpdate = await listen<UpgradeInfo>("update-available", (ev) => {
      if (ev.payload.instance === activeTab()) {
        setUpdateInfo(ev.payload);
      }
    });
    const unProgress = await listen<UpgradeProgress>("upgrade-progress", (ev) => {
      setUpgradeProgress(ev.payload.percent);
      setUpgradeMessage(ev.payload.message);
    });
    const unComplete = await listen<string>("upgrade-complete", (ev) => {
      setUpgrading(false);
      setShowUpgrade(false);
      setUpdateInfo(null);
      props.onInstancesChanged?.();
    });
    const unFailed = await listen<string>("upgrade-failed", (ev) => {
      setUpgrading(false);
      setUpgradeError(String(ev.payload));
    });
    onCleanup(() => { unUpdate(); unProgress(); unComplete(); unFailed(); });

    // Also check now for active instance
    checkUpdate();
  });

  async function checkUpdate() {
    const name = activeTab();
    if (!name) return;
    try {
      const info = await invoke<{ current: string; latest: string; has_upgrade: boolean; is_security_release: boolean }>(
        "check_openclaw_update", { name }
      );
      if (info.has_upgrade) {
        setUpdateInfo({ instance: name, current: info.current, latest: info.latest, security: info.is_security_release });
      }
    } catch { /* ignore check failures */ }
  }

  async function startUpgrade() {
    setUpgrading(true);
    setUpgradeError("");
    setUpgradeProgress(0);
    setUpgradeMessage("Starting...");
    try {
      await invoke("upgrade_openclaw", { name: activeTab(), targetVersion: null });
    } catch (e) {
      setUpgrading(false);
      setUpgradeError(String(e));
    }
  }

  async function fetchToken() {
    try {
      const token = await invoke<string>("get_gateway_token", { name: activeTab() });
      setGatewayToken(token);
    } catch { setGatewayToken(""); }
  }
  fetchToken();

  async function doAction(action: string) {
    setActionLoading(`${action}:${activeTab()}`);
    setActionError("");
    try {
      if (action === "start") {
        await invoke("start_instance", { name: activeTab() });
      } else if (action === "stop") {
        await invoke("stop_instance", { name: activeTab() });
      } else if (action === "restart") {
        await invoke("stop_instance", { name: activeTab() });
        await invoke("start_instance", { name: activeTab() });
      }
      props.onInstancesChanged?.();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setActionLoading(null);
    }
  }

  async function doDelete() {
    setShowDeleteConfirm(false);
    setActionLoading(`delete:${activeTab()}`);
    setActionError("");
    try {
      await invoke("delete_instance", { name: activeTab() });
      props.onInstancesChanged?.();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setActionLoading(null);
    }
  }

  async function openInBrowser() {
    const inst = activeInstance();
    const port = inst?.gateway_port ?? 3000;
    const token = gatewayToken();
    const url = token ? `http://127.0.0.1:${port}/?token=${token}` : `http://127.0.0.1:${port}`;
    try { await invoke("open_url_in_browser", { url }); }
    catch { prompt("Copy this URL:", url); }
  }

  const loading = (action: string) => actionLoading() === `${action}:${activeTab()}`;
  const anyLoading = () => actionLoading()?.endsWith(`:${activeTab()}`) ?? false;

  // Config panel
  const [showConfig, setShowConfig] = createSignal(false);
  const [cfgGatewayPort, setCfgGatewayPort] = createSignal(3000);
  const [cfgTtydPort, setCfgTtydPort] = createSignal(7681);
  const [cfgSaving, setCfgSaving] = createSignal(false);
  const [cfgError, setCfgError] = createSignal("");
  const [caps, setCaps] = createSignal<Record<string,boolean>>({});
  let gatewayPortRef: HTMLInputElement | undefined;
  let ttydPortRef: HTMLInputElement | undefined;

  async function openConfig() {
    const inst = activeInstance();
    if (!inst) return;
    setCfgGatewayPort(inst.gateway_port);
    setCfgTtydPort(inst.ttyd_port);
    setCfgError("");
    try {
      const c = await invoke<Record<string,boolean>>("get_instance_capabilities", { name: inst.name });
      setCaps(c);
    } catch { setCaps({}); }
    setShowConfig(true);
    // Force set input values after dialog renders
    setTimeout(() => {
      if (gatewayPortRef) gatewayPortRef.value = String(inst.gateway_port);
      if (ttydPortRef) ttydPortRef.value = String(inst.ttyd_port);
    }, 0);
  }

  // Check if ports conflict with other instances
  const portConflict = () => {
    const gp = cfgGatewayPort();
    const tp = cfgTtydPort();
    const current = activeTab();
    if (gp === tp) return "Gateway and terminal ports cannot be the same";
    for (const inst of props.instances) {
      if (inst.name === current) continue;
      if (inst.gateway_port === gp || inst.ttyd_port === gp) return `Port ${gp} already used by "${inst.name}"`;
      if (inst.gateway_port === tp || inst.ttyd_port === tp) return `Port ${tp} already used by "${inst.name}"`;
    }
    if (gp < 1024 || gp > 65535) return "Gateway port must be 1024-65535";
    if (tp < 1024 || tp > 65535) return "Terminal port must be 1024-65535";
    return "";
  };

  async function saveConfig(restart: boolean) {
    const conflict = portConflict();
    if (conflict) { setCfgError(conflict); return; }
    setCfgSaving(true); setCfgError("");
    try {
      await invoke("edit_instance_ports", {
        name: activeTab(),
        gatewayPort: cfgGatewayPort(),
        ttydPort: cfgTtydPort(),
      });
      if (restart) {
        await invoke("start_instance", { name: activeTab() });
      }
      setShowConfig(false);
      props.onInstancesChanged?.();
    } catch (e) { setCfgError(String(e)); }
    finally { setCfgSaving(false); }
  }

  return (
    <div class="h-full flex flex-col">
      {/* Top bar with + button */}
      <div class="flex items-center justify-between px-4 py-2 border-b border-gray-800 shrink-0">
        <span class="font-medium">OpenClaw</span>
        <button
          class="w-6 h-6 flex items-center justify-center rounded bg-gray-700 hover:bg-indigo-600 text-sm font-bold"
          title="New Instance"
          onClick={() => props.onAddInstance?.()}
        >+</button>
      </div>

      {/* Tab bar */}
      <div class="flex border-b border-gray-800 px-2 shrink-0">
        <For each={props.instances}>
          {(inst) => (
            <button
              class={`px-3 py-2 text-sm border-b-2 transition-colors ${
                activeTab() === inst.name
                  ? "border-indigo-500 text-white"
                  : "border-transparent text-gray-400 hover:text-gray-200"
              }`}
              onClick={() => { setActiveTab(inst.name); fetchToken(); }}
            >
              <span class="flex items-center gap-1.5">
                <span class={`w-1.5 h-1.5 rounded-full ${healthColor(inst.name)}`} />
                {inst.name}
              </span>
            </button>
          )}
        </For>
      </div>

      {/* Content area */}
      <div class="flex-1 flex items-center justify-center bg-gray-950">
        <Show when={!activeInstance()}>
          <div class="text-center text-gray-500">
            <p class="mb-4">No instances yet</p>
            <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
              onClick={() => props.onAddInstance?.()}>Create Instance</button>
          </div>
        </Show>

        <Show when={activeInstance()}>
          <div class="text-center max-w-lg w-full">
            {/* Icon + status */}
            <div class="mb-4">
              <span class={`text-5xl ${isRunning() ? "" : "opacity-30"}`}>🦞</span>
            </div>
            <h2 class="text-xl font-bold mb-1">
              {isRunning() ? "OpenClaw is Running" : "OpenClaw is Stopped"}
            </h2>
            <p class="text-sm text-gray-400 mb-5">
              {isRunning()
                ? `Gateway active on port ${activeInstance()?.gateway_port}`
                : `Instance "${activeTab()}" is ${activeHealth()}`}
            </p>

            {/* Action buttons */}
            <div class="flex items-center justify-center gap-2 mb-2">
              <Show when={!isRunning() && !anyLoading()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                  onClick={() => doAction("start")}>
                  ▶ Start
                </button>
              </Show>
              <Show when={isRunning() && !anyLoading()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                  onClick={openInBrowser}>
                  Open Control Panel ↗
                </button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                  onClick={() => doAction("stop")}>
                  ⏹ Stop
                </button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                  onClick={() => doAction("restart")}>
                  ↻ Restart
                </button>
              </Show>
              <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                onClick={openConfig}>
                ⚙ Configure
              </button>
              <button class="px-3 py-2 bg-red-900/60 hover:bg-red-800 text-red-300 rounded text-sm disabled:opacity-50 ml-2"
                disabled={anyLoading()} onClick={() => setShowDeleteConfirm(true)}>
                {loading("delete") ? "Deleting..." : "Delete"}
              </button>
            </div>

            {/* Loading indicator */}
            <Show when={anyLoading()}>
              <div class="flex items-center justify-center gap-2 mb-2 text-sm text-indigo-300">
                <span class="animate-pulse">●</span>
                {loading("start") && "Starting instance, please wait..."}
                {loading("stop") && "Stopping instance..."}
                {loading("restart") && "Restarting instance, please wait..."}
                {loading("delete") && "Deleting instance..."}
              </div>
            </Show>

            {actionError() && <p class="text-xs text-red-400 mb-3">{actionError()}</p>}

            {/* Upgrade banner */}
            <Show when={updateInfo() && updateInfo()!.instance === activeTab()}>
              <div class={`rounded-lg p-3 mb-3 text-sm flex items-center justify-between ${
                updateInfo()!.security ? "bg-red-900/30 border border-red-700" : "bg-indigo-900/30 border border-indigo-700"
              }`}>
                <div>
                  <span class={updateInfo()!.security ? "text-red-300" : "text-indigo-300"}>
                    {updateInfo()!.security ? "⚠ Security update" : "Update available"}:
                  </span>
                  <span class="text-gray-300 ml-1">
                    {updateInfo()!.current} → {updateInfo()!.latest}
                  </span>
                </div>
                <button class="px-3 py-1 bg-indigo-600 hover:bg-indigo-500 rounded text-xs text-white"
                  onClick={() => setShowUpgrade(true)}>
                  Upgrade
                </button>
              </div>
            </Show>

            {/* Info table */}
            <div class="bg-gray-900 rounded-lg p-4 text-left text-xs text-gray-500 mx-auto max-w-xl">
              <table class="w-full">
                <tbody>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Instance</td><td>{activeInstance()?.name}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Version</td><td>{activeInstance()?.version}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Sandbox</td><td>{activeInstance()?.sandbox_type}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Gateway</td><td class="font-mono">http://127.0.0.1:{activeInstance()?.gateway_port}</td></tr>
                  <Show when={isRunning()}>
                    <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Token</td><td class="font-mono text-gray-300 break-all">{gatewayToken() || "..."}</td></tr>
                  </Show>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Status</td>
                    <td class={isRunning() ? "text-green-400" : "text-gray-500"}>
                      {isRunning() ? "● running" : "○ " + activeHealth()}
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </Show>
      </div>

      {/* Config dialog */}
      <Show when={showConfig()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl">
            <h3 class="text-base font-bold mb-4">Configure — {activeTab()}</h3>
            <div class="space-y-3">
              <div>
                <label class="block text-xs text-gray-400 mb-1">Gateway Port</label>
                <input ref={gatewayPortRef} type="number"
                  onInput={(e) => {
                    const v = parseInt(e.currentTarget.value) || 3000;
                    setCfgGatewayPort(v);
                    setCfgTtydPort(v + 4681);
                    if (ttydPortRef) ttydPortRef.value = String(v + 4681);
                  }}
                  class="bg-gray-900 border border-gray-600 rounded px-3 py-1.5 w-full text-sm" />
              </div>
              <div>
                <label class="block text-xs text-gray-400 mb-1">Terminal (ttyd) Port</label>
                <input ref={ttydPortRef} type="number"
                  onInput={(e) => setCfgTtydPort(parseInt(e.currentTarget.value) || 7681)}
                  class="bg-gray-900 border border-gray-600 rounded px-3 py-1.5 w-full text-sm" />
              </div>
            </div>
            <Show when={!caps().port_edit}>
              <p class="text-xs text-gray-500 mt-2">Port forwarding not supported by this backend. Config saved locally only.</p>
            </Show>
            {portConflict() && <p class="text-xs text-red-400 mt-2">{portConflict()}</p>}
            {cfgError() && !portConflict() && <p class="text-xs text-red-400 mt-2">{cfgError()}</p>}
            <p class="text-xs text-yellow-500 mt-2">Port changes require instance restart.</p>
            <div class="flex gap-2 justify-end mt-4">
              <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                onClick={() => setShowConfig(false)}>Cancel</button>
              <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
                disabled={cfgSaving() || !!portConflict()} onClick={() => saveConfig(false)}>
                {cfgSaving() ? "..." : "Save"}
              </button>
              <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
                disabled={cfgSaving() || !!portConflict()} onClick={() => saveConfig(true)}>
                {cfgSaving() ? "..." : "Save & Restart"}
              </button>
            </div>
          </div>
        </div>
      </Show>

      {/* Delete confirmation dialog */}
      <Show when={showDeleteConfirm()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-6 max-w-sm shadow-2xl">
            <h3 class="text-lg font-bold text-red-400 mb-2">Delete Instance</h3>
            <p class="text-sm text-gray-300 mb-1">
              Are you sure you want to delete <strong>"{activeTab()}"</strong>?
            </p>
            <p class="text-xs text-gray-500 mb-4">
              This will stop the instance and destroy the VM. All data inside the sandbox will be lost.
            </p>
            <div class="flex gap-2 justify-end">
              <button class="px-4 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                onClick={() => setShowDeleteConfirm(false)}>Cancel</button>
              <button class="px-4 py-2 bg-red-700 hover:bg-red-600 rounded text-sm text-white font-medium"
                onClick={doDelete}>Delete</button>
            </div>
          </div>
        </div>
      </Show>

      {/* Upgrade modal */}
      <Show when={showUpgrade()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-[420px] shadow-2xl">
            <h3 class="text-base font-bold mb-3">
              {upgrading() ? "Upgrading..." : "Upgrade OpenClaw"}
            </h3>

            <Show when={!upgrading()}>
              <div class="text-sm text-gray-300 mb-2">
                <span class="text-gray-400">Current:</span> {updateInfo()?.current}
              </div>
              <div class="text-sm text-gray-300 mb-4">
                <span class="text-gray-400">Latest:</span> <span class="text-green-400">{updateInfo()?.latest}</span>
                {updateInfo()?.security && <span class="text-red-400 ml-2">⚠ Security</span>}
              </div>
              <p class="text-xs text-gray-500 mb-4">
                The upgrade will stop the gateway, update OpenClaw, and restart.
              </p>
              <div class="flex gap-2 justify-end">
                <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => setShowUpgrade(false)}>Cancel</button>
                <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                  onClick={startUpgrade}>Upgrade Now</button>
              </div>
            </Show>

            <Show when={upgrading()}>
              <div class="w-full bg-gray-700 rounded-full h-2 mb-2">
                <div class="h-2 rounded-full bg-indigo-600 transition-all" style={{ width: `${upgradeProgress()}%` }} />
              </div>
              <p class="text-xs text-gray-400 mb-2">{upgradeMessage()}</p>
              {upgradeError() && (
                <div>
                  <p class="text-xs text-red-400 mb-3">{upgradeError()}</p>
                  <div class="flex gap-2 justify-end">
                    <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                      onClick={() => { setShowUpgrade(false); setUpgrading(false); }}>Close</button>
                    <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                      onClick={startUpgrade}>Retry</button>
                  </div>
                </div>
              )}
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
