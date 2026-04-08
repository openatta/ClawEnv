import { createSignal, onMount, Switch, Match } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import MainLayout from "./layouts/MainLayout";
import InstallWizard from "./pages/Install";
import UpgradePrompt from "./components/UpgradePrompt";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
  ttyd_port: number;
};

type LaunchState =
  | { type: "loading" }
  | { type: "first_run" }
  | { type: "not_installed" }
  | { type: "upgrade_available"; instances: Instance[] }
  | { type: "ready"; instances: Instance[] };

export default function App() {
  const [state, setState] = createSignal<LaunchState>({ type: "loading" });

  onMount(async () => {
    try {
      const result = await invoke<LaunchState>("detect_launch_state");
      setState(result);
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
