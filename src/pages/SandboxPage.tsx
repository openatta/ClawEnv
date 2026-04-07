import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import SandboxTerminal from "../components/Terminal";

type SandboxVm = {
  name: string;
  status: string;
  cpus: string;
  memory: string;
  disk: string;
  dir_size: string;
  managed: boolean;
};

export default function SandboxPage() {
  const [vms, setVms] = createSignal<SandboxVm[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal("");
  const [diskUsage, setDiskUsage] = createSignal("");
  const [terminalFor, setTerminalFor] = createSignal<string | null>(null);

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
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Platform</h2>
          <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm text-gray-300">
            <div class="flex justify-between">
              <span>Total VMs/Containers</span>
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
              {(vm) => <VmCard vm={vm} onRefresh={refresh} onTerminal={setTerminalFor} />}
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
              {(vm) => <VmCard vm={vm} onRefresh={refresh} onTerminal={setTerminalFor} />}
            </For>
          </div>
        </section>
      </Show>

      {/* Terminal modal */}
      {terminalFor() && (
        <SandboxTerminal instanceName={terminalFor()!} onClose={() => setTerminalFor(null)} />
      )}
    </div>
  );
}

function VmCard(props: {
  vm: SandboxVm;
  onRefresh: () => void;
  onTerminal: (name: string) => void;
}) {
  const [actionLoading, setActionLoading] = createSignal("");
  const [confirmDelete, setConfirmDelete] = createSignal(false);

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
      await invoke("sandbox_vm_action", { vmName: props.vm.name, action: "delete" });
      props.onRefresh();
    } catch (e) {
      alert(`Delete failed: ${e}`);
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
      <div class="flex gap-2 mt-2">
        {isRunning() ? (
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
            disabled={!!actionLoading()} onClick={() => doAction("stop")}>
            {actionLoading() === "stop" ? "..." : "Stop"}
          </button>
        ) : (
          <button class="px-2 py-0.5 text-xs bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50"
            disabled={!!actionLoading()} onClick={() => doAction("start")}>
            {actionLoading() === "start" ? "..." : "Start"}
          </button>
        )}

        {/* Terminal only for managed + running */}
        <Show when={isRunning() && props.vm.managed}>
          <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => props.onTerminal(instanceName())}>
            Terminal
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
    </div>
  );
}

/** Format size: "4294967296"→"4.0 GB", "4GiB"��"4 GB", "100GiB"��"100 GB" */
function fmtSize(s: string): string {
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
