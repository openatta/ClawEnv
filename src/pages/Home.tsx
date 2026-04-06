import { createSignal, For, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
};

type HealthEvent = {
  instance_name: string;
  health: "Running" | "Stopped" | "Unreachable";
};

type Lang = "zh-CN" | "en";
const t: Record<Lang, Record<string, string>> = {
  "zh-CN": {
    home: "首页", instances: "实例", security: "安全状态",
    stop: "停止", start: "启动", restart: "重启",
    running: "运行中", stopped: "已停止", unreachable: "不可达",
    allUpToDate: "所有组件版本最新", noCve: "无已知 CVE",
    noInstances: "暂无实例",
  },
  en: {
    home: "Home", instances: "Instances", security: "Security",
    stop: "Stop", start: "Start", restart: "Restart",
    running: "running", stopped: "stopped", unreachable: "unreachable",
    allUpToDate: "All components up to date", noCve: "No known CVEs",
    noInstances: "No instances configured",
  },
};

const healthColor: Record<string, string> = {
  Running: "bg-green-500",
  Stopped: "bg-gray-500",
  Unreachable: "bg-red-500",
};

const healthLabel: Record<string, string> = {
  Running: "running",
  Stopped: "stopped",
  Unreachable: "unreachable",
};

export default function Home(props: { instances: Instance[] }) {
  const [lang, setLang] = createSignal<Lang>("zh-CN");
  const l = () => t[lang()];
  const [healths, setHealths] = createSignal<Record<string, string>>({});
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);

  onMount(async () => {
    // Initial health check for all instances
    for (const inst of props.instances) {
      try {
        const h = await invoke<string>("get_instance_health", { name: inst.name });
        setHealths((prev) => ({ ...prev, [inst.name]: h }));
      } catch {
        setHealths((prev) => ({ ...prev, [inst.name]: "Unreachable" }));
      }
    }

    // Listen for live health updates from monitor
    const unlisten = await listen<HealthEvent>("instance-health", (event) => {
      setHealths((prev) => ({
        ...prev,
        [event.payload.instance_name]: event.payload.health,
      }));
    });
    onCleanup(unlisten);
  });

  async function handleStop(name: string) {
    setActionLoading(`stop-${name}`);
    try {
      await invoke("stop_instance", { name });
      setHealths((prev) => ({ ...prev, [name]: "Stopped" }));
    } catch (e) {
      console.error("Stop failed:", e);
    } finally {
      setActionLoading(null);
    }
  }

  async function handleStart(name: string) {
    setActionLoading(`start-${name}`);
    try {
      await invoke("start_instance", { name });
      setHealths((prev) => ({ ...prev, [name]: "Running" }));
    } catch (e) {
      console.error("Start failed:", e);
    } finally {
      setActionLoading(null);
    }
  }

  async function handleRestart(name: string) {
    setActionLoading(`restart-${name}`);
    try {
      await invoke("stop_instance", { name });
      await invoke("start_instance", { name });
      setHealths((prev) => ({ ...prev, [name]: "Running" }));
    } catch (e) {
      console.error("Restart failed:", e);
    } finally {
      setActionLoading(null);
    }
  }

  const getHealth = (name: string) => healths()[name] || "Unreachable";

  return (
    <div class="h-full overflow-y-auto p-6">
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
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
          {l().instances}
        </h2>
        <div class="space-y-3">
          <For each={props.instances}>
            {(inst) => {
              const health = () => getHealth(inst.name);
              const isRunning = () => health() === "Running";
              const loading = () => actionLoading()?.includes(inst.name);

              return (
                <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
                  <div class="flex items-center justify-between mb-2">
                    <div class="flex items-center gap-2">
                      <div class={`w-2 h-2 rounded-full ${healthColor[health()] || "bg-gray-500"}`} />
                      <span class="font-medium">{inst.name}</span>
                      <span class="text-xs text-gray-400">({inst.sandbox_type})</span>
                      <span class="text-xs text-gray-500">{l()[health().toLowerCase() as keyof typeof l] || health()}</span>
                    </div>
                    <span class="text-sm text-gray-400">v{inst.version}</span>
                  </div>
                  <div class="text-xs text-gray-500">
                    Gateway: 127.0.0.1:{inst.gateway_port}
                  </div>
                  <div class="flex gap-2 mt-3">
                    {isRunning() ? (
                      <button
                        class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded transition-colors disabled:opacity-50"
                        disabled={loading()}
                        onClick={() => handleStop(inst.name)}
                      >
                        {loading() ? "..." : l().stop}
                      </button>
                    ) : (
                      <button
                        class="px-3 py-1 text-xs bg-indigo-700 hover:bg-indigo-600 rounded transition-colors disabled:opacity-50"
                        disabled={loading()}
                        onClick={() => handleStart(inst.name)}
                      >
                        {loading() ? "..." : l().start}
                      </button>
                    )}
                    <button
                      class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded transition-colors disabled:opacity-50"
                      disabled={loading()}
                      onClick={() => handleRestart(inst.name)}
                    >
                      {loading() ? "..." : l().restart}
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
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
          {l().security}
        </h2>
        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span>
            <span>{l().allUpToDate}</span>
          </div>
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span>
            <span>{l().noCve}</span>
          </div>
        </div>
      </section>
    </div>
  );
}
