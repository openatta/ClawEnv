import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Instance = { name: string; sandbox_type: string; version: string; gateway_port: number };

export default function OpenClawPage(props: {
  instances: Instance[];
  healths: Record<string, string>;
}) {
  const [activeTab, setActiveTab] = createSignal(props.instances[0]?.name ?? "");

  const activeInstance = () => props.instances.find((i) => i.name === activeTab());
  const activeHealth = () => props.healths[activeTab()] || "Unreachable";
  const isRunning = () => activeHealth() === "Running";

  const openInBrowser = () => {
    const inst = activeInstance();
    if (inst) window.open(`http://127.0.0.1:${inst.gateway_port}`, "_blank");
  };

  function healthColor(name: string): string {
    const h = props.healths[name] ?? "unknown";
    if (h === "Running") return "bg-green-500";
    if (h === "Stopped") return "bg-gray-500";
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
            <div class="w-full h-full flex flex-col">
              <iframe
                src={`http://127.0.0.1:${activeInstance()!.gateway_port}`}
                class="w-full flex-1 border-0"
                title="OpenClaw Web UI"
              />
              <div class="px-3 py-1 text-xs text-gray-500 border-t border-gray-800 shrink-0">
                Gateway: 127.0.0.1:{activeInstance()!.gateway_port}
                {" | "}
                Note: If blank, ensure Lima port forwarding is configured for VM access.
              </div>
            </div>
          ) : (
            <div class="text-center text-gray-400 max-w-md">
              <div class="mb-4 opacity-30">
                <svg viewBox="0 0 24 24" class="w-16 h-16 mx-auto">
                  <path d="M12 2C8 2 5 4.5 5 8c0 2 1 3.5 2.5 4.5L6 17c-.5 1.5.5 3 2 3h8c1.5 0 2.5-1.5 2-3l-1.5-4.5C18 11.5 19 10 19 8c0-3.5-3-6-7-6z" fill="#ef4444" opacity="0.3" />
                </svg>
              </div>
              <p class="text-lg mb-2">OpenClaw is not running</p>
              <p class="text-sm text-gray-500 mb-4">
                Instance "{activeTab()}" is {activeHealth().toLowerCase()}.
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
              <div class="mt-6 text-left bg-gray-900 rounded p-3 text-xs text-gray-500">
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
