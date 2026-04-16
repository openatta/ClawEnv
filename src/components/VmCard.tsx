import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

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

/** Format size: "4294967296"->"4.0 GB", "4GiB"->"4 GB", "100GiB"->"100 GB" */
export function fmtSize(s: string): string {
  if (!s || s === "-") return "-";
  const m = s.match(/^([\d.]+)\s*(GiB|MiB|KiB|GB|MB|KB|TB)/i);
  if (m) {
    const v = parseFloat(m[1]);
    const u = m[2].replace("iB", "B");
    return `${v} ${u}`;
  }
  const b = parseInt(s);
  if (isNaN(b)) return s;
  if (b >= 1073741824) return `${(b / 1073741824).toFixed(1)} GB`;
  if (b >= 1048576) return `${(b / 1048576).toFixed(0)} MB`;
  return s;
}

export default function VmCard(props: {
  vm: SandboxVm;
  onRefresh: () => void;
  onTerminal: (info: { name: string; ttydPort?: number }) => void;
  onChromium: (name: string) => void;
  onBrowser: (name: string) => void;
  onExport: (name: string) => void;
  browserLoading: string;
}) {
  const [actionLoading, setActionLoading] = createSignal("");
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const [showConfig, setShowConfig] = createSignal(false);
  const [chromiumInstalled, setChromiumInstalled] = createSignal<boolean | null>(null);

  // Check chromium status when VM is running and managed
  if (props.vm.managed) {
    const name = props.vm.name.replace(/^clawenv-/, "");
    invoke<boolean>("check_chromium_installed", { name }).then(
      (v) => setChromiumInstalled(v),
      () => setChromiumInstalled(null)
    );
  }
  const [cfgCpus, setCfgCpus] = createSignal(parseInt(props.vm.cpus) || 4);
  const [cfgMemory, setCfgMemory] = createSignal(Math.round(parseInt(props.vm.memory) / 1073741824) || 4);
  const [cfgSaving, setCfgSaving] = createSignal(false);
  const [cfgError, setCfgError] = createSignal("");

  const isRunning = () => props.vm.status.toLowerCase().includes("running");
  const statusColor = () => isRunning() ? "bg-green-500" : "bg-gray-500";
  const borderColor = () => props.vm.managed ? "border-green-700/30" : "border-gray-700";

  // Strip "clawenv-" prefix for IPC calls that expect instance name
  const instanceName = () => props.vm.name.replace(/^clawenv-/, "");

  async function doAction(action: string) {
    setActionLoading(action);
    try {
      await invoke("sandbox_vm_action", { vmName: props.vm.name, action });
      // Wait a moment for state to settle
      await new Promise((r) => setTimeout(r, 1000));
      props.onRefresh();
    } catch (e) {
      alert(`${action} failed: ${e}`);
    } finally {
      setActionLoading("");
    }
  }

  async function doDelete() {
    setConfirmDelete(false);
    setActionLoading("delete");
    try {
      // If managed, also delete the associated OC instance config
      if (props.vm.managed) {
        try { await invoke("delete_instance", { name: instanceName() }); } catch { /* may already be gone */ }
      }
      await invoke("sandbox_vm_action", { vmName: props.vm.name, action: "delete" });
      props.onRefresh();
    } catch (e) {
      console.error(`Delete failed: ${e}`);
    } finally {
      setActionLoading("");
    }
  }

  return (
    <div class={`bg-gray-800 rounded-lg p-3 border ${borderColor()}`}>
      {/* Header row */}
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <div class={`w-2 h-2 rounded-full ${statusColor()}`} />
          <span class="font-medium text-sm">{props.vm.name}</span>
          {props.vm.managed ? (
            <span class="text-[10px] px-1.5 py-0.5 bg-green-900/50 text-green-400 rounded">managed</span>
          ) : (
            <span class="text-[10px] px-1.5 py-0.5 bg-gray-700 text-gray-400 rounded">external</span>
          )}
        </div>
        <span class="text-xs text-gray-400">{props.vm.status}</span>
      </div>

      {/* Resource info */}
      <div class="flex gap-4 mt-2 text-xs text-gray-500">
        <span>CPU: {props.vm.cpus}</span>
        <span>RAM: {fmtSize(props.vm.memory)}</span>
        <span>Disk: {fmtSize(props.vm.disk)}</span>
        <Show when={props.vm.dir_size && props.vm.dir_size !== "-"}>
          <span>Used: {props.vm.dir_size}</span>
        </Show>
      </div>

      {/* Action buttons */}
      <div class="flex gap-2 mt-2 items-center">
        {actionLoading() ? (
          <span class="text-xs text-indigo-300 animate-pulse">
            {actionLoading() === "start" ? "Starting..." : actionLoading() === "stop" ? "Stopping..." : actionLoading() === "delete" ? "Deleting..." : "Processing..."}
          </span>
        ) : isRunning() ? (
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => doAction("stop")}>
            Stop
          </button>
        ) : (
          <button class="px-2 py-0.5 text-xs bg-indigo-700 hover:bg-indigo-600 rounded"
            onClick={() => doAction("start")}>
            Start
          </button>
        )}

        {/* Terminal only for managed + running */}
        <Show when={isRunning() && props.vm.managed}>
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => props.onTerminal({ name: instanceName(), ttydPort: props.vm.ttyd_port })}>
            Terminal
          </button>
          <button
            class={`px-2 py-0.5 text-xs rounded ${chromiumInstalled() ? "bg-gray-600 text-gray-400 cursor-default" : "bg-indigo-700 hover:bg-indigo-600"}`}
            disabled={chromiumInstalled() === true}
            onClick={() => { if (!chromiumInstalled()) props.onChromium(instanceName()); }}>
            {chromiumInstalled() ? "Chromium installed" : "Install Chromium"}
          </button>
          <Show when={chromiumInstalled()}>
            <button class="px-2 py-0.5 text-xs bg-orange-700 hover:bg-orange-600 rounded"
              disabled={!!props.browserLoading}
              onClick={() => props.onBrowser(instanceName())}>
              {props.browserLoading === instanceName() ? "Starting..." : "Browser HIL"}
            </button>
          </Show>
        </Show>

        {/* Configure (managed only) */}
        <Show when={props.vm.managed}>
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => setShowConfig(true)}>
            Configure
          </button>
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => props.onExport(instanceName())}>
            Export
          </button>
        </Show>

        {/* Delete with confirmation */}
        {confirmDelete() ? (
          <div class="flex items-center gap-1 ml-auto">
            <span class="text-xs text-red-400">Delete?</span>
            <button class="px-2 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded"
              onClick={doDelete}>Yes</button>
            <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
              onClick={() => setConfirmDelete(false)}>No</button>
          </div>
        ) : (
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-red-700 rounded ml-auto disabled:opacity-50"
            disabled={!!actionLoading()} onClick={() => setConfirmDelete(true)}>
            {actionLoading() === "delete" ? "..." : "Delete"}
          </button>
        )}
      </div>

      {/* Config modal */}
      <Show when={showConfig()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl">
            <h3 class="text-base font-bold mb-4">Configure — {props.vm.name}</h3>
            <div class="space-y-4">
              <div>
                <label class="block text-xs text-gray-400 mb-1">CPU Cores: {cfgCpus()}</label>
                <input type="range" min="1" max="8" value={cfgCpus()}
                  onInput={(e) => setCfgCpus(parseInt(e.currentTarget.value))} class="w-full" />
                <div class="flex justify-between text-[10px] text-gray-500"><span>1</span><span>8</span></div>
              </div>
              <div>
                <label class="block text-xs text-gray-400 mb-1">Memory: {cfgMemory()} GB</label>
                <input type="range" min="1" max="16" value={cfgMemory()}
                  onInput={(e) => setCfgMemory(parseInt(e.currentTarget.value))} class="w-full" />
                <div class="flex justify-between text-[10px] text-gray-500"><span>1 GB</span><span>16 GB</span></div>
              </div>
            </div>
            {cfgError() && <p class="text-xs text-red-400 mt-3">{cfgError()}</p>}
            <p class="text-xs text-yellow-500 mt-3">Changes require VM restart to take effect.</p>
            <div class="flex gap-2 justify-end mt-4">
              <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                onClick={() => setShowConfig(false)}>Cancel</button>
              <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
                disabled={cfgSaving()}
                onClick={async () => {
                  setCfgSaving(true); setCfgError("");
                  try {
                    await invoke("edit_instance_resources", { name: instanceName(), cpus: cfgCpus(), memoryMb: cfgMemory() * 1024, diskGb: null });
                    setShowConfig(false); props.onRefresh();
                  } catch (e) { setCfgError(String(e)); }
                  finally { setCfgSaving(false); }
                }}>
                {cfgSaving() ? "Saving..." : "Save"}
              </button>
              <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
                disabled={cfgSaving()}
                onClick={async () => {
                  setCfgSaving(true); setCfgError("");
                  try {
                    await invoke("edit_instance_resources", { name: instanceName(), cpus: cfgCpus(), memoryMb: cfgMemory() * 1024, diskGb: null });
                    await invoke("sandbox_vm_action", { vmName: props.vm.name, action: "start" });
                    setShowConfig(false); props.onRefresh();
                  } catch (e) { setCfgError(String(e)); }
                  finally { setCfgSaving(false); }
                }}>
                {cfgSaving() ? "..." : "Save & Restart"}
              </button>
            </div>
          </div>
        </div>
      </Show>

    </div>
  );
}
