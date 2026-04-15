import { createSignal, onMount, onCleanup, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type StageInfo = {
  stage: string;
  status: string;
  message: string;
};

const STAGE_LABELS: Record<string, string> = {
  stop: "Stop Instance",
  kill: "Kill Processes",
  delete_files: "Delete Files",
  update_config: "Update Config",
};

const STAGES = ["stop", "kill", "delete_files", "update_config"];

export default function DeleteProgress(props: {
  instanceName: string;
  onComplete: () => void;
  onError: (msg: string) => void;
}) {
  const [stageStates, setStageStates] = createSignal<Record<string, StageInfo>>({});
  const [done, setDone] = createSignal(false);
  const [error, setError] = createSignal("");

  onMount(async () => {
    const unProgress = await listen<StageInfo>("delete-progress", (ev) => {
      setStageStates((prev) => ({ ...prev, [ev.payload.stage]: ev.payload }));
    });
    const unComplete = await listen("delete-complete", () => {
      setDone(true);
      setTimeout(() => { props.onComplete(); }, 1500);
    });
    const unFailed = await listen<string>("delete-failed", (ev) => {
      setError(String(ev.payload));
      props.onError(String(ev.payload));
    });

    // Start the delete
    try {
      await invoke("delete_instance_with_progress", { name: props.instanceName });
    } catch (e) {
      setError(String(e));
    }

    onCleanup(() => { unProgress(); unComplete(); unFailed(); });
  });

  function stageStatus(name: string): string {
    return stageStates()[name]?.status || "pending";
  }

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl">
        <h3 class="text-base font-bold mb-4">
          {done() ? "Deleted" : error() ? "Delete Failed" : `Deleting '${props.instanceName}'...`}
        </h3>

        <div class="mb-4 space-y-1">
          <For each={STAGES}>
            {(name) => {
              const st = () => stageStatus(name);
              const info = () => stageStates()[name];
              return (
                <div class="flex items-center gap-2 text-sm">
                  <div class={`w-4 h-4 rounded-full flex items-center justify-center text-[10px] shrink-0 ${
                    st() === "done" ? "bg-green-600 text-white" :
                    st() === "active" ? "bg-indigo-600 text-white animate-pulse" :
                    st() === "error" ? "bg-red-600 text-white" :
                    "bg-gray-600 text-gray-400"
                  }`}>
                    {st() === "done" ? "✓" : st() === "active" ? "●" : st() === "error" ? "✗" : "○"}
                  </div>
                  <span class={st() === "active" ? "text-white" : st() === "done" ? "text-green-400" : st() === "error" ? "text-red-400" : "text-gray-500"}>
                    {STAGE_LABELS[name] || name}
                  </span>
                  <Show when={info()?.message}>
                    <span class="text-xs text-gray-500 ml-auto">{info()?.message}</span>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>

        <Show when={error()}>
          <div class="mb-3 p-2 bg-red-900/30 border border-red-700 rounded text-xs text-red-400">
            {error()}
          </div>
          <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded w-full"
            onClick={() => props.onComplete()}>Close</button>
        </Show>

        <Show when={done()}>
          <div class="p-2 bg-green-900/30 border border-green-700 rounded text-xs text-green-400">
            Instance deleted successfully
          </div>
        </Show>
      </div>
    </div>
  );
}
