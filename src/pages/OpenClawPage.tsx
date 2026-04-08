import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Instance = { name: string; sandbox_type: string; version: string; gateway_port: number; ttyd_port: number };

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

  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal("");
  const [showDeleteConfirm, setShowDeleteConfirm] = createSignal(false);
  const [gatewayToken, setGatewayToken] = createSignal("");

  async function fetchToken() {
    try {
      const token = await invoke<string>("get_gateway_token", { name: activeTab() });
      setGatewayToken(token);
    } catch { setGatewayToken(""); }
  }
  fetchToken();

  async function doAction(action: string) {
    setActionLoading(action);
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
    setActionLoading("delete");
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

  const loading = (action: string) => actionLoading() === action;

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
            <div class="flex items-center justify-center gap-2 mb-4">
              <Show when={!isRunning()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm disabled:opacity-50"
                  disabled={!!actionLoading()} onClick={() => doAction("start")}>
                  {loading("start") ? "Starting..." : "▶ Start"}
                </button>
              </Show>
              <Show when={isRunning()}>
                <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                  onClick={openInBrowser}>
                  Open Control Panel ↗
                </button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm disabled:opacity-50"
                  disabled={!!actionLoading()} onClick={() => doAction("stop")}>
                  {loading("stop") ? "..." : "⏹ Stop"}
                </button>
                <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm disabled:opacity-50"
                  disabled={!!actionLoading()} onClick={() => doAction("restart")}>
                  {loading("restart") ? "..." : "↻ Restart"}
                </button>
              </Show>
              <button class="px-3 py-2 bg-red-900/60 hover:bg-red-800 text-red-300 rounded text-sm disabled:opacity-50 ml-2"
                disabled={!!actionLoading()} onClick={() => setShowDeleteConfirm(true)}>
                {loading("delete") ? "Deleting..." : "Delete"}
              </button>
            </div>

            {actionError() && <p class="text-xs text-red-400 mb-3">{actionError()}</p>}

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
    </div>
  );
}
