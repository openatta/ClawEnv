import { createSignal, onMount, Switch, Match } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";
import MainLayout from "./layouts/MainLayout";
import InstallWizard from "./pages/Install";
import UpgradePrompt from "./components/UpgradePrompt";
import type { Instance } from "./types";

type LaunchState =
  | { type: "loading" }
  | { type: "first_run" }
  | { type: "not_installed" }
  | { type: "upgrade_available"; instances: Instance[] }
  | { type: "ready"; instances: Instance[] }
  | { type: "install_window"; instanceName: string; clawType: string };

export default function App() {
  const [state, setState] = createSignal<LaunchState>({ type: "loading" });

  onMount(async () => {
    // Check URL params — install window passes ?mode=install&name=xxx
    const params = new URLSearchParams(window.location.search);
    if (params.get("mode") === "install") {
      setState({ type: "install_window", instanceName: params.get("name") || "default", clawType: params.get("clawType") || "openclaw" });
      return;
    }

    // Normal main window
    try {
      const result = await invoke<LaunchState>("detect_launch_state");
      // detect_launch_state returns InstanceConfig (nested openclaw.gateway_port)
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
