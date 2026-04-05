import { createSignal, For, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
};

export default function OpenClawPage(props: { instances: Instance[] }) {
  const [activeTab, setActiveTab] = createSignal(
    props.instances[0]?.name ?? ""
  );
  const [healths, setHealths] = createSignal<Record<string, string>>({});

  const activeInstance = () =>
    props.instances.find((i) => i.name === activeTab());

  const isRunning = () => {
    const h = healths()[activeTab()] ?? "unknown";
    return h.toLowerCase().includes("running") || h.toLowerCase().includes("healthy");
  };

  const openInBrowser = () => {
    const inst = activeInstance();
    if (inst) {
      window.open(`http://127.0.0.1:${inst.gateway_port}`, "_blank");
    }
  };

  /** Fetch health for all instances */
  async function refreshHealths() {
    const result: Record<string, string> = {};
    for (const inst of props.instances) {
      try {
        const h = await invoke<string>("get_instance_health", { name: inst.name });
        result[inst.name] = h;
      } catch {
        result[inst.name] = "unknown";
      }
    }
    setHealths(result);
  }

  onMount(() => {
    refreshHealths();
  });

  const interval = setInterval(refreshHealths, 5000);
  onCleanup(() => clearInterval(interval));

  /** Map health string to dot color class */
  function healthColor(name: string): string {
    const h = healths()[name] ?? "unknown";
    if (h.toLowerCase().includes("healthy") || h.toLowerCase().includes("running")) {
      return "bg-green-500";
    }
    if (h.toLowerCase().includes("degraded") || h.toLowerCase().includes("starting")) {
      return "bg-yellow-500";
    }
    if (h.toLowerCase().includes("unhealthy") || h.toLowerCase().includes("stopped") || h.toLowerCase().includes("error")) {
      return "bg-red-500";
    }
    return "bg-gray-500";
  }

  /** Overall status: worst of all instances */
  function overallStatus(): { color: string; label: string } {
    const vals = Object.values(healths());
    if (vals.length === 0) return { color: "bg-gray-500", label: "Unknown" };
    const hasError = vals.some(
      (v) => v.toLowerCase().includes("unhealthy") || v.toLowerCase().includes("stopped") || v.toLowerCase().includes("error")
    );
    if (hasError) return { color: "bg-red-500", label: "Error" };
    const hasDegraded = vals.some(
      (v) => v.toLowerCase().includes("degraded") || v.toLowerCase().includes("starting")
    );
    if (hasDegraded) return { color: "bg-yellow-500", label: "Degraded" };
    const allHealthy = vals.every(
      (v) => v.toLowerCase().includes("healthy") || v.toLowerCase().includes("running")
    );
    if (allHealthy) return { color: "bg-green-500", label: "Running" };
    return { color: "bg-gray-500", label: "Unknown" };
  }

  return (
    <div class="h-full flex flex-col">
      {/* Top bar */}
      <div class="flex items-center justify-between px-4 py-2 border-b border-gray-800 shrink-0">
        <span class="font-medium">OpenClaw</span>
        <div class="flex items-center gap-2">
          <div class={`w-2 h-2 rounded-full ${overallStatus().color}`} />
          <span class="text-sm text-gray-400">{overallStatus().label}</span>
          <button
            class="ml-2 px-2 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={openInBrowser}
            title="Open in browser"
          >
            &#8599;
          </button>
        </div>
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
              onClick={() => setActiveTab(inst.name)}
            >
              <span class="flex items-center gap-1.5">
                <span
                  class={`w-1.5 h-1.5 rounded-full ${healthColor(inst.name)}`}
                />
                {inst.name}
                <span class="text-xs text-gray-500">
                  ({inst.sandbox_type.toLowerCase()})
                </span>
              </span>
            </button>
          )}
        </For>
      </div>

      {/* WebView area */}
      <div class="flex-1 bg-gray-950">
        {activeInstance() ? (
          isRunning() ? (
            <iframe
              src={`http://127.0.0.1:${activeInstance()!.gateway_port}`}
              class="w-full h-full border-0"
              title="OpenClaw Web UI"
            />
          ) : (
            <div class="flex flex-col items-center justify-center h-full text-gray-400">
              <p class="text-lg mb-4">OpenClaw is not running</p>
              <button
                class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white"
                onClick={async () => {
                  try {
                    await invoke("start_instance", { name: activeTab() });
                    refreshHealths();
                  } catch (e) {
                    console.error("Failed to start:", e);
                  }
                }}
              >
                Start Instance
              </button>
            </div>
          )
        ) : (
          <div class="flex items-center justify-center h-full text-gray-500">
            No instance selected
          </div>
        )}
      </div>

      {/* Status bar */}
      <div class="px-4 py-1.5 text-xs text-gray-500 border-t border-gray-800 shrink-0">
        {activeInstance()
          ? `Connected | Gateway: 127.0.0.1:${activeInstance()!.gateway_port}`
          : "Disconnected"}
      </div>
    </div>
  );
}
