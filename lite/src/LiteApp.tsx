import { createSignal, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { t } from "@shared/i18n";
import type { Instance } from "@shared/types";
import LiteInstall from "./LiteInstall";
import LiteMain from "./LiteMain";

type State = "loading" | "install" | "manage";

export default function LiteApp() {
  const [state, setState] = createSignal<State>("loading");
  const [instance, setInstance] = createSignal<Instance | null>(null);

  onMount(async () => {
    try {
      const list = await invoke<Instance[]>("list_instances");
      if (list.length > 0) {
        setInstance(list[0]);
        setState("manage");
      } else {
        setState("install");
      }
    } catch {
      setState("install");
    }
  });

  function onInstallComplete(inst: Instance) {
    setInstance(inst);
    setState("manage");
  }

  async function onDeleted() {
    setInstance(null);
    setState("install");
  }

  async function refreshInstance() {
    try {
      const list = await invoke<Instance[]>("list_instances");
      if (list.length > 0) setInstance(list[0]);
    } catch {}
  }

  return (
    <div class="h-screen bg-gray-900 text-white overflow-hidden">
      <Show when={state() === "loading"}>
        <div class="flex h-full items-center justify-center">
          <div class="text-center">
            <div class="text-2xl font-bold mb-2">ClawEnv Lite</div>
            <div class="text-gray-400">{t("加载中...", "Loading...")}</div>
          </div>
        </div>
      </Show>

      <Show when={state() === "install"}>
        <LiteInstall onComplete={onInstallComplete} />
      </Show>

      <Show when={state() === "manage" && instance()}>
        <LiteMain
          instance={instance()!}
          onRefresh={refreshInstance}
          onDeleted={onDeleted}
        />
      </Show>
    </div>
  );
}
