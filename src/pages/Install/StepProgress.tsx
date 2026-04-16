import { createSignal, onMount, onCleanup, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { InstallState, InstallProgress } from "./types";
import { makeInstallStages } from "./types";
import LogBox from "./LogBox";

export default function StepProgress(props: {
  state: InstallState;
  stages: Array<{ key: string; label: string }>;
  onComplete: () => void;
  onError: (msg: string) => void;
}) {
  const [progress, setProgress] = createSignal(0);
  const [progressMessage, setProgressMessage] = createSignal("");
  const [installing, setInstalling] = createSignal(false);
  const [installError, setInstallError] = createSignal("");
  const [installLogs, setInstallLogs] = createSignal<string[]>([]);
  const [completedStages, setCompletedStages] = createSignal<Set<string>>(new Set<string>());
  const [currentStage, setCurrentStage] = createSignal("");

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFailed: UnlistenFn | null = null;

  onCleanup(() => { unlistenProgress?.(); unlistenComplete?.(); unlistenFailed?.(); });

  async function startInstall() {
    setInstalling(true); setInstallError(""); setProgress(0);
    setProgressMessage("Starting..."); setInstallLogs([]);
    setCompletedStages(new Set<string>()); setCurrentStage("");

    const IDLE_TIMEOUT = 5 * 60 * 1000; // 5 min without any update -> timeout
    let done = false;
    let timer: ReturnType<typeof setTimeout> = undefined!;
    function resetTimer() {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (!done) { cleanup(); setInstalling(false); setInstallError("Installation stalled — no progress for 5 minutes"); props.onError("Installation stalled — no progress for 5 minutes"); }
      }, IDLE_TIMEOUT);
    }
    function cleanup() { done = true; clearTimeout(timer); unlistenProgress?.(); unlistenComplete?.(); unlistenFailed?.(); }
    resetTimer();

    unlistenProgress = await listen<InstallProgress>("install-progress", (ev) => {
      resetTimer();
      const p = ev.payload;
      setProgress(p.percent); setProgressMessage(p.message);
      setInstallLogs(l => [...l, `[${p.percent}%] ${p.message}`]);
      setCurrentStage(p.stage);
      const idx = props.stages.findIndex(s => s.key === p.stage);
      if (idx > 0) {
        setCompletedStages(prev => {
          const next = new Set<string>(prev);
          for (let i = 0; i < idx; i++) next.add(props.stages[i].key);
          return next;
        });
      }
    });

    unlistenComplete = await listen("install-complete", () => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setCompletedStages(new Set<string>(props.stages.map(s => s.key)));
      setInstallLogs(l => [...l, "✓ Installation complete!"]);
      props.onComplete();
    });

    unlistenFailed = await listen<string>("install-failed", (ev) => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setInstallError(String(ev.payload));
      setInstallLogs(l => [...l, `✗ ERROR: ${ev.payload}`]);
      props.onError(String(ev.payload));
    });

    try {
      const method = props.state.installMethod;
      if (method === "native-import") {
        await invoke("install_openclaw", {
          instanceName: props.state.instanceName, clawType: props.state.clawType, clawVersion: "latest",
          apiKey: props.state.apiKey || null, useNative: true,
          installBrowser: false, installMcpBridge: false,
          gatewayPort: 0, image: props.state.localFilePath,
        });
      } else {
        await invoke("install_openclaw", {
          instanceName: props.state.instanceName, clawType: props.state.clawType, clawVersion: "latest",
          apiKey: props.state.apiKey || null, useNative: method === "native",
          installBrowser: props.state.installBrowser && method !== "native",
          installMcpBridge: props.state.installMcpBridge,
          gatewayPort: 0,
          image: method === "local" ? props.state.localFilePath : null,
        });
      }
    } catch (e) { clearTimeout(timer); cleanup(); setInstalling(false); setInstallError(String(e)); props.onError(String(e)); }
  }

  // Expose retry for parent — called via ref or re-mount
  onMount(() => { startInstall(); });

  return (
    <div class="flex flex-col h-full">
      <h2 class="text-xl font-bold mb-3">Installing...</h2>

      {/* Progress bar */}
      <div class="w-full bg-gray-800 rounded-full h-2 mb-1">
        <div class={`h-2 rounded-full transition-all ${installError() ? "bg-red-600" : "bg-indigo-600"}`}
          style={{ width: `${progress()}%` }} />
      </div>
      <p class="text-xs text-gray-400 mb-3">{progressMessage() || "Preparing..."}</p>

      {/* Install stages checklist */}
      <div class="bg-gray-800 rounded border border-gray-700 p-2 mb-3 max-h-32 overflow-y-auto">
        <For each={props.stages}>
          {(s) => {
            const done = () => completedStages().has(s.key);
            const active = () => currentStage() === s.key && !done();
            const failed = () => !!(installError()) && active();
            return (
              <div class={`flex items-center gap-2 text-xs py-0.5 px-1 ${
                done() ? "text-green-400" : active() ? (failed() ? "text-red-400" : "text-indigo-300") : "text-gray-600"
              }`}>
                <span class="w-4 text-center shrink-0">
                  {done() ? "✓" : failed() ? "✗" : active() ? "▶" : "○"}
                </span>
                <span>{s.label}</span>
              </div>
            );
          }}
        </For>
      </div>

      {/* Log output — fills remaining space */}
      <div class="flex-1 min-h-0">
        <LogBox logs={installLogs()} height="h-full" />
      </div>
    </div>
  );
}
