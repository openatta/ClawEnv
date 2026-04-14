import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { Instance, ClawType } from "../types";

function ClawTypePicker(props: { clawTypes: ClawType[]; onSelect: (id: string) => void; onClose: () => void }) {
  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={props.onClose}>
      <div class="bg-gray-800 rounded-xl p-5 w-80 max-h-[80vh] overflow-y-auto shadow-xl" onClick={(e: MouseEvent) => e.stopPropagation()}>
        <h3 class="text-base font-semibold text-white mb-1">New Instance</h3>
        <p class="text-xs text-gray-400 mb-4">Choose a claw type to install</p>
        <div class="space-y-1">
          <For each={props.clawTypes}>
            {(claw) => (
              <button
                class="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg hover:bg-gray-700 transition-colors text-left"
                onClick={() => props.onSelect(claw.id)}
              >
                <span class="text-lg">{claw.logo || "📦"}</span>
                <div class="flex-1">
                  <div class="text-sm text-white">{claw.display_name}</div>
                  <div class="text-[10px] text-gray-500">{claw.npm_package}</div>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
type Lang = "zh-CN" | "en";
type StatusDetail = { gateway_log: string };

const t: Record<Lang, Record<string, string>> = {
  "zh-CN": {
    home: "首页", instances: "实例", security: "安全状态",
    stop: "停止", start: "启动", restart: "重启", status: "状态",
    running: "运行中", stopped: "已停止", unreachable: "不可达",
    allUpToDate: "所有组件版本最新", noCve: "无已知 CVE",
    noInstances: "暂无实例", close: "关闭", refresh: "刷新",
    logs: "终端日志",
  },
  en: {
    home: "Home", instances: "Instances", security: "Security",
    stop: "Stop", start: "Start", restart: "Restart", status: "Status",
    running: "running", stopped: "stopped", unreachable: "unreachable",
    allUpToDate: "All components up to date", noCve: "No known CVEs",
    noInstances: "No instances configured", close: "Close", refresh: "Refresh",
    logs: "Logs",
  },
};

const healthColor: Record<string, string> = {
  running: "bg-green-500", stopped: "bg-gray-500", unreachable: "bg-red-500",
};

export default function Home(props: {
  instances: Instance[];
  healths: Record<string, string>;
  onHealthChange: () => void;
  clawTypes?: ClawType[];
  onAddInstance?: (clawType?: string) => void;
}) {
  const [lang, setLang] = createSignal<Lang>("zh-CN");
  const l = () => t[lang()];
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal("");
  const [showClawPicker, setShowClawPicker] = createSignal(false);

  // Status modal (gateway log only)
  const [statusFor, setStatusFor] = createSignal<string | null>(null);
  const [statusData, setStatusData] = createSignal<StatusDetail | null>(null);
  const [statusLoading, setStatusLoading] = createSignal(false);

  async function openStatus(name: string) {
    setStatusFor(name);
    await refreshStatus(name);
  }

  function closeStatus() {
    setStatusFor(null);
  }

  async function refreshStatus(name: string) {
    setStatusLoading(true);
    try {
      const logs = await invoke<string>("get_instance_logs", { name });
      setStatusData({ gateway_log: logs });
    } catch (e) {
      setStatusData({ gateway_log: `Error: ${e}` });
    } finally {
      setStatusLoading(false);
    }
  }

  async function handleStop(name: string) {
    setActionLoading(`stop-${name}`); setActionError("");
    try { await invoke("stop_instance", { name }); props.onHealthChange(); }
    catch (e) { setActionError(`Stop failed: ${e}`); }
    finally { setActionLoading(null); }
  }
  async function handleStart(name: string) {
    setActionLoading(`start-${name}`); setActionError("");
    try { await invoke("start_instance", { name }); props.onHealthChange(); }
    catch (e) { setActionError(`Start failed: ${e}`); }
    finally { setActionLoading(null); }
  }
  async function handleRestart(name: string) {
    setActionLoading(`restart-${name}`); setActionError("");
    try { await invoke("stop_instance", { name }); await invoke("start_instance", { name }); props.onHealthChange(); }
    catch (e) { setActionError(`Restart failed: ${e}`); }
    finally { setActionLoading(null); }
  }

  const getHealth = (name: string) => props.healths[name] || "unreachable";

  return (
    <div class="h-full overflow-y-auto p-6 relative">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-xl font-bold">{l().home}</h1>
        <div class="flex gap-1">
          <button class={`px-2 py-0.5 text-xs rounded ${lang() === "zh-CN" ? "bg-indigo-600" : "bg-gray-700 hover:bg-gray-600"}`}
            onClick={() => setLang("zh-CN")}>中文</button>
          <button class={`px-2 py-0.5 text-xs rounded ${lang() === "en" ? "bg-indigo-600" : "bg-gray-700 hover:bg-gray-600"}`}
            onClick={() => setLang("en")}>EN</button>
        </div>
      </div>

      <Show when={actionError()}>
        <div class="mb-4 p-3 bg-red-900/30 border border-red-700 rounded text-sm text-red-400">{actionError()}</div>
      </Show>

      <section class="mb-6">
        <div class="flex items-center justify-between mb-3">
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide">{l().instances}</h2>
          <Show when={props.onAddInstance && props.clawTypes && props.clawTypes.length > 0}>
            <button
              class="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 rounded text-white"
              onClick={() => setShowClawPicker(true)}
            >+ Add</button>
          </Show>
        </div>
        <div class="space-y-3">
          <For each={props.instances}>
            {(inst) => {
              const health = () => getHealth(inst.name);
              const isRunning = () => health() === "running";
              const loading = () => actionLoading()?.includes(inst.name);
              return (
                <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
                  <div class="flex items-center justify-between mb-2">
                    <div class="flex items-center gap-2">
                      <div class={`w-2 h-2 rounded-full ${healthColor[health()] || "bg-gray-500"}`} />
                      <span class="text-sm">{inst.logo}</span>
                      <span class="font-medium">{inst.name}</span>
                      <span class="text-xs text-gray-400">{inst.display_name}</span>
                      <span class="text-xs text-gray-500">
                        {health() === "running" ? l().running : health() === "stopped" ? l().stopped : l().unreachable}
                      </span>
                    </div>
                    <span class="text-sm text-gray-400">v{inst.version}</span>
                  </div>
                  <div class="text-xs text-gray-500">Gateway: 127.0.0.1:{inst.gateway_port}</div>
                  <div class="flex gap-2 mt-3">
                    {isRunning() ? (
                      <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                        disabled={loading()} onClick={() => handleStop(inst.name)}>
                        {actionLoading() === `stop-${inst.name}` ? `${l().stop}...` : l().stop}
                      </button>
                    ) : (
                      <button class="px-3 py-1 text-xs bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50"
                        disabled={loading()} onClick={() => handleStart(inst.name)}>
                        {actionLoading() === `start-${inst.name}` ? `${l().start}...` : l().start}
                      </button>
                    )}
                    <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                      disabled={loading()} onClick={() => handleRestart(inst.name)}>
                      {actionLoading() === `restart-${inst.name}` ? `${l().restart}...` : l().restart}
                    </button>
                    <button class="px-3 py-1 text-xs bg-gray-600 hover:bg-gray-500 rounded ml-auto"
                      onClick={() => openStatus(inst.name)}>
                      {l().status}
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
          {props.instances.length === 0 && (
            <div class="text-gray-500 text-sm">{l().noInstances}</div>
          )}
        </div>
      </section>

      <section class="mb-6">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">{l().security}</h2>
        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span><span>{l().allUpToDate}</span>
          </div>
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span><span>{l().noCve}</span>
          </div>
        </div>
      </section>

      {/* Log Modal */}
      <Show when={statusFor()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl w-[700px] h-[70vh] flex flex-col shadow-2xl">
            <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700 shrink-0">
              <span class="font-medium">{l().logs}: {statusFor()}</span>
              <div class="flex gap-2">
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => refreshStatus(statusFor()!)}>{l().refresh}</button>
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => {
                    const content = statusData()?.gateway_log;
                    if (content) navigator.clipboard.writeText(content);
                  }}>Copy</button>
                <button class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium"
                  onClick={closeStatus}>✕ {l().close}</button>
              </div>
            </div>
            <div class="flex-1 overflow-y-auto p-4 min-h-0">
              <Show when={statusLoading()}>
                <div class="text-gray-400 text-sm">Loading...</div>
              </Show>
              <Show when={!statusLoading() && statusData()}>
                <pre class="font-mono text-xs text-gray-300 whitespace-pre-wrap select-text">
                  {statusData()!.gateway_log}
                </pre>
              </Show>
            </div>
          </div>
        </div>
      </Show>

      {/* Claw type picker dialog */}
      <Show when={showClawPicker()}>
        <ClawTypePicker
          clawTypes={props.clawTypes || []}
          onSelect={(id) => { setShowClawPicker(false); props.onAddInstance?.(id); }}
          onClose={() => setShowClawPicker(false)}
        />
      </Show>
    </div>
  );
}
