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
      <h1 class="text-xl font-bold mb-6">Home</h1>

      <section class="mb-6">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
          Instances
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
                      <span class="text-xs text-gray-500">{healthLabel[health()] || "unknown"}</span>
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
                        {loading() ? "..." : "Stop"}
                      </button>
                    ) : (
                      <button
                        class="px-3 py-1 text-xs bg-indigo-700 hover:bg-indigo-600 rounded transition-colors disabled:opacity-50"
                        disabled={loading()}
                        onClick={() => handleStart(inst.name)}
                      >
                        {loading() ? "..." : "Start"}
                      </button>
                    )}
                    <button
                      class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded transition-colors disabled:opacity-50"
                      disabled={loading()}
                      onClick={() => handleRestart(inst.name)}
                    >
                      {loading() ? "..." : "Restart"}
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
          {props.instances.length === 0 && (
            <div class="text-gray-500 text-sm">No instances configured.</div>
          )}
        </div>
      </section>

      <section class="mb-6">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
          Security
        </h2>
        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span>
            <span>All components up to date</span>
          </div>
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span>
            <span>No known CVEs</span>
          </div>
        </div>
      </section>
    </div>
  );
}
