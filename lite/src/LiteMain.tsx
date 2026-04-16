import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "@shared/i18n";
import type { Instance } from "@shared/types";
import OperationModal from "@components/OperationModal";
import ExportProgress from "@components/ExportProgress";
import DeleteProgress from "@components/DeleteProgress";

type Props = {
  instance: Instance;
  onRefresh: () => void;
  onDeleted: () => void;
};

export default function LiteMain(props: Props) {
  const [health, setHealth] = createSignal("unreachable");
  const [opModal, setOpModal] = createSignal<{ op: "start" | "stop" | "restart" } | null>(null);
  const [showExport, setShowExport] = createSignal(false);
  const [showDelete, setShowDelete] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const [gatewayToken, setGatewayToken] = createSignal("");

  const inst = () => props.instance;
  const isNative = () => inst().sandbox_type?.toLowerCase() === "native";
  const isRunning = () => health() === "running";

  // Health polling
  async function refreshHealth() {
    try {
      const h = await invoke<string>("get_instance_health", { name: inst().name });
      setHealth(h);
    } catch { setHealth("unreachable"); }
  }

  onMount(() => {
    refreshHealth();
    const timer = setInterval(refreshHealth, 8000);
    onCleanup(() => clearInterval(timer));
  });

  // Token fetch
  async function fetchToken() {
    try {
      const tok = await invoke<string>("get_gateway_token", { name: inst().name });
      setGatewayToken(tok);
    } catch { setGatewayToken(""); }
  }

  async function openPanel() {
    await fetchToken();
    const port = inst().gateway_port;
    const token = gatewayToken();
    const url = token ? `http://127.0.0.1:${port}/?token=${token}` : `http://127.0.0.1:${port}`;
    invoke("open_url_in_browser", { url }).catch(() => {});
  }

  function onOpComplete() { setOpModal(null); refreshHealth(); props.onRefresh(); }

  async function doExport() {
    const cmd = isNative() ? "export_native_bundle" : "export_sandbox";
    try { await invoke(cmd, { name: inst().name }); setShowExport(true); } catch {}
  }

  const healthColor = () => isRunning() ? "bg-green-500" : health() === "stopped" ? "bg-gray-500" : "bg-red-500";
  const healthText = () => isRunning() ? t("运行中", "running") : health() === "stopped" ? t("已停止", "stopped") : t("不可达", "unreachable");

  return (
    <div class="flex flex-col h-full">
      {/* Header */}
      <div class="bg-gray-800 border-b border-gray-700 px-6 py-4">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <span class="text-lg">{inst().logo || "🦞"}</span>
            <div>
              <h1 class="text-lg font-bold">{inst().display_name || "OpenClaw"}</h1>
              <span class="text-xs text-gray-400">{inst().name} — v{inst().version || "?"}</span>
            </div>
          </div>
          <div class="flex items-center gap-2">
            <div class={`w-2.5 h-2.5 rounded-full ${healthColor()}`} />
            <span class="text-sm text-gray-300">{healthText()}</span>
          </div>
        </div>
      </div>

      {/* Content */}
      <div class="flex-1 overflow-y-auto p-6">
        <div class="max-w-lg mx-auto space-y-4">
          {/* Instance info */}
          <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm">
            <div class="flex justify-between"><span class="text-gray-400">Gateway</span><span>127.0.0.1:{inst().gateway_port}</span></div>
            <div class="flex justify-between mt-1"><span class="text-gray-400">{t("类型", "Type")}</span><span>{inst().sandbox_type}</span></div>
          </div>

          {/* Primary actions — row 1 */}
          <div class="flex gap-2 justify-center">
            <Show when={isRunning()}>
              <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm"
                onClick={openPanel}>{t("打开控制面板", "Open Control Panel")}</button>
              <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                onClick={() => setOpModal({ op: "stop" })}>{t("停止", "Stop")}</button>
              <button class="px-3 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                onClick={() => setOpModal({ op: "restart" })}>{t("重启", "Restart")}</button>
            </Show>
            <Show when={!isRunning()}>
              <button class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm"
                onClick={() => setOpModal({ op: "start" })}>{t("启动", "Start")}</button>
            </Show>
          </div>

          {/* Secondary actions — row 2 */}
          <div class="flex gap-2 justify-center">
            <button class="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 rounded text-xs"
              onClick={doExport}>{t("导出备份", "Export Backup")}</button>
            <button class="px-3 py-1.5 bg-red-900/60 hover:bg-red-800 text-red-300 rounded text-xs"
              onClick={() => setConfirmDelete(true)}>{t("删除", "Delete")}</button>
          </div>

          {/* Sandbox-only features */}
          <Show when={!isNative() && isRunning()}>
            <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
              <h3 class="text-sm font-medium text-gray-400 mb-2">{t("沙盒工具", "Sandbox Tools")}</h3>
              <div class="flex gap-2">
                <button class="px-3 py-1.5 bg-gray-700 hover:bg-gray-600 rounded text-xs"
                  onClick={() => invoke("open_install_window", { instanceName: inst().name })}>{t("终端", "Terminal")}</button>
              </div>
            </div>
          </Show>
        </div>
      </div>

      {/* Footer */}
      <div class="bg-gray-800 border-t border-gray-700 px-6 py-2 flex items-center justify-between text-xs text-gray-500">
        <span>ClawEnv Lite v0.2.0</span>
        <button class="hover:text-gray-300" onClick={() => invoke("exit_app")}>{t("退出", "Quit")}</button>
      </div>

      {/* Modals */}
      <Show when={opModal()}>
        <OperationModal
          operation={opModal()!.op}
          instanceName={inst().name}
          onComplete={onOpComplete}
          doAction={async () => {
            const op = opModal()!.op;
            if (op === "start") await invoke("start_instance", { name: inst().name });
            else if (op === "stop") await invoke("stop_instance", { name: inst().name });
            else { await invoke("stop_instance", { name: inst().name }); await invoke("start_instance", { name: inst().name }); }
          }}
        />
      </Show>

      <Show when={showExport()}>
        <ExportProgress isNative={isNative()} onClose={() => setShowExport(false)} />
      </Show>

      <Show when={showDelete}>
        <DeleteProgress instanceName={inst().name} onComplete={props.onDeleted} onError={() => setShowDelete(false)} />
      </Show>

      <Show when={confirmDelete()}>
        <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
          <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-80 shadow-2xl">
            <h3 class="text-base font-bold mb-2">{t("确认删除", "Confirm Delete")}</h3>
            <p class="text-sm text-gray-300 mb-4">
              {t("删除后将清除所有数据，无法恢复。", "This will remove all data permanently.")}
            </p>
            <div class="flex gap-2 justify-end">
              <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                onClick={() => setConfirmDelete(false)}>{t("取消", "Cancel")}</button>
              <button class="px-3 py-1.5 text-sm bg-red-700 hover:bg-red-600 rounded"
                onClick={() => { setConfirmDelete(false); setShowDelete(true); }}>{t("删除", "Delete")}</button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}
