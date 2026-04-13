import { createSignal, onMount, For, Switch, Match } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";
import MainLayout from "./layouts/MainLayout";
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

  return (
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
                // Notify main window to refresh
                await emit("instances-changed");
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
  );
}
