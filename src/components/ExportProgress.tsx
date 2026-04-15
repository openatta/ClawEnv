import { createSignal, onMount, onCleanup, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type StageInfo = {
  stage: string;
  status: string; // pending | active | done | error
  percent: number;
  message: string;
};

const SANDBOX_STAGES = ["stop", "count", "compress", "checksum", "restart"];
const NATIVE_STAGES = ["count", "compress", "checksum"];

const STAGE_LABELS: Record<string, string> = {
  stop: "Stop Instance",
  count: "Count Files",
  compress: "Compress",
  checksum: "Checksum",
  restart: "Restart Instance",
};

export default function ExportProgress(props: {
  isNative: boolean;
  onClose: () => void;
}) {
  const stages = () => props.isNative ? NATIVE_STAGES : SANDBOX_STAGES;
  const [stageStates, setStageStates] = createSignal<Record<string, StageInfo>>({});
  const [currentFile, setCurrentFile] = createSignal("");
  const [currentPercent, setCurrentPercent] = createSignal(0);
  const [done, setDone] = createSignal(false);
  const [error, setError] = createSignal("");

  onMount(async () => {
    const unProgress = await listen<StageInfo>("export-progress", (ev) => {
      const s = ev.payload;
      setStageStates((prev) => ({ ...prev, [s.stage]: s }));
      if (s.status === "active") {
        setCurrentFile(s.message);
        setCurrentPercent(s.percent);
      }
    });
    const unComplete = await listen<string>("export-complete", (ev) => {
      setDone(true);
      setCurrentFile(`Exported to ${ev.payload}`);
    });
    const unFailed = await listen<string>("export-failed", (ev) => {
      setError(String(ev.payload));
    });
    onCleanup(() => { unProgress(); unComplete(); unFailed(); });
  });

  function stageStatus(name: string): string {
    return stageStates()[name]?.status || "pending";
  }

  async function doCancel() {
    await invoke("export_cancel");
    setError("Cancelled");
  }

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-[420px] shadow-2xl">
        <h3 class="text-base font-bold mb-4">
          {done() ? "Export Complete" : error() ? "Export Failed" : "Exporting..."}
        </h3>

        {/* Stage timeline */}
        <div class="mb-4 space-y-1">
          <For each={stages()}>
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
                  <span class={st() === "active" ? "text-white" : st() === "done" ? "text-green-400" : "text-gray-500"}>
                    {STAGE_LABELS[name] || name}
                  </span>
                  <Show when={info()?.status === "done"}>
                    <span class="text-xs text-gray-500 ml-auto">{info()?.message}</span>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>

        {/* Active stage progress bar */}
        <Show when={!done() && !error()}>
          <div class="mb-3">
            <div class="w-full bg-gray-700 rounded-full h-1.5 mb-1">
              <div class="bg-indigo-500 h-1.5 rounded-full transition-all"
                style={{ width: `${currentPercent()}%` }} />
            </div>
            <p class="text-[11px] text-gray-400 truncate">{currentFile()}</p>
          </div>
        </Show>

        {/* Error */}
        <Show when={error()}>
          <div class="mb-3 p-2 bg-red-900/30 border border-red-700 rounded text-xs text-red-400">
            {error()}
          </div>
        </Show>

        {/* Done message */}
        <Show when={done()}>
          <div class="mb-3 p-2 bg-green-900/30 border border-green-700 rounded text-xs text-green-400">
            {currentFile()}
          </div>
        </Show>

        {/* Buttons */}
        <div class="flex justify-end gap-2">
          <Show when={!done() && !error()}>
            <button class="px-3 py-1.5 text-sm bg-red-700 hover:bg-red-600 rounded"
              onClick={doCancel}>Cancel</button>
          </Show>
          <Show when={done() || error()}>
            <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
              onClick={props.onClose}>Close</button>
          </Show>
        </div>
      </div>
    </div>
  );
}
