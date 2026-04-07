import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Instance = { name: string; sandbox_type: string; version: string; gateway_port: number; ttyd_port: number };

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

  const [gatewayToken, setGatewayToken] = createSignal("");

  // Fetch gateway token when tab changes
  async function fetchToken() {
    const name = activeTab();
    try {
      const token = await invoke<string>("get_gateway_token", { name });
      setGatewayToken(token);
    } catch {
      setGatewayToken("");
    }
  }
  // Fetch on first render
  fetchToken();

  async function openInBrowser() {
    const name = activeTab();
    const inst = props.instances.find((i) => i.name === name);
    const port = inst?.gateway_port ?? 3000;
    const token = gatewayToken();
    const url = token
      ? `http://127.0.0.1:${port}/?token=${token}`
      : `http://127.0.0.1:${port}`;
    try {
      await invoke("open_url_in_browser", { url });
    } catch (e) {
      prompt("Could not open browser. Copy this URL:", url);
    }
  }

  async function startInstance() {
    try {
      await invoke("start_instance", { name: activeTab() });
    } catch (e) {
      console.error("Failed to start:", e);
    }
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
        {/* No instance */}
        <Show when={!activeInstance()}>
          <div class="text-gray-500">No instance selected</div>
        </Show>

        {/* Instance running */}
        <Show when={activeInstance() && isRunning()}>
          <div class="text-center max-w-lg">
            <div class="mb-6">
              <span class="text-5xl">🦞</span>
            </div>
            <h2 class="text-xl font-bold mb-2">OpenClaw is Running</h2>
            <p class="text-sm text-gray-400 mb-6">
              Gateway is active on port {activeInstance()?.gateway_port}.
              Click below to open the control panel in your browser.
            </p>

            <button
              class="px-6 py-3 bg-indigo-600 hover:bg-indigo-500 rounded-lg text-white font-medium transition-colors"
              onClick={openInBrowser}
            >
              Open OpenClaw Control Panel ↗
            </button>

            <div class="mt-8 bg-gray-900 rounded-lg p-4 text-left text-xs text-gray-500 max-w-xl w-full">
              <table class="w-full">
                <tbody>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Instance</td><td>{activeInstance()?.name}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Version</td><td>{activeInstance()?.version}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Sandbox</td><td>{activeInstance()?.sandbox_type}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Gateway</td><td class="font-mono break-all">http://127.0.0.1:{activeInstance()?.gateway_port}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Token</td><td class="font-mono text-gray-300 break-all">{gatewayToken() || "loading..."}</td></tr>
                  <tr><td class="text-gray-400 pr-4 py-0.5 whitespace-nowrap align-top">Status</td><td class="text-green-400">● running</td></tr>
                </tbody>
              </table>
            </div>
          </div>
        </Show>

        {/* Instance not running */}
        <Show when={activeInstance() && !isRunning()}>
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
              onClick={startInstance}
            >
              Start Instance
            </button>
            <div class="mt-6 bg-gray-900 rounded p-3 text-left text-xs text-gray-500">
              <div>Instance: {activeInstance()?.name}</div>
              <div>Type: {activeInstance()?.sandbox_type}</div>
              <div>Version: {activeInstance()?.version}</div>
              <div>Gateway: 127.0.0.1:{activeInstance()?.gateway_port}</div>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
}
