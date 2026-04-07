import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import SandboxTerminal from "../components/Terminal";

type Instance = { name: string; sandbox_type: string; version: string; gateway_port: number };
type Lang = "zh-CN" | "en";
type StatusDetail = { processes: string; resources: string; gateway_log: string };

const t: Record<Lang, Record<string, string>> = {
  "zh-CN": {
    home: "首页", instances: "实例", security: "安全状态",
    stop: "停止", start: "启动", restart: "重启", status: "状态",
    running: "运行中", stopped: "已停止", unreachable: "不可达",
    allUpToDate: "所有组件版本最新", noCve: "无已知 CVE",
    noInstances: "暂无实例", close: "关闭", refresh: "刷新",
    processes: "进程列表", resources: "资源使用", logs: "终端日志",
  },
  en: {
    home: "Home", instances: "Instances", security: "Security",
    stop: "Stop", start: "Start", restart: "Restart", status: "Status",
    running: "running", stopped: "stopped", unreachable: "unreachable",
    allUpToDate: "All components up to date", noCve: "No known CVEs",
    noInstances: "No instances configured", close: "Close", refresh: "Refresh",
    processes: "Processes", resources: "Resources", logs: "Logs",
  },
};

const healthColor: Record<string, string> = {
  running: "bg-green-500", stopped: "bg-gray-500", unreachable: "bg-red-500",
};

export default function Home(props: {
  instances: Instance[];
  healths: Record<string, string>;
  onHealthChange: () => void;
}) {
  const [lang, setLang] = createSignal<Lang>("zh-CN");
  const l = () => t[lang()];
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);

  // Status modal
  const [statusFor, setStatusFor] = createSignal<string | null>(null);
  const [statusTab, setStatusTab] = createSignal<"processes" | "resources" | "logs">("processes");
  const [statusData, setStatusData] = createSignal<StatusDetail | null>(null);
  const [statusLoading, setStatusLoading] = createSignal(false);

  // Terminal modal
  const [terminalFor, setTerminalFor] = createSignal<string | null>(null);

  async function openStatus(name: string) {
    setStatusFor(name);
    setStatusTab("processes");
    await refreshStatus(name);
  }

  function closeStatus() {
    setStatusFor(null);
  }

  async function refreshStatus(name: string) {
    setStatusLoading(true);
    try {
      const data = await invoke<StatusDetail>("get_instance_status_detail", { name });
      setStatusData(data);
    } catch (e) {
      setStatusData({ processes: `Error: ${e}`, resources: `Error: ${e}`, gateway_log: `Error: ${e}` });
    } finally {
      setStatusLoading(false);
    }
  }

  async function handleStop(name: string) {
    setActionLoading(`stop-${name}`);
    try { await invoke("stop_instance", { name }); props.onHealthChange(); }
    catch (e) { console.error(e); }
    finally { setActionLoading(null); }
  }
  async function handleStart(name: string) {
    setActionLoading(`start-${name}`);
    try { await invoke("start_instance", { name }); props.onHealthChange(); }
    catch (e) { console.error(e); }
    finally { setActionLoading(null); }
  }
  async function handleRestart(name: string) {
    setActionLoading(`restart-${name}`);
    try { await invoke("stop_instance", { name }); await invoke("start_instance", { name }); props.onHealthChange(); }
    catch (e) { console.error(e); }
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

      <section class="mb-6">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">{l().instances}</h2>
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
                      <span class="font-medium">{inst.name}</span>
                      <span class="text-xs text-gray-400">({inst.sandbox_type})</span>
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
                        {loading() ? "..." : l().stop}
                      </button>
                    ) : (
                      <button class="px-3 py-1 text-xs bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50"
                        disabled={loading()} onClick={() => handleStart(inst.name)}>
                        {loading() ? "..." : l().start}
                      </button>
                    )}
                    <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                      disabled={loading()} onClick={() => handleRestart(inst.name)}>
                      {loading() ? "..." : l().restart}
                    </button>
                    <button class="px-3 py-1 text-xs bg-gray-600 hover:bg-gray-500 rounded ml-auto"
                      onClick={() => openStatus(inst.name)}>
                      {l().status}
                    </button>
                    <button class="px-3 py-1 text-xs bg-gray-600 hover:bg-gray-500 rounded"
                      onClick={() => setTerminalFor(inst.name)}>
                      Terminal
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

      {/* Status Modal — no click-outside-close */}
      <Show when={statusFor()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl w-[700px] h-[70vh] flex flex-col shadow-2xl">
            {/* Header */}
            <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700 shrink-0">
              <span class="font-medium">{l().status}: {statusFor()}</span>
              <div class="flex gap-2">
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => refreshStatus(statusFor()!)}>{l().refresh}</button>
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => {
                    const content = statusTab() === "processes" ? statusData()?.processes
                      : statusTab() === "resources" ? statusData()?.resources
                      : statusData()?.gateway_log;
                    if (content) navigator.clipboard.writeText(content);
                  }}>Copy</button>
                <button class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium"
                  onClick={closeStatus}>✕ {l().close}</button>
              </div>
            </div>
            {/* Tabs */}
            <div class="flex border-b border-gray-700 px-2 shrink-0">
              {(["processes", "resources", "logs"] as const).map((tab) => (
                <button
                  class={`px-3 py-2 text-sm border-b-2 transition-colors ${
                    statusTab() === tab ? "border-indigo-500 text-white" : "border-transparent text-gray-400 hover:text-gray-200"
                  }`}
                  onClick={() => setStatusTab(tab)}
                >
                  {l()[tab]}
                </button>
              ))}
            </div>
            {/* Content — fixed height */}
            <div class="flex-1 overflow-y-auto p-4 min-h-0">
              <Show when={statusLoading()}>
                <div class="text-gray-400 text-sm">Loading...</div>
              </Show>
              <Show when={!statusLoading() && statusData()}>
                <pre class="font-mono text-xs text-gray-300 whitespace-pre-wrap select-text">
                  {statusTab() === "processes" && statusData()!.processes}
                  {statusTab() === "resources" && statusData()!.resources}
                  {statusTab() === "logs" && statusData()!.gateway_log}
                </pre>
              </Show>
            </div>
          </div>
        </div>
      </Show>

      {/* Terminal Modal */}
      {terminalFor() && (
        <SandboxTerminal
          instanceName={terminalFor()!}
          onClose={() => setTerminalFor(null)}
        />
      )}
    </div>
  );
}
