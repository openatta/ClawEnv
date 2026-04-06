import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Instance = { name: string; sandbox_type: string; version: string; gateway_port: number };

export default function OpenClawPage(props: {
  instances: Instance[];
  healths: Record<string, string>;
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

  return (
    <div class="h-full flex flex-col">
      {/* Top bar */}
      <div class="flex items-center justify-between px-4 py-2 border-b border-gray-800 shrink-0">
        <span class="font-medium">OpenClaw</span>
        <div class="flex items-center gap-2">
          <div class={`w-2 h-2 rounded-full ${healthColor(activeTab())}`} />
          <span class="text-sm text-gray-400">{activeHealth()}</span>
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
                <span class={`w-1.5 h-1.5 rounded-full ${healthColor(inst.name)}`} />
                {inst.name}
                <span class="text-xs text-gray-500">({inst.sandbox_type.toLowerCase()})</span>
              </span>
            </button>
          )}
        </For>
      </div>

      {/* Content area */}
      <div class="flex-1 flex items-center justify-center bg-gray-950">
        <Show when={activeInstance()} fallback={
          <div class="text-gray-500">No instance selected</div>
        }>
          {isRunning() ? (
            <div class="text-center max-w-lg">
              {/* Running state — show open button */}
              <div class="mb-6">
                <span class="text-5xl">🦞</span>
              </div>
              <h2 class="text-xl font-bold mb-2">OpenClaw is Running</h2>
              <p class="text-sm text-gray-400 mb-6">
                Gateway is active on port {activeInstance()!.gateway_port}.
                OpenClaw's web UI blocks iframe embedding (X-Frame-Options: DENY),
                so it must be opened in an external browser.
              </p>

              <button
                class="px-6 py-3 bg-indigo-600 hover:bg-indigo-500 rounded-lg text-white font-medium transition-colors"
                onClick={async () => {
                  const url = `http://127.0.0.1:${activeInstance()!.gateway_port}`;
                  try {
                    await invoke("open_url_in_browser", { url });
                  } catch (e) {
                    alert(`Please open manually: ${url}`);
                  }
                }}
              >
                Open OpenClaw Control Panel ↗
              </button>

              <div class="mt-8 bg-gray-900 rounded-lg p-4 text-left text-xs text-gray-500">
                <div class="grid grid-cols-2 gap-y-1.5">
                  <span class="text-gray-400">Instance</span>
                  <span>{activeInstance()!.name}</span>
                  <span class="text-gray-400">Version</span>
                  <span>{activeInstance()!.version}</span>
                  <span class="text-gray-400">Sandbox</span>
                  <span>{activeInstance()!.sandbox_type}</span>
                  <span class="text-gray-400">Gateway</span>
                  <span class="font-mono">http://127.0.0.1:{activeInstance()!.gateway_port}</span>
                  <span class="text-gray-400">Status</span>
                  <span class="text-green-400">● {activeHealth()}</span>
                </div>
              </div>
            </div>
          ) : (
            <div class="text-center text-gray-400 max-w-md">
              <div class="mb-4 opacity-30">
                <span class="text-6xl">🦞</span>
              </div>
              <p class="text-lg mb-2">OpenClaw is not running</p>
              <p class="text-sm text-gray-500 mb-4">
                Instance "{activeTab()}" is {activeHealth()}.
              </p>
              <button
                class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-white text-sm"
                onClick={async () => {
                  try { await invoke("start_instance", { name: activeTab() }); }
                  catch (e) { console.error("Failed to start:", e); }
                }}
              >
                Start Instance
              </button>
              <div class="mt-6 bg-gray-900 rounded p-3 text-left text-xs text-gray-500">
                <div>Instance: {activeInstance()!.name}</div>
                <div>Type: {activeInstance()!.sandbox_type}</div>
                <div>Version: {activeInstance()!.version}</div>
                <div>Gateway: 127.0.0.1:{activeInstance()!.gateway_port}</div>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
