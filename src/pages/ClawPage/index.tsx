import { createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Instance, ClawType, UpgradeInfo } from "../../types";
import ExportProgress from "../../components/ExportProgress";
import DeleteProgress from "../../components/DeleteProgress";
import ConfigModal from "./ConfigModal";
import UpgradeModal from "./UpgradeModal";
import { t } from "../../i18n";

export default function ClawPage(props: {
  clawType: ClawType;
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

  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal("");
  const [showDeleteConfirm, setShowDeleteConfirm] = createSignal(false);
  const [gatewayToken, setGatewayToken] = createSignal("");

  // Export state
  const [showExport, setShowExport] = createSignal(false);

  async function doExportBundle() {
    try {
      await invoke("export_native_bundle", { name: activeTab() });
      setShowExport(true);
    } catch { /* cancelled */ }
  }

  // Upgrade state
  const [updateInfo, setUpdateInfo] = createSignal<UpgradeInfo | null>(null);
  const [showUpgrade, setShowUpgrade] = createSignal(false);

  onMount(async () => {
    const unUpdate = await listen<UpgradeInfo>("update-available", (ev) => {
      if (ev.payload.instance === activeTab()) {
        setUpdateInfo(ev.payload);
      }
    });
    onCleanup(() => { unUpdate(); });
  });

  async function checkUpdate() {
    const name = activeTab();
    if (!name) return;
    try {
      const info = await invoke<{ current: string; latest: string; has_upgrade: boolean; is_security_release: boolean }>(
        "check_instance_update", { name }
      );
      if (info.has_upgrade) {
        setUpdateInfo({ instance: name, current: info.current, latest: info.latest, security: info.is_security_release });
      }
    } catch { /* ignore */ }
  }

  async function fetchToken(name?: string) {
    const n = name || activeTab();
    try {
      const token = await invoke<string>("get_gateway_token", { name: n });
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

  // Delete with progress dialog
  const [deletingInstance, setDeletingInstance] = createSignal<string | null>(null);

  function doDelete() {
    setShowDeleteConfirm(false);
    setDeletingInstance(activeTab());
  }

  function onDeleteComplete() {
    const deleted = deletingInstance();
    setDeletingInstance(null);
    if (deleted) {
      const remaining = props.instances.filter(i => i.name !== deleted);
      setActiveTab(remaining[0]?.name ?? "");
    }
    props.onInstancesChanged?.();
  }

  async function openInBrowser() {
    const inst = activeInstance();
    const port = inst?.gateway_port ?? props.clawType.default_port;
    await fetchToken();
    const token = gatewayToken();
    const url = token ? `http://127.0.0.1:${port}/?token=${token}` : `http://127.0.0.1:${port}`;
    try { await invoke("open_url_in_browser", { url }); }
    catch { prompt("Copy this URL:", url); }
  }

  const loading = (action: string) => actionLoading() === `${action}:${activeTab()}`;
  const anyLoading = () => actionLoading()?.endsWith(`:${activeTab()}`) ?? false;

  // Config panel
  const [showConfig, setShowConfig] = createSignal(false);

  function openConfig() {
    const inst = activeInstance();
    if (!inst) return;
    setShowConfig(true);
  }

  function onConfigSave() {
    setShowConfig(false);
    props.onInstancesChanged?.();
  }

  function onUpgradeComplete() {
    setShowUpgrade(false);
    setUpdateInfo(null);
    props.onInstancesChanged?.();
  }

  // Shorthand for display name and logo
  const dn = () => props.clawType.display_name;
  const logo = () => props.clawType.logo;

  return (
    <div class="h-full flex flex-col">
      {/* Top bar */}
      <div class="flex items-center justify-between px-4 py-2 border-b border-gray-800 shrink-0">
        <span class="font-medium flex items-center gap-2">
          <span class="text-lg">{logo()}</span> {dn()}
        </span>
        <button
          class="w-6 h-6 flex items-center justify-center rounded bg-gray-700 hover:bg-indigo-600 text-sm font-bold"
          title={`New ${dn()} Instance`}
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

      {/* Content */}
      <div class="flex-1 flex items-center justify-center bg-gray-950">
        <Show when={!activeInstance()}>
          <div class="text-center text-gray-500">
            <p class="mb-4">No {dn()} instances yet</p>
            <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
              onClick={() => props.onAddInstance?.()}>Create Instance</button>
          </div>
        </Show>

        <Show when={activeInstance()}>
          <div class="text-center max-w-lg w-full">
            <div class="mb-4">
              <span class={`text-5xl ${isRunning() ? "" : "opacity-30"}`}>{logo()}</span>
            </div>
            <h2 class="text-xl font-bold mb-1">
              {isRunning() ? `${dn()} is Running` : `${dn()} is Stopped`}
            </h2>
            <p class="text-sm text-gray-400 mb-5">
              {isRunning()
                ? `Gateway active on port ${activeInstance()?.gateway_port}`
                : `Instance "${activeTab()}" is ${activeHealth()}`}
            </p>

            {/* Action buttons -- two rows */}
            <div class="flex items-center justify-center gap-2 mb-1">
              <Show when={!isRunning() && !anyLoading()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                  onClick={() => doAction("start")}>{t("启动", "Start")}</button>
              </Show>
              <Show when={isRunning() && !anyLoading()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                  onClick={openInBrowser}>{t("打开控制面板", "Open Control Panel")}</button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                  onClick={() => doAction("stop")}>{t("停止", "Stop")}</button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                  onClick={() => doAction("restart")}>{t("重启", "Restart")}</button>
              </Show>
            </div>
            <div class="flex items-center justify-center gap-2 mb-2">
              <button class="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 rounded text-xs"
                onClick={openConfig}>{t("配置", "Configure")}</button>
              <Show when={activeInstance()?.sandbox_type?.toLowerCase() === "native"}>
                <button class="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 rounded text-xs"
                  onClick={doExportBundle}>{t("导出 Bundle", "Export Bundle")}</button>
              </Show>
              <button class="px-3 py-1.5 bg-red-900/60 hover:bg-red-800 text-red-300 rounded text-xs disabled:opacity-50"
                disabled={anyLoading()} onClick={() => setShowDeleteConfirm(true)}>
                {loading("delete") ? t("删除中...", "Deleting...") : t("删除", "Delete")}
              </button>
            </div>

            <Show when={anyLoading()}>
              <div class="flex items-center justify-center gap-2 mb-2 text-sm text-indigo-300">
                <span class="animate-pulse">...</span>
                {loading("start") && "Starting instance..."}
                {loading("stop") && "Stopping instance..."}
                {loading("restart") && "Restarting instance..."}
                {loading("delete") && "Deleting instance..."}
              </div>
            </Show>

            {actionError() && <p class="text-xs text-red-400 mb-3">{actionError()}</p>}

            {/* Upgrade banner */}
            <Show when={updateInfo() && updateInfo()!.instance === activeTab()}>
              {(() => {
                const info = updateInfo()!;
                const isBeta = /[-](beta|alpha|rc|pre|dev)/i.test(info.latest)
                  || /[-](beta|alpha|rc|pre|dev)/i.test(info.current);
                return (
                  <div class={`rounded-lg p-3 mb-3 text-sm flex items-center justify-between ${
                    info.security ? "bg-red-900/30 border border-red-700" : "bg-indigo-900/30 border border-indigo-700"
                  }`}>
                    <div>
                      <span class={info.security ? "text-red-300" : "text-indigo-300"}>
                        {info.security ? "Security update" : "Update available"}:
                      </span>
                      <span class="text-gray-300 ml-1">
                        {info.current} → {info.latest}
                      </span>
                      {isBeta && <span class="text-yellow-400 ml-2 text-xs font-medium">(beta)</span>}
                    </div>
                    <button class="px-3 py-1 bg-indigo-600 hover:bg-indigo-500 rounded text-xs text-white"
                      onClick={() => setShowUpgrade(true)}>Upgrade</button>
                  </div>
                );
              })()}
            </Show>

            {/* Info table */}
            <div class="bg-gray-900 rounded-lg p-4 text-left text-xs text-gray-500 mx-auto max-w-xl">
              <table class="w-full"><tbody>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Instance</td><td>{activeInstance()?.name}</td></tr>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Type</td><td>{dn()}</td></tr>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Version</td><td>{activeInstance()?.version}</td></tr>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Sandbox</td><td>{activeInstance()?.sandbox_type}</td></tr>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Gateway</td><td class="font-mono">http://127.0.0.1:{activeInstance()?.gateway_port}</td></tr>
                <Show when={isRunning()}>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Token</td><td class="font-mono text-gray-300 break-all">{gatewayToken() || "..."}</td></tr>
                </Show>
                <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap">Status</td>
                  <td class={isRunning() ? "text-green-400" : "text-gray-500"}>
                    {isRunning() ? "running" : activeHealth()}
                  </td>
                </tr>
              </tbody></table>
            </div>
          </div>
        </Show>
      </div>

      {/* Config dialog */}
      <Show when={showConfig() && activeInstance()}>
        <ConfigModal
          instanceName={activeTab()}
          instances={props.instances}
          clawType={props.clawType}
          sandboxType={activeInstance()!.sandbox_type}
          gatewayPort={activeInstance()!.gateway_port}
          ttydPort={activeInstance()!.ttyd_port}
          onSave={onConfigSave}
          onClose={() => setShowConfig(false)}
        />
      </Show>

      {/* Delete confirmation */}
      <Show when={showDeleteConfirm()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-6 max-w-sm shadow-2xl">
            <h3 class="text-lg font-bold text-red-400 mb-2">Delete Instance</h3>
            <p class="text-sm text-gray-300 mb-1">
              Are you sure you want to delete <strong>"{activeTab()}"</strong>?
            </p>
            <p class="text-xs text-gray-500 mb-4">
              This will stop the instance and destroy the sandbox. All data will be lost.
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
      <Show when={showUpgrade() && updateInfo()}>
        <UpgradeModal
          clawDisplayName={dn()}
          instanceName={activeTab()}
          updateInfo={updateInfo()!}
          onClose={() => setShowUpgrade(false)}
          onComplete={onUpgradeComplete}
        />
      </Show>

      {/* Export progress */}
      <Show when={showExport()}>
        <ExportProgress isNative={true} onClose={() => setShowExport(false)} />
      </Show>

      {/* Delete progress */}
      <Show when={deletingInstance()}>
        <DeleteProgress
          instanceName={deletingInstance()!}
          onComplete={onDeleteComplete}
          onError={() => setDeletingInstance(null)}
        />
      </Show>
    </div>
  );
}
