import { createSignal, onMount, onCleanup, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import SandboxTerminal from "../components/Terminal";
import NoVncPanel from "../components/NoVncPanel";
import ExportProgress from "../components/ExportProgress";
import VmCard from "../components/VmCard";
import { t } from "../i18n";

type SandboxVm = {
  name: string;
  status: string;
  cpus: string;
  memory: string;
  disk: string;
  dir_size: string;
  managed: boolean;
  ttyd_port?: number;
};

export default function SandboxPage() {
  const [vms, setVms] = createSignal<SandboxVm[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal("");
  const [diskUsage, setDiskUsage] = createSignal("");
  const [terminalFor, setTerminalFor] = createSignal<{ name: string; ttydPort?: number } | null>(null);

  // Chromium install state
  const [chromiumFor, setChromiumFor] = createSignal<string | null>(null);
  const [chromiumInstalling, setChromiumInstalling] = createSignal(false);
  const [chromiumLogs, setChromiumLogs] = createSignal<string[]>([]);
  const [chromiumDone, setChromiumDone] = createSignal(false);
  let chromiumLogRef: HTMLDivElement | undefined;

  // HIL (Human-in-the-Loop) noVNC state
  const [hilInstance, setHilInstance] = createSignal<string | null>(null);
  const [hilUrl, setHilUrl] = createSignal("");
  const [browserLoading, setBrowserLoading] = createSignal("");

  // Listen for HIL events from backend health monitor AND bridge server
  onMount(async () => {
    const un1 = await listen<{ instance: string; novnc_url: string }>("hil-required", (ev) => {
      setHilInstance(ev.payload.instance);
      setHilUrl(ev.payload.novnc_url);
    });
    // Bridge-originated HIL: hil-skill called /api/hil/request
    const un2 = await listen<string>("hil-bridge-request", async (ev) => {
      try {
        const data = JSON.parse(ev.payload);
        // Auto-start interactive mode for the first managed sandbox instance
        const vmList = vms().filter(v => v.managed && v.status.toLowerCase().includes("running"));
        if (vmList.length > 0) {
          const name = vmList[0].name.replace(/^clawenv-/, "");
          await startBrowserInteractive(name);
        }
      } catch { /* ignore parse errors */ }
    });
    onCleanup(() => { un1(); un2(); });
  });

  async function startBrowserInteractive(instanceName: string) {
    setBrowserLoading(instanceName);
    try {
      const url = await invoke<string>("browser_start_interactive", { name: instanceName });
      setHilInstance(instanceName);
      setHilUrl(url);
    } catch (e) {
      console.error(`Failed to start browser: ${e}`);
    } finally {
      setBrowserLoading("");
    }
  }

  async function resumeHeadless() {
    const name = hilInstance();
    if (!name) return;
    try {
      await invoke("browser_resume_headless", { name });
      // Notify bridge to unblock the hil_request call
      try { await invoke("hil_complete"); } catch { /* bridge may not be running */ }
    } catch { /* ignore */ }
    setHilInstance(null);
    setHilUrl("");
  }

  async function handleChromium(instanceName: string) {
    setChromiumFor(instanceName);
    setChromiumInstalling(true);
    setChromiumDone(false);
    setChromiumLogs(["Starting Chromium installation..."]);

    const unlisten = await listen<string>("chromium-install-progress", (event) => {
      setChromiumLogs((l) => [...l, event.payload]);
      if (chromiumLogRef) setTimeout(() => { if (chromiumLogRef) chromiumLogRef.scrollTop = chromiumLogRef.scrollHeight; }, 10);
    });

    try {
      await invoke("install_chromium", { name: instanceName });
      setChromiumLogs((l) => [...l, "✓ Chromium installed successfully"]);
    } catch (e) {
      setChromiumLogs((l) => [...l, `✗ Error: ${e}`]);
    } finally {
      setChromiumInstalling(false);
      setChromiumDone(true);
      unlisten();
    }
  }

  // Export state
  const [showExport, setShowExport] = createSignal(false);

  async function handleExport(instanceName: string) {
    try {
      await invoke("export_sandbox", { name: instanceName });
      setShowExport(true);
    } catch { /* cancelled or error */ }
  }

  async function refresh() {
    setLoading(true);
    setError("");
    try {
      const result = await invoke<SandboxVm[]>("list_sandbox_vms");
      setVms(result);
      const usage = await invoke<string>("get_sandbox_disk_usage");
      setDiskUsage(usage);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  onMount(refresh);

  const managed = () => vms().filter((v) => v.managed);
  const external = () => vms().filter((v) => !v.managed);

  return (
    <div class="h-full overflow-y-auto p-6">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-xl font-bold">Sandbox Infrastructure</h1>
        <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded" onClick={refresh}>
          Refresh
        </button>
      </div>

      <Show when={error()}>
        <div class="mb-4 p-3 bg-red-900/30 border border-red-700 rounded text-sm text-red-400">{error()}</div>
      </Show>

      <Show when={loading()}>
        <div class="text-gray-400 text-sm">Loading sandbox status...</div>
      </Show>

      <Show when={!loading()}>
        <section class="mb-6">
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Virtual Machines</h2>
          <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm text-gray-300">
            <div class="flex justify-between">
              <span>VMs / Containers</span>
              <span>{vms().length} ({managed().length} managed, {external().length} external)</span>
            </div>
            <Show when={diskUsage()}>
              <div class="flex justify-between mt-1">
                <span>Total Disk Usage</span>
                <span class="text-gray-400">{diskUsage()}</span>
              </div>
            </Show>
          </div>
        </section>

        <section class="mb-6">
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
            Managed (ClawEnv)
          </h2>
          <div class="space-y-2">
            <For each={managed()} fallback={
              <div class="text-gray-500 text-sm">No managed sandbox instances</div>
            }>
              {(vm) => <VmCard vm={vm} onRefresh={refresh} onTerminal={setTerminalFor} onChromium={handleChromium} onBrowser={startBrowserInteractive} onExport={handleExport} browserLoading={browserLoading()} />}
            </For>
          </div>
        </section>

        <section class="mb-6">
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
            External
          </h2>
          <div class="space-y-2">
            <For each={external()} fallback={
              <div class="text-gray-500 text-sm">No external instances</div>
            }>
              {(vm) => <VmCard vm={vm} onRefresh={refresh} onTerminal={setTerminalFor} onChromium={handleChromium} onBrowser={startBrowserInteractive} onExport={handleExport} browserLoading={browserLoading()} />}
            </For>
          </div>
        </section>
      </Show>

      {/* Terminal modal */}
      {terminalFor() && (
        <SandboxTerminal instanceName={terminalFor()!.name} ttydPort={terminalFor()!.ttydPort} onClose={() => setTerminalFor(null)} />
      )}

      {/* Chromium install modal */}
      <Show when={chromiumFor()}>
        <div class="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl w-[650px] h-[420px] flex flex-col shadow-2xl">
            <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700 shrink-0">
              <span class="font-medium text-sm">
                {chromiumInstalling() ? `Installing Chromium in '${chromiumFor()}'...` : "Chromium Install"}
              </span>
              <div class="flex gap-2">
                <Show when={chromiumDone() && !chromiumInstalling()}>
                  <button class="px-3 py-0.5 text-xs bg-gray-600 hover:bg-gray-500 rounded"
                    onClick={() => handleChromium(chromiumFor()!)}>Retry</button>
                </Show>
                <Show when={!chromiumInstalling()}>
                  <button class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium"
                    onClick={() => setChromiumFor(null)}>Close</button>
                </Show>
              </div>
            </div>
            <div class="flex-1 overflow-y-auto p-3 font-mono text-xs bg-gray-950 min-h-0" ref={chromiumLogRef}>
              {chromiumLogs().map((line) => (
                <div class={
                  line.includes("✓") || line.includes("OK:") ? "text-green-400"
                  : line.includes("✗") || line.includes("ERROR") ? "text-red-400"
                  : line.includes("Installing") || line.includes("Fetching") ? "text-indigo-300"
                  : "text-gray-400"
                }>
                  {line || "\u00A0"}
                </div>
              ))}
              <Show when={chromiumInstalling()}>
                <div class="text-indigo-400 animate-pulse mt-1">Working...</div>
              </Show>
            </div>
          </div>
        </div>
      </Show>

      {/* Export progress */}
      <Show when={showExport()}>
        <ExportProgress isNative={false} onClose={() => setShowExport(false)} />
      </Show>

      {/* noVNC HIL Panel */}
      <Show when={hilInstance()}>
        <div class="fixed inset-0 z-50 flex flex-col">
          <NoVncPanel
            novncUrl={hilUrl()}
            onClose={resumeHeadless}
          />
        </div>
      </Show>
    </div>
  );
}
