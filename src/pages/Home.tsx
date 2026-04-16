import { createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { Instance, ClawType } from "../types";
import OperationModal from "../components/OperationModal";
import { t } from "../i18n";

function ClawTypePicker(props: { clawTypes: ClawType[]; onSelect: (id: string) => void; onClose: () => void }) {
  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={props.onClose}>
      <div class="bg-gray-800 rounded-xl p-5 w-80 max-h-[80vh] overflow-y-auto shadow-xl" onClick={(e: MouseEvent) => e.stopPropagation()}>
        <h3 class="text-base font-semibold text-white mb-1">New Instance</h3>
        <p class="text-xs text-gray-400 mb-4">Choose a claw type to install</p>
        <div class="space-y-1">
          <For each={props.clawTypes}>
            {(claw) => (
              <button
                class="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg hover:bg-gray-700 transition-colors text-left"
                onClick={() => props.onSelect(claw.id)}
              >
                <span class="text-lg">{claw.logo || "📦"}</span>
                <div class="flex-1">
                  <div class="text-sm text-white">{claw.display_name}</div>
                  <div class="text-[10px] text-gray-500">{claw.package_manager !== "npm" ? claw.pip_package : claw.npm_package}</div>
                </div>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
type StatusDetail = { gateway_log: string };

const healthColor: Record<string, string> = {
  running: "bg-green-500", stopped: "bg-gray-500", unreachable: "bg-red-500",
};

export default function Home(props: {
  instances: Instance[];
  healths: Record<string, string>;
  onHealthChange: () => void;
  clawTypes?: ClawType[];
  onAddInstance?: (clawType?: string) => void;
}) {
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal("");
  const [actionHint, setActionHint] = createSignal("");
  const [showClawPicker, setShowClawPicker] = createSignal(false);

  // Status modal (gateway log only)
  const [statusFor, setStatusFor] = createSignal<string | null>(null);
  const [statusData, setStatusData] = createSignal<StatusDetail | null>(null);
  const [statusLoading, setStatusLoading] = createSignal(false);

  async function openStatus(name: string) {
    setStatusFor(name);
    await refreshStatus(name);
  }

  function closeStatus() {
    setStatusFor(null);
  }

  async function refreshStatus(name: string) {
    setStatusLoading(true);
    try {
      const logs = await invoke<string>("get_instance_logs", { name });
      setStatusData({ gateway_log: logs });
    } catch (e) {
      setStatusData({ gateway_log: `Error: ${e}` });
    } finally {
      setStatusLoading(false);
    }
  }

  // Operation modal state
  const [opModal, setOpModal] = createSignal<{ op: "start" | "stop" | "restart"; name: string } | null>(null);

  function showOp(op: "start" | "stop" | "restart", name: string) {
    setActionError("");
    setOpModal({ op, name });
  }

  function onOpComplete() {
    setOpModal(null);
    props.onHealthChange();
  }

  function handleStop(name: string) { showOp("stop", name); }
  function handleStart(name: string) { showOp("start", name); }
  function handleRestart(name: string) { showOp("restart", name); }

  const getHealth = (name: string) => props.healths[name] || "unreachable";

  return (
    <div class="h-full overflow-y-auto p-6 relative">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-xl font-bold">{t("首页", "Home")}</h1>
      </div>

      {/* Operation modal */}
      <Show when={opModal()}>
        <OperationModal
          operation={opModal()!.op}
          instanceName={opModal()!.name}
          onComplete={onOpComplete}
          doAction={async () => {
            const { op, name } = opModal()!;
            if (op === "start") await invoke("start_instance", { name });
            else if (op === "stop") await invoke("stop_instance", { name });
            else { await invoke("stop_instance", { name }); await invoke("start_instance", { name }); }
          }}
        />
      </Show>
      <Show when={actionError()}>
        <div class="mb-4 p-3 bg-red-900/30 border border-red-700 rounded text-sm text-red-400">{actionError()}</div>
      </Show>

      <section class="mb-6">
        <div class="flex items-center justify-between mb-3">
          <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide">{t("实例", "Instances")}</h2>
          <Show when={props.onAddInstance}>
            <button
              class="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 rounded text-white"
              onClick={() => setShowClawPicker(true)}
            >+ Add</button>
          </Show>
        </div>
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
                      <span class="text-sm">{inst.logo}</span>
                      <span class="font-medium">{inst.name}</span>
                      <span class="text-xs text-gray-400">{inst.display_name}</span>
                      <span class="text-xs text-gray-500">
                        {health() === "running" ? t("运行中", "running") : health() === "stopped" ? t("已停止", "stopped") : t("不可达", "unreachable")}
                      </span>
                    </div>
                    <span class="text-sm text-gray-400">v{inst.version}</span>
                  </div>
                  <div class="text-xs text-gray-500">Gateway: 127.0.0.1:{inst.gateway_port}</div>
                  <div class="flex gap-2 mt-3">
                    {isRunning() ? (
                      <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                        disabled={loading()} onClick={() => handleStop(inst.name)}>
                        {actionLoading() === `stop-${inst.name}` ? `${t("停止", "Stop")}...` : t("停止", "Stop")}
                      </button>
                    ) : (
                      <button class="px-3 py-1 text-xs bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50"
                        disabled={loading()} onClick={() => handleStart(inst.name)}>
                        {actionLoading() === `start-${inst.name}` ? `${t("启动", "Start")}...` : t("启动", "Start")}
                      </button>
                    )}
                    <button class="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                      disabled={loading()} onClick={() => handleRestart(inst.name)}>
                      {actionLoading() === `restart-${inst.name}` ? `${t("重启", "Restart")}...` : t("重启", "Restart")}
                    </button>
                    <button class="px-3 py-1 text-xs bg-gray-600 hover:bg-gray-500 rounded ml-auto"
                      onClick={() => openStatus(inst.name)}>
                      {t("状态", "Status")}
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
          {props.instances.length === 0 && (
            <div class="text-gray-500 text-sm">{t("暂无实例", "No instances configured")}</div>
          )}
        </div>
      </section>

      <section class="mb-6">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">{t("安全状态", "Security")}</h2>
        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span><span>{t("所有组件版本最新", "All components up to date")}</span>
          </div>
          <div class="flex items-center gap-2 text-sm text-green-400">
            <span>&#10003;</span><span>{t("无已知 CVE", "No known CVEs")}</span>
          </div>
        </div>
      </section>

      {/* Log Modal */}
      <Show when={statusFor()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl w-[700px] h-[70vh] flex flex-col shadow-2xl">
            <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700 shrink-0">
              <span class="font-medium">{t("终端日志", "Logs")}: {statusFor()}</span>
              <div class="flex gap-2">
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => refreshStatus(statusFor()!)}>{t("刷新", "Refresh")}</button>
                <button class="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => {
                    const content = statusData()?.gateway_log;
                    if (content) navigator.clipboard.writeText(content);
                  }}>Copy</button>
                <button class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium"
                  onClick={closeStatus}>{t("关闭", "Close")}</button>
              </div>
            </div>
            <div class="flex-1 overflow-y-auto p-4 min-h-0">
              <Show when={statusLoading()}>
                <div class="text-gray-400 text-sm">Loading...</div>
              </Show>
              <Show when={!statusLoading() && statusData()}>
                <pre class="font-mono text-xs text-gray-300 whitespace-pre-wrap select-text">
                  {statusData()!.gateway_log}
                </pre>
              </Show>
            </div>
          </div>
        </div>
      </Show>

      {/* Claw type picker dialog */}
      <Show when={showClawPicker()}>
        <ClawTypePicker
          clawTypes={props.clawTypes || []}
          onSelect={(id) => { setShowClawPicker(false); props.onAddInstance?.(id); }}
          onClose={() => setShowClawPicker(false)}
        />
      </Show>
    </div>
  );
}
