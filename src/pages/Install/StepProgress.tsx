import { createSignal, createEffect, on, onCleanup, For, type Accessor } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { InstallState, InstallProgress } from "./types";
import { makeInstallStages } from "./types";
import LogBox from "./LogBox";

export default function StepProgress(props: {
  state: InstallState;
  stages: Array<{ key: string; label: string }>;
  /**
   * Monotonic counter that drives the install. The install runs on every
   * change of this accessor — including its first value. This replaces the
   * previous `<Show keyed>` remount hack, which silently swallowed the
   * onMount-driven listener registration under certain Tauri+Solid timing.
   */
  retryTrigger: Accessor<number>;
  onComplete: () => void;
  onError: (msg: string) => void;
}) {
  const [progress, setProgress] = createSignal(0);
  const [progressMessage, setProgressMessage] = createSignal("");
  const [currentActivity, setCurrentActivity] = createSignal("");
  const [installing, setInstalling] = createSignal(false);
  const [installError, setInstallError] = createSignal("");
  const [installLogs, setInstallLogs] = createSignal<string[]>([]);
  const [completedStages, setCompletedStages] = createSignal<Set<string>>(new Set<string>());
  const [currentStage, setCurrentStage] = createSignal("");

  // Extract a user-friendly "current activity" hint from a raw log line.
  // Returns null when the line is noise (heartbeats, generic progress) so the
  // previous activity hint stays on screen.
  function extractActivity(msg: string): string | null {
    if (!msg) return null;
    // VM heartbeat or generic poller label — don't clobber activity.
    if (/\[heartbeat\b|\bInstalling\b.*\(\d+s\)/.test(msg)) return null;
    // `npm info run <pkg>@<ver> <phase>` — e.g. postinstall hooks
    let m = msg.match(/npm\s+info\s+run\s+(\S+@\S+)\s+(\S+)/);
    if (m) return `${m[2]}: ${m[1]}`;
    // `npm http fetch GET 200 https://registry.npmjs.org/<pkg>/-/...`
    m = msg.match(/https?:\/\/[^\s]*?\/([^/\s]+?)\/-\//);
    if (m) return `fetch: ${decodeURIComponent(m[1])}`;
    // `added N packages in ...`
    m = msg.match(/added\s+(\d+)\s+packages?/);
    if (m) return `linked ${m[1]} packages`;
    // Optional dep failure signal — show the package name for transparency.
    m = msg.match(/reify failed optional dependency.*node_modules\/([^/\s]+)/);
    if (m) return `optional dep failed (ignored): ${m[1]}`;
    // apk progress — e.g. `(1/25) Installing busybox (1.37.0-r12)`
    m = msg.match(/\(\d+\/\d+\)\s+Installing\s+(\S+)/);
    if (m) return `apk: ${m[1]}`;
    return null;
  }

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFailed: UnlistenFn | null = null;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  function detachAll() {
    unlistenProgress?.(); unlistenProgress = null;
    unlistenComplete?.(); unlistenComplete = null;
    unlistenFailed?.();   unlistenFailed = null;
    clearTimeout(idleTimer);
  }
  onCleanup(detachAll);

  async function startInstall() {
    // A retry reruns this whole flow — drop any listeners/timers from the
    // previous attempt before registering fresh ones, so we never accumulate
    // duplicate handlers or leak a timer.
    detachAll();

    setInstalling(true); setInstallError(""); setProgress(0);
    setProgressMessage("Starting..."); setCurrentActivity(""); setInstallLogs([]);
    setCompletedStages(new Set<string>()); setCurrentStage("");

    // 22 min: must exceed the backend idle ceiling (20 min) so the user-facing
    // timer doesn't fire before the backend gives its own stalled error.
    const IDLE_TIMEOUT = 22 * 60 * 1000;
    let done = false;
    function resetTimer() {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        if (!done) {
          detachAll();
          setInstalling(false);
          setInstallError("Installation stalled — no progress for 22 minutes");
          props.onError("Installation stalled — no progress for 22 minutes");
        }
      }, IDLE_TIMEOUT);
    }
    function finishRun() { done = true; detachAll(); }
    resetTimer();

    unlistenProgress = await listen<InstallProgress>("install-progress", (ev) => {
      resetTimer();
      const p = ev.payload;
      setProgress(p.percent); setProgressMessage(p.message);
      const activity = extractActivity(p.message);
      if (activity) setCurrentActivity(activity);
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
      finishRun();
      setInstalling(false);
      setCompletedStages(new Set<string>(props.stages.map(s => s.key)));
      setInstallLogs(l => [...l, "✓ Installation complete!"]);
      props.onComplete();
    });

    unlistenFailed = await listen<string>("install-failed", (ev) => {
      finishRun();
      setInstalling(false);
      setInstallError(String(ev.payload));
      setInstallLogs(l => [...l, `✗ ERROR: ${ev.payload}`]);
      props.onError(String(ev.payload));
    });

    try {
      const method = props.state.installMethod;
      if (method === "native-import") {
        await invoke("install_openclaw", {
          instanceName: props.state.instanceName, clawType: props.state.clawType, clawVersion: "latest",
          useNative: true,
          installBrowser: false, installMcpBridge: false,
          gatewayPort: 0, image: props.state.localFilePath,
          proxyJson: props.state.proxyJson,
        });
      } else {
        await invoke("install_openclaw", {
          instanceName: props.state.instanceName, clawType: props.state.clawType, clawVersion: "latest",
          useNative: method === "native",
          installBrowser: props.state.installBrowser && method !== "native",
          installMcpBridge: props.state.installMcpBridge,
          gatewayPort: 0,
          image: method === "local" ? props.state.localFilePath : null,
          proxyJson: props.state.proxyJson,
        });
      }
    } catch (e) {
      finishRun();
      setInstalling(false);
      setInstallError(String(e));
      props.onError(String(e));
    }
  }

  // Drive the install off the parent's retry counter. The effect runs once on
  // mount (first value) and once per retry-click (each increment). `on(..., ,
  // { defer: false })` keeps the eager first run without needing onMount.
  createEffect(on(() => props.retryTrigger(), () => { void startInstall(); }));

  return (
    <div class="flex flex-col h-full">
      <h2 class="text-xl font-bold mb-3">Installing...</h2>

      {/* Progress bar */}
      <div class="w-full bg-gray-800 rounded-full h-2 mb-1">
        <div class={`h-2 rounded-full transition-all ${installError() ? "bg-red-600" : "bg-indigo-600"}`}
          style={{ width: `${progress()}%` }} />
      </div>
      {/* Current activity: the extracted package/phase, sticks around across
          heartbeats so the user always sees the most recent concrete work. */}
      <p class="text-xs text-indigo-300 mb-0.5 truncate" title={currentActivity()}>
        {currentActivity() || "Preparing..."}
      </p>
      <p class="text-[11px] text-gray-500 mb-3 truncate" title={progressMessage()}>
        {progressMessage() || ""}
      </p>

      {/* Install stages checklist — compact so LogBox gets the space */}
      <div class="bg-gray-800 rounded border border-gray-700 p-2 mb-3 max-h-24 overflow-y-auto">
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

      {/* Live log — fills remaining space; auto-scrolls to latest line. */}
      <div class="flex-1 min-h-0 flex flex-col">
        <div class="text-[11px] text-gray-500 mb-1 flex justify-between">
          <span>Live log ({installLogs().length} lines)</span>
          <span class="text-gray-600">scroll to follow</span>
        </div>
        <LogBox logs={installLogs()} height="h-full" />
      </div>
    </div>
  );
}
