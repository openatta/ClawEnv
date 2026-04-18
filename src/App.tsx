import { createSignal, onMount, onCleanup, Show, For, Switch, Match } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit, listen } from "@tauri-apps/api/event";
import MainLayout from "./layouts/MainLayout";
import { t } from "./i18n";
import InstallWizard from "./pages/Install";
import UpgradePrompt from "./components/UpgradePrompt";
import type { Instance } from "./types";

/** Tray popup — a small borderless window acting as the tray right-click menu. */
function TrayPopup() {
  const [instances, setInstances] = createSignal<Instance[]>([]);

  onMount(async () => {
    try {
      const list = await invoke<Instance[]>("list_instances");
      setInstances(list);
    } catch { /* empty */ }
  });

  const openMain = async () => {
    try {
      // Show main window via IPC to the main window
      const wins = await invoke<string[]>("list_instances"); // just to trigger app
    } catch {}
    getCurrentWindow().close();
  };

  const doAction = async (action: string, name: string) => {
    try { await invoke(`${action}_instance`, { name }); } catch {}
    getCurrentWindow().close();
  };

  return (
    <div class="bg-gray-900 text-white text-sm h-full flex flex-col p-1 select-none" style="border: 1px solid #374151; border-radius: 6px;">
      <For each={instances()}>
        {(inst) => (
          <div class="px-3 py-1.5 flex items-center justify-between hover:bg-gray-800 rounded">
            <span class="flex items-center gap-1.5">
              <span class="text-xs">{inst.logo}</span>
              <span>{inst.name}</span>
            </span>
            <span class="text-[10px] text-gray-500">{inst.version}</span>
          </div>
        )}
      </For>
      <div class="border-t border-gray-700 my-1" />
      <button class="px-3 py-1.5 text-left hover:bg-gray-800 rounded w-full" onClick={openMain}>
        Open ClawEnv
      </button>
      <button class="px-3 py-1.5 text-left hover:bg-red-900/50 text-red-400 rounded w-full"
        onClick={() => invoke("exit_app").catch(() => getCurrentWindow().close())}>
        Quit
      </button>
    </div>
  );
}

type LaunchState =
  | { type: "loading" }
  | { type: "first_run" }
  | { type: "not_installed" }
  | { type: "upgrade_available"; instances: Instance[] }
  | { type: "ready"; instances: Instance[] }
  | { type: "install_window"; instanceName: string; clawType: string }
  | { type: "tray_popup" };

export default function App() {
  const [state, setState] = createSignal<LaunchState>({ type: "loading" });
  const [showQuitDialog, setShowQuitDialog] = createSignal(false);
  const [runningCount, setRunningCount] = createSignal(0);

  // Exec approval dialog state
  const [approvalCommand, setApprovalCommand] = createSignal<string | null>(null);

  // Listen for exec-approval-required from bridge
  onMount(async () => {
    const unlisten = await listen<string>("exec-approval-required", (ev) => {
      try {
        const data = JSON.parse(ev.payload);
        setApprovalCommand(data.command || "unknown command");
      } catch { setApprovalCommand(ev.payload); }
    });
    onCleanup(() => unlisten());
  });

  // Listen for quit-requested from tray — fast path, no health check
  onMount(async () => {
    const unlisten = await listen("quit-requested", () => {
      // Just show the dialog immediately — no slow CLI calls
      setShowQuitDialog(true);
    });
    onCleanup(() => unlisten());
  });

  onMount(async () => {
    // Check URL params — install window passes ?mode=install&name=xxx
    const params = new URLSearchParams(window.location.search);
    if (params.get("mode") === "install") {
      setState({ type: "install_window", instanceName: params.get("name") || "default", clawType: params.get("clawType") || "openclaw" });
      return;
    }
    if (params.get("mode") === "tray-popup") {
      setState({ type: "tray_popup" });
      return;
    }

    // Normal main window
    try {
      const result = await invoke<LaunchState>("detect_launch_state");
      // detect_launch_state returns InstanceConfig (nested gateway.gateway_port)
      // but UI needs flat Instance format. Re-fetch via list_instances for correct shape.
      if (result.type === "ready" || result.type === "upgrade_available") {
        const instances = await invoke<Instance[]>("list_instances");
        setState({ ...result, instances });
      } else {
        setState(result);
      }
    } catch {
      setState({ type: "first_run" });
    }
  });

  return (<>
    <Switch
      fallback={
        <div class="flex h-screen items-center justify-center bg-gray-900 text-white">
          <div class="text-center">
            <div class="text-2xl font-bold mb-2">ClawEnv</div>
            <div class="text-gray-400">Loading...</div>
          </div>
        </div>
      }
    >
      {/* Independent install window */}
      <Match when={state().type === "install_window"}>
        {(() => {
          const s = state();
          if (s.type !== "install_window") return null;
          return (
            <InstallWizard
              defaultInstanceName={s.instanceName}
              clawType={s.clawType}
              onComplete={async () => {
                // Notify the main window so Home / ClawPage pick up the new
                // instance. The backend's install_openclaw IPC also emits
                // `instance-changed` on success (belt-and-braces); this
                // frontend emit is the fallback path so even a future refactor
                // that drops the backend emit keeps the UI in sync. Shape
                // mirrors tauri/src/ipc/emit.rs::InstanceChanged.
                await emit("instance-changed", {
                  action: "install",
                  instance: s.instanceName,
                });
                // Close this install window
                getCurrentWindow().close();
              }}
              onBack={() => getCurrentWindow().close()}
            />
          );
        })()}
      </Match>

      {/* First run — go straight to install */}
      <Match when={state().type === "first_run" || state().type === "not_installed"}>
        <InstallWizard
          onComplete={(instances: Instance[]) =>
            setState({ type: "ready", instances })
          }
        />
      </Match>

      <Match when={state().type === "upgrade_available"}>
        {(() => {
          const s = state();
          if (s.type !== "upgrade_available") return null;
          return (
            <>
              <MainLayout instances={s.instances} />
              <UpgradePrompt
                instances={s.instances}
                onSkip={() => setState({ type: "ready", instances: s.instances })}
                onUpgraded={(instances) =>
                  setState({ type: "ready", instances })
                }
              />
            </>
          );
        })()}
      </Match>

      <Match when={state().type === "ready"}>
        {(() => {
          const s = state();
          if (s.type !== "ready") return null;
          return <MainLayout instances={s.instances} />;
        })()}
      </Match>
    </Switch>

    {/* Quit confirmation dialog */}
    <Show when={showQuitDialog()}>
      <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]">
        <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl text-white">
          <h3 class="text-base font-bold mb-2">{t("退出 ClawEnv", "Quit ClawEnv")}</h3>
          <p class="text-sm text-gray-300 mb-4">
            {t("实例可能仍在运行，请选择退出方式：", "Instances may still be running. Choose how to exit:")}
          </p>
          <div class="flex flex-col gap-2">
            <button class="px-3 py-2 text-sm bg-indigo-600 hover:bg-indigo-500 rounded w-full"
              onClick={() => invoke("exit_app")}>
              {t("退出（保持实例运行）", "Quit (keep instances running)")}
            </button>
            <button class="px-3 py-2 text-sm bg-red-700 hover:bg-red-600 rounded w-full"
              onClick={async () => {
                try { await invoke("stop_all_instances"); } catch {}
                invoke("exit_app");
              }}>
              {t("停止所有实例并退出", "Stop all instances and quit")}
            </button>
            <button class="px-3 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded w-full"
              onClick={() => setShowQuitDialog(false)}>
              {t("取消", "Cancel")}
            </button>
          </div>
        </div>
      </div>
    </Show>

    {/* Exec approval dialog */}
    <Show when={approvalCommand()}>
      <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]">
        <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl text-white">
          <h3 class="text-base font-bold mb-2 text-orange-400">{t("执行审批", "Exec Approval Required")}</h3>
          <p class="text-sm text-gray-300 mb-2">{t("Agent 请求在你的机器上执行命令：", "An agent wants to execute a command on your machine:")}</p>
          <pre class="bg-gray-950 border border-gray-600 rounded p-2 text-xs text-green-400 mb-4 whitespace-pre-wrap break-all">
            {approvalCommand()}
          </pre>
          <div class="flex gap-2 justify-end">
            <button class="px-3 py-1.5 text-sm bg-red-700 hover:bg-red-600 rounded"
              onClick={async () => { await invoke("exec_deny"); setApprovalCommand(null); }}>
              {t("拒绝", "Deny")}
            </button>
            <button class="px-3 py-1.5 text-sm bg-green-700 hover:bg-green-600 rounded"
              onClick={async () => { await invoke("exec_approve"); setApprovalCommand(null); }}>
              {t("允许", "Approve")}
            </button>
          </div>
        </div>
      </div>
    </Show>
    </>
  );
}
